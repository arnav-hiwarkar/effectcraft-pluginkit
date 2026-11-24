use craft_plugin::prelude::*;

use crate::stabilise::NAME;
use crate::track::{KEY, Request, Track};

#[derive(Default)]
pub struct TrackerPanel {
    selected: Vec<u64>,
    running: Option<u64>,
    status: String,
}

fn param(layer: u64, id: &str) -> Result<serde_json::Value> {
    let found = host::command("prop.get", json!({"layer": layer, "path": format!("effects/{NAME}/{id}")}))?;
    Ok(found["value"].clone())
}

fn request(layer: u64) -> Result<Request> {
    let info = host::command("comp.info", json!({}))?;
    let fps = info["frameRate"].as_f64().ok_or("the composition has no frame rate")?;
    let [from, to] = [0, 1].map(|i| info["workArea"][i].as_f64().unwrap_or(0.0));
    let [min, max] = [param(layer, "min")?, param(layer, "max")?].map(serde_json::from_value::<[f64; 2]>);
    let search = param(layer, "search")?.as_f64().ok_or("the search range is not a number")?;
    let (min, max) = (min?, max?);
    Ok(Request { layer, min, max, search, from, to, fps })
}

pub fn apply(source: u64, target: u64) -> Result<usize> {
    let bytes = host::data(Place::Layer(source), KEY)?.ok_or("the first selected layer has no track: track it first")?;
    let track: Track = serde_json::from_slice(&bytes)?;
    let steps: Vec<_> = (track.points.iter().enumerate())
        .map(|(frame, at)| {
            let time = track.start + frame as f64 / track.fps;
            json!({"command": "prop.addKey", "params": {"layer": target, "path": "transform/position", "time": time, "timeBase": "comp", "value": at}})
        })
        .collect();
    host::command("engine.batch", json!({"label": "Tracker: Apply Track", "steps": steps}))?;
    Ok(track.points.len())
}

impl TrackerPanel {
    fn track(&mut self) -> Result<String> {
        let layer = *self.selected.first().ok_or("select the layer to track")?;
        self.running = Some(host::start("track", request(layer)?)?);
        Ok("Tracking".into())
    }

    fn apply(&self) -> Result<String> {
        let [source, target] = self.selected[..] else { return Err("select the tracked layer, then the layer to move".into()) };
        Ok(format!("Wrote {} position keyframes. One undo removes them.", apply(source, target)?))
    }

    fn poll(&mut self) {
        let Some(state) = self.running.and_then(|id| host::task(id).ok()) else { return };
        self.status = match state.status {
            TaskStatus::Running => format!("{:.0}% {}", state.progress * 100.0, state.message),
            TaskStatus::Done => "Tracked. The layer is stabilised.".into(),
            TaskStatus::Failed => state.error.unwrap_or_default(),
            TaskStatus::Cancelled => "Cancelled".into(),
        };
        if state.status != TaskStatus::Running {
            self.running = None;
        }
    }
}

impl Panel for TrackerPanel {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(AppEvent::Selection { layers, .. }) = ui.on(Topic::Selection) {
            self.selected = layers.clone();
        }
        self.poll();
        if self.running.is_some() {
            ui.every("progress", 100);
        }
        ui.heading("Tracker");
        ui.row(|ui| {
            if ui.button_enabled("Track", self.running.is_none()) {
                self.status = self.track().unwrap_or_else(|e| e.0);
            }
            if ui.button_enabled("Cancel", self.running.is_some()) {
                let _ = self.running.map(host::cancel);
            }
            if ui.button_enabled("Apply to Layer", self.running.is_none()) {
                self.status = self.apply().unwrap_or_else(|e| e.0);
            }
        });
        ui.weak(&self.status);
    }
}
