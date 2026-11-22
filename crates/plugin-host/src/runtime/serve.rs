use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use craft_plugin::{HostCall, ImageDesc, LogLevel, Outcome, Permission, Pixel, Root};
use serde_json::{Value, json};

use super::gate;
use crate::cache::Cache;
use crate::host::{Host, Pixels};

pub(crate) struct Identity {
    pub id: String,
    pub dir: PathBuf,
    pub data_dir: PathBuf,
    pub grants: BTreeSet<Permission>,
    pub cache: Arc<Cache>,
}

pub(crate) struct HostCx<'a> {
    pub host: &'a mut dyn Host,
    pub plugin: &'a Identity,
}

pub(crate) enum Bulk {
    None,
    Bytes(Vec<u8>),
    Shared(Arc<[u8]>),
    Pixels(Vec<Pixel>),
}

impl Bulk {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Bulk::None => &[],
            Bulk::Bytes(bytes) => bytes,
            Bulk::Shared(bytes) => bytes,
            Bulk::Pixels(pixels) => bytemuck::cast_slice(pixels),
        }
    }
}

pub(crate) fn serve(cx: &mut HostCx, request: &[u8], bulk: &[u8]) -> (Vec<u8>, Bulk) {
    let answered = serde_json::from_slice::<HostCall>(request).map_err(|e| format!("malformed host call: {e}")).and_then(|call| answer(cx, call, bulk));
    let (outcome, bulk) = match answered {
        Ok((value, bulk)) => (Outcome::Ok(value), bulk),
        Err(message) => (Outcome::Err(message), Bulk::None),
    };
    (serde_json::to_vec(&outcome).unwrap_or_default(), bulk)
}

fn answer(cx: &mut HostCx, call: HostCall, bulk: &[u8]) -> Result<(Value, Bulk), String> {
    gate::check(&call, cx.host, &cx.plugin.grants)?;
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
        HostCall::ReadFile { root, path } => {
            let bytes = std::fs::read(resolve(cx.plugin, root, &path)?).map_err(|e| format!("{path}: {e}"))?;
            Ok((Value::Null, Bulk::Bytes(bytes)))
        }
        HostCall::WriteFile { root, path } => {
            let target = resolve(cx.plugin, root, &path)?;
            std::fs::write(target, bulk).map_err(|e| format!("{path}: {e}"))?;
            plain(Value::Null)
        }
        HostCall::CacheGet { key } => Ok(match cx.plugin.cache.get(&key) {
            Some(bytes) => (Value::Bool(true), Bulk::Shared(bytes)),
            None => (Value::Bool(false), Bulk::None),
        }),
        HostCall::CachePut { key } => cx.plugin.cache.put(key, bulk).map(|()| (Value::Null, Bulk::None)),
        HostCall::CacheRemove { key } => {
            cx.plugin.cache.remove(&key);
            plain(Value::Null)
        }
        HostCall::ListFiles { root, path } => {
            let dir = resolve(cx.plugin, root, &path)?;
            let mut names: Vec<String> =
                std::fs::read_dir(dir).map_err(|e| format!("{path}: {e}"))?.filter_map(|entry| entry.ok()?.file_name().into_string().ok()).collect();
            names.sort();
            plain(json!(names))
        }
    }
}

fn image(pixels: Option<Pixels>) -> Result<(Value, Bulk), String> {
    let pixels = pixels.ok_or("no image available")?;
    Ok((json!(ImageDesc { width: pixels.width, height: pixels.height }), Bulk::Pixels(pixels.data)))
}

fn resolve(plugin: &Identity, root: Root, path: &str) -> Result<PathBuf, String> {
    let base = match root {
        Root::User => {
            let path = Path::new(path);
            return path.is_absolute().then(|| path.to_path_buf()).ok_or("a user file needs an absolute path".into());
        }
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
