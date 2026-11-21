use craft_plugin::{Root, host};
use serde_json::Value;

pub trait Host {
    fn command(&self, id: &str, params: Value) -> Result<Value, String>;
    fn read_data(&self, path: &str) -> Result<Vec<u8>, String>;
    fn write_data(&self, path: &str, bytes: &[u8]) -> Result<(), String>;
}

pub struct LiveHost;

impl Host for LiveHost {
    fn command(&self, id: &str, params: Value) -> Result<Value, String> {
        host::command(id, params).map_err(|e| e.0)
    }

    fn read_data(&self, path: &str) -> Result<Vec<u8>, String> {
        host::read_file(Root::Data, path).map_err(|e| e.0)
    }

    fn write_data(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        host::write_file(Root::Data, path, bytes).map_err(|e| e.0)
    }
}
