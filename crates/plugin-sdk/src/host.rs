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

pub fn list_files(root: Root, path: &str) -> Result<Vec<String>> {
    call(&HostCall::ListFiles { root, path: path.into() }, &[]).map(|(names, _)| names)
}
