use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_plugin::{AudioDesc, HostCall, ImageDesc, LogLevel, Outcome, Permission, Pixel, Root, file_failure};
use same_file::Handle;
use serde_json::{Value, json};

use super::tasks::{Slot, Tasks};
use super::{Engine, gate};
use crate::cache::Cache;
use crate::host::{Host, Origin, Pixels};
use crate::images::Images;

pub(crate) struct Identity {
    pub id: String,
    pub dir: PathBuf,
    pub data_dir: PathBuf,
    pub app_folders: Vec<PathBuf>,
    pub grants: BTreeSet<Permission>,
    pub cache: Arc<Cache>,
    pub tasks: Tasks,
    pub picked: Mutex<Vec<Picked>>,
    pub images: Images,
}

pub(crate) struct Picked {
    path: PathBuf,
    writable: bool,
    held: Option<Arc<Handle>>,
    folder: Option<Arc<Folder>>,
}

#[cfg(unix)]
type Folder = std::os::fd::OwnedFd;
#[cfg(not(unix))]
type Folder = Handle;

#[cfg(unix)]
fn pin_folder(dir: &Path) -> std::io::Result<Folder> {
    use rustix::fs::{Mode, OFlags};
    Ok(rustix::fs::open(dir, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?)
}

#[cfg(not(unix))]
fn pin_folder(dir: &Path) -> std::io::Result<Folder> {
    Handle::from_path(dir)
}

#[cfg(unix)]
fn create_in(folder: &Folder, path: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    Ok(File::from(rustix::fs::openat(folder, name, flags, Mode::from_raw_mode(0o666))?))
}

#[cfg(not(unix))]
fn create_in(folder: &Folder, path: &Path) -> std::io::Result<File> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    if Handle::from_path(parent)? != *folder {
        return Err(std::io::Error::other("the folder it was picked in has moved"));
    }
    OpenOptions::new().write(true).create_new(true).open(path)
}

impl Picked {
    fn matches(&self, path: &Path, write: bool) -> bool {
        self.path == path && (self.writable || !write)
    }
}

impl Identity {
    pub fn picked(&self, path: &str, write: bool) -> bool {
        self.picked.lock().is_ok_and(|picked| picked.iter().any(|chosen| chosen.matches(Path::new(path), write)))
    }

    pub fn remember_pick(&self, path: PathBuf, writable: bool) -> Result<(), String> {
        user_path(self, &path)?;
        let held = Handle::from_path(&path).ok().map(Arc::new);
        let folder = match (writable, &held, path.parent()) {
            (true, None, Some(parent)) => Some(Arc::new(pin_folder(parent).map_err(|e| format!("{}: {e}", parent.display()))?)),
            _ => None,
        };
        let mut picked = self.picked.lock().map_err(|_| "the picked files are unavailable")?;
        picked.retain(|chosen| chosen.path != path);
        picked.push(Picked { path, writable, held, folder });
        Ok(())
    }

    fn open_picked(&self, path: &str, write: bool) -> Result<File, String> {
        let (held, folder) = {
            let picked = self.picked.lock().map_err(|_| "the picked files are unavailable")?;
            let chosen = picked.iter().find(|chosen| chosen.matches(Path::new(path), write)).ok_or(format!("`{path}` was not picked"))?;
            (chosen.held.clone(), chosen.folder.clone())
        };
        let opened = match (write, &held, folder) {
            (true, Some(_), _) => OpenOptions::new().write(true).open(path),
            (false, Some(_), _) => File::open(path),
            (true, None, Some(folder)) => create_in(&folder, Path::new(path)),
            _ => return Err(format!("{path}: the picked file did not exist when it was picked")),
        };
        let file = opened.map_err(|e| file_failure(path, &e))?;
        let found = file.try_clone().and_then(Handle::from_file).map_err(|e| format!("{path}: {e}"))?;
        match held {
            Some(held) if *held != found => Err(format!("`{path}` is no longer the file the user picked; ask the user to pick it again")),
            Some(_) => Ok(file),
            None => {
                let mut picked = self.picked.lock().map_err(|_| "the picked files are unavailable")?;
                if let Some(chosen) = picked.iter_mut().find(|chosen| chosen.matches(Path::new(path), write)) {
                    chosen.held = Some(Arc::new(found));
                }
                Ok(file)
            }
        }
    }
}

