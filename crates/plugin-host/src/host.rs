use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use craft_plugin::{AudioRequest, FrameRequest, LayerPlacement, MaskPath, ParamsAt, Permission, PickFile, Pixel, Scene};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Interactive,
    Render,
    Task,
}

pub struct Samples {
    pub channels: u32,
    pub rate: u32,
    pub samples: Vec<f32>,
}

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
    fn access(&self, _command: &str, _params: &Value) -> Access {
        Access::Refused("commands are not available here".into())
    }

    fn command(&mut self, _id: &str, _params: Value) -> Result<Value, String> {
        Err("commands are not available here".into())
    }

    fn notify(&mut self, _message: &str) {}

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

    fn scene_at(&mut self, _comp_time: f64) -> Option<Scene> {
        None
    }

    fn placement(&mut self, _param: &str, _comp_time: Option<f64>) -> Option<LayerPlacement> {
        None
    }

    fn frame(&mut self, _request: &FrameRequest) -> Result<Pixels, String> {
        Err("frames are not available here".into())
    }

    fn audio(&mut self, _request: &AudioRequest) -> Result<Samples, String> {
        Err("audio is not available here".into())
    }

    fn pick_file(&mut self, _request: &PickFile) -> Result<Option<PathBuf>, String> {
        Err("a file dialog is not available here".into())
    }

    fn params_at(&mut self, _layer_time: f64) -> Option<ParamsAt> {
        None
    }

    fn project_closed(&self) -> Option<Arc<AtomicBool>> {
        None
    }

    fn finish(&mut self, _written: &mut [Pixel]) {}

    fn clipboard(&mut self) -> Result<String, String> {
        Err("the clipboard is not available here".into())
    }

    fn set_clipboard(&mut self, _text: &str) -> Result<(), String> {
        Err("the clipboard is not available here".into())
    }
}
