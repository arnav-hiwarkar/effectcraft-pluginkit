use craft_plugin::{MaskPath, Permission, Pixel, Scene};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Access {
    Free,
    Needs(Vec<Permission>),
    Refused(String),
}

pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<Pixel>,
}

pub trait Host {
    fn access(&mut self, _command: &str, _params: &Value) -> Access {
        Access::Refused("commands are not available here".into())
    }

    fn command(&mut self, _id: &str, _params: Value) -> Result<Value, String> {
        Err("commands are not available here".into())
    }

    fn layer(&mut self, _param: &str, _comp_time: Option<f64>) -> Option<Pixels> {
        None
    }

    fn source(&mut self, _layer_time: f64) -> Option<Pixels> {
        None
    }

    fn masks(&mut self) -> Vec<MaskPath> {
        Vec::new()
    }

    fn scene(&mut self) -> Option<Scene> {
        None
    }
}
