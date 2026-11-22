use std::fmt::Display;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::abi::{Buffer, call_host};
use crate::wire::{HostCall, LogLevel, Outcome, Root};
use crate::{Error, Result};

pub fn call<T: DeserializeOwned>(call: &HostCall, bulk: &[u8]) -> Result<(T, Buffer)> {
    let request = serde_json::to_vec(call)?;
    let (reply, bulk) = call_host(&request, bulk);
    let outcome: Outcome<T> = serde_json::from_slice(reply.bytes())?;
    std::result::Result::<T, String>::from(outcome).map(|value| (value, bulk)).map_err(Error)
}

pub fn log(level: LogLevel, message: impl Display) {
    let _ = call::<serde_json::Value>(&HostCall::Log { level, message: message.to_string() }, &[]);
}

pub fn info(message: impl Display) {
    log(LogLevel::Info, message)
}

pub fn notify(message: impl Display) {
    let _ = call::<serde_json::Value>(&HostCall::Notify { message: message.to_string() }, &[]);
}

pub fn command(id: &str, params: impl Serialize) -> Result<serde_json::Value> {
    let params = serde_json::to_value(params)?;
    call(&HostCall::Command { id: id.into(), params }, &[]).map(|(value, _)| value)
}

pub fn read_file(root: Root, path: &str) -> Result<Vec<u8>> {
    call::<serde_json::Value>(&HostCall::ReadFile { root, path: path.into() }, &[]).map(|(_, bulk)| bulk.bytes().to_vec())
}

pub fn write_file(root: Root, path: &str, bytes: &[u8]) -> Result<()> {
    call::<serde_json::Value>(&HostCall::WriteFile { root, path: path.into() }, bytes).map(|_| ())
}

pub fn cache_get(key: &str) -> Option<Vec<u8>> {
    let (found, bulk) = call::<bool>(&HostCall::CacheGet { key: key.into() }, &[]).ok()?;
    found.then(|| bulk.bytes().to_vec())
}

pub fn cache_put(key: &str, bytes: &[u8]) -> Result<()> {
    call::<serde_json::Value>(&HostCall::CachePut { key: key.into() }, bytes).map(|_| ())
}

pub fn cache_remove(key: &str) {
    let _ = call::<serde_json::Value>(&HostCall::CacheRemove { key: key.into() }, &[]);
}

pub fn list_files(root: Root, path: &str) -> Result<Vec<String>> {
    call(&HostCall::ListFiles { root, path: path.into() }, &[]).map(|(names, _)| names)
}

pub const MAX_IMAGE_SIDE: u32 = 8192;

pub fn upload_image(key: &str, width: u32, height: u32, rgba8: &[u8]) -> Result<()> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(Error(format!("an image is 1 to {MAX_IMAGE_SIDE} pixels on a side, not {width}x{height}")));
    }
    if rgba8.len() != width as usize * height as usize * 4 {
        return Err(Error(format!("a {width}x{height} image has {} bytes, not {}", width as usize * height as usize * 4, rgba8.len())));
    }
    call::<serde_json::Value>(&HostCall::UploadImage { key: key.into(), width, height }, rgba8).map(|_| ())
}