pub(crate) struct HostCx<'a> {
    pub host: &'a mut dyn Host,
    pub plugin: &'a Identity,
    pub engine: &'a Arc<Engine>,
    pub origin: Origin,
    pub task: Option<Arc<Slot>>,
    pub on_task_thread: bool,
    pub paused: Duration,
}

pub(crate) enum Bulk {
    None,
    Bytes(Vec<u8>),
    Shared(Arc<[u8]>),
    Pixels(Vec<Pixel>),
    Floats(Vec<f32>),
}

impl Bulk {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Bulk::None => &[],
            Bulk::Bytes(bytes) => bytes,
            Bulk::Shared(bytes) => bytes,
            Bulk::Pixels(pixels) => bytemuck::cast_slice(pixels),
            Bulk::Floats(samples) => bytemuck::cast_slice(samples),
        }
    }
}

fn needs_the_app(call: &HostCall) -> bool {
    matches!(
        call,
        HostCall::Command { .. }
            | HostCall::Notify { .. }
            | HostCall::Frame { .. }
            | HostCall::Audio { .. }
            | HostCall::ClipboardRead
            | HostCall::ClipboardWrite { .. }
    )
}

pub(crate) fn serve(cx: &mut HostCx, request: &[u8], bulk: &[u8]) -> (Vec<u8>, Bulk) {
    let call = match serde_json::from_slice::<HostCall>(request) {
        Ok(call) => call,
        Err(e) => return reply(Err(format!("malformed host call: {e}"))),
    };
    if cx.on_task_thread
        && needs_the_app(&call)
        && let Some(task) = cx.task.clone()
    {
        return cx.plugin.tasks.forward(&task, request, bulk).unwrap_or_else(|e| reply(Err(e)));
    }
    reply(answer(cx, call, bulk))
}

fn reply(answered: Result<(Value, Bulk), String>) -> (Vec<u8>, Bulk) {
    let (outcome, bulk) = match answered {
        Ok((value, bulk)) => (Outcome::Ok(value), bulk),
        Err(message) => (Outcome::Err(message), Bulk::None),
    };
    (serde_json::to_vec(&outcome).unwrap_or_default(), bulk)
}

