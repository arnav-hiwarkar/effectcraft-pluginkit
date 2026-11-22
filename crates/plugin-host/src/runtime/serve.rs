use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_plugin::{AudioDesc, HostCall, ImageDesc, LogLevel, Outcome, Permission, Pixel, Root};
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
    held: Option<Handle>,
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

    fn remember_pick(&self, path: PathBuf, writable: bool) {
        let held = Handle::from_path(&path).ok();
        if let Ok(mut picked) = self.picked.lock() {
            picked.retain(|chosen| chosen.path != path);
            picked.push(Picked { path, writable, held });
        }
    }

    fn open_picked(&self, path: &str, write: bool) -> Result<File, String> {
        let mut picked = self.picked.lock().map_err(|_| "the picked files are unavailable")?;
        let chosen = picked.iter_mut().find(|chosen| chosen.matches(Path::new(path), write)).ok_or(format!("`{path}` was not picked"))?;
        let opened = match (write, chosen.held.is_some()) {
            (true, false) => OpenOptions::new().write(true).create_new(true).open(path),
            (true, true) => OpenOptions::new().write(true).open(path),
            (false, true) => File::open(path),
            (false, false) => return Err(format!("{path}: the picked file did not exist when it was picked")),
        };
        let file = opened.map_err(|e| format!("{path}: {e}"))?;
        let found = file.try_clone().and_then(Handle::from_file).map_err(|e| format!("{path}: {e}"))?;
        match &chosen.held {
            Some(held) if *held != found => Err(format!("`{path}` is no longer the file the user picked; ask the user to pick it again")),
            Some(_) => Ok(file),
            None => {
                chosen.held = Some(found);
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
    pub deadline: Option<Instant>,
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
        HostCall::Command { .. } | HostCall::Notify { .. } | HostCall::Frame { .. } | HostCall::Audio { .. } | HostCall::ClipboardRead | HostCall::ClipboardWrite { .. }
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
        call @ (HostCall::ReadFile { .. } | HostCall::WriteFile { .. } | HostCall::ListFiles { .. }) => on_disk(cx, call, bulk),
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
                cx.plugin.remember_pick(path.clone(), request.save);
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

fn on_disk(cx: &HostCx, call: HostCall, bulk: &[u8]) -> Result<(Value, Bulk), String> {
    let Some(deadline) = cx.deadline else { return disk(cx.plugin, call, bulk) };
    let (engine, bulk) = (cx.engine.clone(), bulk.to_vec());
    let (send, receive) = channel();
    std::thread::Builder::new()
        .name("craft-plugin-disk".into())
        .spawn(move || {
            let _ = send.send(disk(&engine.identity, call, &bulk));
        })
        .map_err(|e| e.to_string())?;
    receive.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| Err("the disk did not answer within the time limit".into()))
}

fn disk(plugin: &Identity, call: HostCall, bulk: &[u8]) -> Result<(Value, Bulk), String> {
    let plain = |value: Value| Ok((value, Bulk::None));
    match call {
        HostCall::ReadFile { root: Root::User, path } if !plugin.grants.contains(&Permission::FilesRead) => {
            let mut bytes = Vec::new();
            plugin.open_picked(&path, false)?.read_to_end(&mut bytes).map_err(|e| format!("{path}: {e}"))?;
            Ok((Value::Null, Bulk::Bytes(bytes)))
        }
        HostCall::WriteFile { root: Root::User, path } if !plugin.grants.contains(&Permission::FilesWrite) => {
            let mut file = plugin.open_picked(&path, true)?;
            file.set_len(0).and_then(|()| file.write_all(bulk)).map_err(|e| format!("{path}: {e}"))?;
            plain(Value::Null)
        }
        HostCall::ReadFile { root, path } => {
            let bytes = std::fs::read(resolve(plugin, root, &path)?).map_err(|e| format!("{path}: {e}"))?;
            Ok((Value::Null, Bulk::Bytes(bytes)))
        }
        HostCall::WriteFile { root, path } => {
            let target = resolve(plugin, root, &path)?;
            std::fs::write(target, bulk).map_err(|e| format!("{path}: {e}"))?;
            plain(Value::Null)
        }
        HostCall::ListFiles { root, path } => {
            let dir = resolve(plugin, root, &path)?;
            let mut names: Vec<String> =
                std::fs::read_dir(dir).map_err(|e| format!("{path}: {e}"))?.filter_map(|entry| entry.ok()?.file_name().into_string().ok()).collect();
            names.sort();
            plain(json!(names))
        }
        _ => Err("not a file call".into()),
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
    let anchor = anchor.canonicalize().map_err(|e| format!("{path}: {e}"))?;
    anchor.starts_with(&base).then_some(joined).ok_or(format!("`{path}` leaves the plugin's folder"))
}
