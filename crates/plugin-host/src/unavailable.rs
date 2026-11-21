use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use craft_plugin::Permission;

use crate::host::Host;
use crate::plugin::{Io, PluginError};
use crate::registry::Bundle;

pub(crate) enum Engine {}

impl Engine {
    pub fn open(_: &Bundle, _: BTreeSet<Permission>, _: PathBuf) -> Result<Engine, PluginError> {
        Err(PluginError::Load("plugins run in the desktop app only".into()))
    }

    pub fn copies_pixels(&self) -> bool {
        match *self {}
    }

    pub fn call(&self, _: &[u8], _: Io, _: &mut dyn Host, _: bool, _: Duration) -> Result<Vec<u8>, PluginError> {
        match *self {}
    }
}