fn answer(cx: &mut HostCx, call: HostCall, bulk: &[u8]) -> Result<(Value, Bulk), String> {
    gate::check(&call, cx.host, cx.plugin, cx.origin)?;
    let plain = |value: Value| Ok((value, Bulk::None));
    match call {
        HostCall::Log { level, message } => {
            let level = match level {
                LogLevel::Info => log::Level::Info,
                LogLevel::Warn => log::Level::Warn,
                LogLevel::Error => log::Level::Error,
            };
            log::log!(target: "plugin", level, "[{}] {message}", cx.plugin.id);
            plain(Value::Null)
        }
        HostCall::Notify { message } => {
            cx.host.notify(&message);
            plain(Value::Null)
        }
        HostCall::Command { id, params } => cx.host.command(&id, params).map(|value| (value, Bulk::None)),
        HostCall::Layer { param, time } => image(cx.host.layer(&param, time)),
        HostCall::Source { time } => image(cx.host.source(time)),
        HostCall::Masks => plain(json!(cx.host.masks())),
        HostCall::Scene => cx.host.scene().map(|scene| (json!(scene), Bulk::None)).ok_or("no scene here".into()),
        HostCall::ReadFile { root: Root::User, path } if !cx.plugin.grants.contains(&Permission::FilesRead) => {
            let mut bytes = Vec::new();
            cx.plugin.open_picked(&path, false)?.read_to_end(&mut bytes).map_err(|e| file_failure(&path, &e))?;
            Ok((Value::Null, Bulk::Bytes(bytes)))
        }
        HostCall::WriteFile { root: Root::User, path } if !cx.plugin.grants.contains(&Permission::FilesWrite) => {
            let mut file = cx.plugin.open_picked(&path, true)?;
            file.set_len(0).and_then(|()| file.write_all(bulk)).map_err(|e| format!("{path}: {e}"))?;
            plain(Value::Null)
        }
        HostCall::ReadFile { root, path } => {
            let bytes = std::fs::read(resolve(cx.plugin, root, &path)?).map_err(|e| file_failure(&path, &e))?;
            Ok((Value::Null, Bulk::Bytes(bytes)))
        }
        HostCall::WriteFile { root, path } => {
            std::fs::write(resolve(cx.plugin, root, &path)?, bulk).map_err(|e| format!("{path}: {e}"))?;
            plain(Value::Null)
        }
        HostCall::ListFiles { root, path } => {
            let dir = resolve(cx.plugin, root, &path)?;
            let mut names: Vec<String> =
                std::fs::read_dir(dir).map_err(|e| format!("{path}: {e}"))?.filter_map(|entry| entry.ok()?.file_name().into_string().ok()).collect();
            names.sort();
            plain(json!(names))
        }
        HostCall::CacheGet { key } => Ok(match cx.plugin.cache.get(&key) {
            Some(bytes) => (Value::Bool(true), Bulk::Shared(bytes)),
            None => (Value::Bool(false), Bulk::None),
        }),
        HostCall::CachePut { key } => {
            cx.plugin.cache.put(key, bulk)?;
            if cx.origin != Origin::Render {
                cx.plugin.cache.changed_outside_a_render();
            }
            plain(Value::Null)
        }
        HostCall::CacheHas { key } => plain(Value::Bool(cx.plugin.cache.get(&key).is_some())),
        HostCall::CacheRemove { key } => {
            cx.plugin.cache.remove(&key);
            if cx.origin != Origin::Render {
                cx.plugin.cache.changed_outside_a_render();
            }
            plain(Value::Null)
        }
        HostCall::Start { task, args } => {
            let project = cx.task.as_ref().map_or_else(|| cx.host.project_closed(), |task| task.project());
            cx.engine.start(&task, args, project).map(|id| (json!(id), Bulk::None))
        }
        HostCall::Task { id } => cx.plugin.tasks.get(id).map(|slot| (json!(slot.state()), Bulk::None)).ok_or(format!("no task {id}")),
        HostCall::Cancel { id } => match cx.plugin.tasks.cancel(id) {
            true => plain(Value::Null),
            false => Err(format!("no task {id}")),
        },
        HostCall::Progress { fraction, message } => {
            let task = cx.task.as_ref().ok_or("only a task reports progress")?;
            task.progress(fraction, message).map(|()| (Value::Null, Bulk::None))
        }
        HostCall::Frame { request } => image(Some(cx.host.frame(&request)?)),
        HostCall::Audio { request } => {
            let audio = cx.host.audio(&request)?;
            Ok((json!(AudioDesc { channels: audio.channels, rate: audio.rate }), Bulk::Floats(audio.samples)))
        }
        HostCall::PickFile { request } => {
            let started = Instant::now();
            let chosen = cx.host.pick_file(&request);
            cx.paused += started.elapsed();
            let chosen = chosen?;
            if let Some(path) = &chosen {
                cx.plugin.remember_pick(path.clone(), request.save)?;
            }
            plain(json!(chosen.map(|path| path.to_string_lossy().into_owned())))
        }
        HostCall::UploadImage { key, width, height } => cx.plugin.images.put(key, width, height, bulk).map(|()| (Value::Null, Bulk::None)),
        HostCall::RemoveImage { key } => plain(json!(cx.plugin.images.remove(&key))),
        HostCall::ParamsAt { time } => cx.host.params_at(time).map(|answer| (json!(answer), Bulk::None)).ok_or("no parameters at that time".into()),
        HostCall::ClipboardRead => cx.host.clipboard().map(|text| (json!(text), Bulk::None)),
        HostCall::ClipboardWrite { text } => cx.host.set_clipboard(&text).map(|()| (Value::Null, Bulk::None)),
    }
}

