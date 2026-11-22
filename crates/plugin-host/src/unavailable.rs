use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use craft_plugin::{Permission, TaskState};

use crate::host::{Host, Origin};
use crate::plugin::{Io, PluginError};
use crate::registry::Bundle;

pub(crate) enum Engine {}

impl Engine {
    pub fn open(_: &Bundle, _: BTreeSet<Permission>, _: PathBuf) -> Result<Arc<Engine>, PluginError> {
        Err(PluginError::Load("plugins run in the desktop app only".into()))
    }

    pub fn cached(&self, _: &str) -> Option<Arc<[u8]>> {
        match *self {}
    }

    pub fn image(&self, _: &str) -> Option<Arc<crate::images::PluginImage>> {
        match *self {}
    }

    pub fn uncache(&self, _: &str) {
        match *self {}
    }

    pub fn cache_generation(&self) -> u64 {
        match *self {}
    }

    pub fn copies_pixels(&self) -> bool {
        match *self {}
    }

    pub fn call(self: &Arc<Self>, _: &[u8], _: Io, _: &mut dyn Host, _: Origin, _: Duration) -> Result<Vec<u8>, PluginError> {
        match **self {}
    }

    pub fn accept_drop(&self, _: &std::path::Path) -> Result<(), String> {
        match *self {}
    }

    pub fn know_tasks(&self, _: Vec<String>) {
        match *self {}
    }

    pub fn start(self: &Arc<Self>, _: &str, _: serde_json::Value, _: Option<Arc<std::sync::atomic::AtomicBool>>) -> Result<u64, String> {
        match **self {}
    }

    pub fn tasks(&self) -> Vec<TaskState> {
        match *self {}
    }

    pub fn running_tasks(&self) -> usize {
        match *self {}
    }

    pub fn cancel(&self, _: u64) -> bool {
        match *self {}
    }

    pub fn cancel_all(&self) {
        match *self {}
    }

    pub fn pump(self: &Arc<Self>, _: &mut dyn Host) -> usize {
        match **self {}
    }
}
