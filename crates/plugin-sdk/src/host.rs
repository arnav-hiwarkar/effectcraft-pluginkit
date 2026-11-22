use std::fmt::Display;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::abi::{Buffer, call_host};
use crate::effect::{Image, image_of};
use crate::wire::{AudioDesc, AudioRequest, FrameRequest, HostCall, LogLevel, Outcome, PickFile, Root, TaskState};
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

pub fn start(task: &str, args: impl Serialize) -> Result<u64> {
    let args = serde_json::to_value(args)?;
    call(&HostCall::Start { task: task.into(), args }, &[]).map(|(id, _)| id)
}

pub fn task(id: u64) -> Result<TaskState> {
    call(&HostCall::Task { id }, &[]).map(|(state, _)| state)
}

pub fn cancel(id: u64) -> Result<()> {
    call::<serde_json::Value>(&HostCall::Cancel { id }, &[]).map(|_| ())
}

pub fn frame(request: FrameRequest) -> Result<Image> {
    let (desc, buffer) = call::<crate::wire::ImageDesc>(&HostCall::Frame { request }, &[])?;
    image_of(desc, buffer).ok_or_else(|| Error("the app returned an empty frame".into()))
}

pub struct Audio {
    pub channels: u32,
    pub rate: u32,
    pub samples: Vec<f32>,
}

pub fn audio(request: AudioRequest) -> Result<Audio> {
    let (desc, bulk) = call::<AudioDesc>(&HostCall::Audio { request }, &[])?;
    let samples = bytemuck_floats(bulk.bytes());
    Ok(Audio { channels: desc.channels, rate: desc.rate, samples })
}

fn bytemuck_floats(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect()
}

pub fn pick_file(request: PickFile) -> Result<Option<String>> {
    call(&HostCall::PickFile { request }, &[]).map(|(path, _)| path)
}