fn image(pixels: Option<Pixels>) -> Result<(Value, Bulk), String> {
    let pixels = pixels.ok_or("no image available")?;
    Ok((json!(ImageDesc { width: pixels.width, height: pixels.height }), Bulk::Pixels(pixels.data)))
}

fn user_path(plugin: &Identity, path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("a user file needs an absolute path".into());
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(format!("`{}` must not contain `..`", path.display()));
    }
    let existing = path.ancestors().find(|ancestor| ancestor.symlink_metadata().is_ok()).ok_or("no part of that path exists")?;
    let real = existing.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let owned = plugin.app_folders.iter().any(|folder| real.starts_with(folder.canonicalize().as_deref().unwrap_or(folder)));
    if owned {
        return Err("that folder belongs to the app".into());
    }
    Ok(path.to_path_buf())
}

fn resolve(plugin: &Identity, root: Root, path: &str) -> Result<PathBuf, String> {
    let base = match root {
        Root::User => return user_path(plugin, Path::new(path)),
        Root::Bundle => &plugin.dir,
        Root::Data => {
            std::fs::create_dir_all(&plugin.data_dir).map_err(|e| e.to_string())?;
            &plugin.data_dir
        }
    };
    let relative = Path::new(path);
    if !relative.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("`{path}` must be a plain relative path"));
    }
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    let joined = base.join(relative);
    if joined.symlink_metadata().is_ok_and(|found| found.file_type().is_symlink()) {
        return Err(format!("`{path}` is a link"));
    }
    let anchor = if joined.exists() { joined.clone() } else { joined.parent().unwrap_or(&base).to_path_buf() };
    let anchor = anchor.canonicalize().map_err(|e| file_failure(path, &e))?;
    anchor.starts_with(&base).then_some(joined).ok_or(format!("`{path}` leaves the plugin's folder"))
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::Identity;
    use crate::cache::{BUDGET, Cache};
    use crate::images::{IMAGE_BUDGET, Images};

    fn identity(dir: &std::path::Path) -> Identity {
        Identity {
            id: "test.serve".into(),
            dir: dir.to_path_buf(),
            data_dir: dir.join("data"),
            app_folders: vec![dir.join("app")],
            grants: Default::default(),
            cache: Arc::new(Cache::new(BUDGET)),
            tasks: Default::default(),
            picked: Mutex::new(Vec::new()),
            images: Images::new(IMAGE_BUDGET),
        }
    }

    #[test]
    fn a_stalled_open_of_a_picked_file_does_not_hold_up_other_permission_checks() {
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("picked.txt");
        std::fs::write(&picked, "chosen").unwrap();
        let identity = identity(dir.path());
        identity.remember_pick(picked.clone(), false).unwrap();
        std::fs::remove_file(&picked).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(&picked).status().unwrap().success());
        let path = picked.to_str().unwrap();
        std::thread::scope(|scope| {
            let opener = scope.spawn(|| identity.open_picked(path, false).map(drop));
            std::thread::sleep(Duration::from_millis(100));
            let (send, answered) = channel();
            let identity = &identity;
            scope.spawn(move || send.send(identity.picked(path, false)));
            let answer = answered.recv_timeout(Duration::from_secs(5));
            drop(std::fs::OpenOptions::new().write(true).open(&picked));
            assert_eq!(answer, Ok(true));
            assert_eq!(opener.join().unwrap(), Err(format!("`{path}` is no longer the file the user picked; ask the user to pick it again")));
        });
    }
}
