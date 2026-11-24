use craft_plugin::prelude::*;

use crate::stabilise::NAME;
use crate::track::{KEY, Request, Track, frame_grid};

#[derive(Default)]
pub struct TrackerPanel {
    selected: Vec<u64>,
    running: Option<u64>,
    status: String,
}

fn param(comp: u64, layer: u64, id: &str, time: f64) -> Result<serde_json::Value> {
    let found = host::command("prop.get", json!({"comp": comp, "layer": layer, "path": format!("effects/{NAME}/{id}"), "time": time}))?;
    Ok(found["value"].clone())
}

fn request(layer: u64) -> Result<Request> {
    let info = host::command("comp.info", json!({}))?;
    let comp = info["id"].as_u64().ok_or("there is no active composition")?;
    let fps = info["frameRate"].as_f64().ok_or("the composition has no frame rate")?;
    let [from, to] = [0, 1].map(|i| info["workArea"][i].as_f64().unwrap_or(0.0));
    let (first, frames) = frame_grid(from, to, fps);
    let time = first as f64 / fps;
    let [min, max] = [param(comp, layer, "min", time)?, param(comp, layer, "max", time)?].map(serde_json::from_value::<[f64; 2]>);
    let search = param(comp, layer, "search", time)?.as_f64().ok_or("the search range is not a number")?;
    let (min, max) = (min?, max?);
    Ok(Request { comp, layer, min, max, search, first, frames, fps })
}

fn transform(tree: &serde_json::Value) -> Vec<&serde_json::Value> {
    let groups = tree["properties"]["children"].as_array().into_iter().flatten();
    groups.filter(|group| group["match"] == "transform").flat_map(|group| group["children"].as_array().into_iter().flatten()).collect()
}

fn flat_2d(tree: &serde_json::Value, role: &str) -> Result<()> {
    let separated = transform(tree).iter().any(|prop| prop["match"] == "positionX");
    if !tree["parent"].is_null() || tree["switches"]["three_d"] == true || separated {
        return Err(format!("the {role} layer must be a 2D layer with no parent and one Position property").into());
    }
    Ok(())
}

fn numbers(value: &serde_json::Value) -> Vec<f64> {
    value.as_f64().map_or_else(|| value.as_array().into_iter().flatten().filter_map(serde_json::Value::as_f64).collect(), |one| vec![one])
}

fn layer_to_comp(tree: &serde_json::Value) -> Result<[f64; 2]> {
    let props = transform(tree);
    let prop = |id: &str| props.iter().find(|prop| prop["match"] == id);
    let geometry = ["anchor", "position", "scale", "rotation", "skew"];
    let moving = props.iter().any(|prop| geometry.iter().any(|id| prop["match"] == *id) && (prop.get("keys").is_some() || !prop["expression"].is_null()));
    let at_rest = |id: &str, rest: f64| prop(id).is_none_or(|prop| numbers(&prop["value"]).iter().all(|v| (v - rest).abs() < 1e-9));
    if moving || !at_rest("scale", 100.0) || !at_rest("rotation", 0.0) || !at_rest("skew", 0.0) {
        return Err("the tracked layer must not be scaled, rotated, skewed or animated, so its pixels map to the composition by one offset".into());
    }
    let point = |id: &str, i: usize| prop(id).and_then(|prop| numbers(&prop["value"]).get(i).copied()).unwrap_or(0.0);
    Ok([0, 1].map(|i| point("position", i) - point("anchor", i)))
}

pub fn apply(source: u64, target: u64) -> Result<usize> {
    let comp = host::command("comp.info", json!({}))?["id"].as_u64().ok_or("there is no active composition")?;
    let bytes = host::data(Place::Layer(source), KEY)?.ok_or("the first selected layer has no track: track it first")?;
    let track: Track = serde_json::from_slice(&bytes)?;
    let tree = |layer: u64| host::command("layer.tree", json!({"comp": comp, "layer": layer, "depth": 2, "time": track.start}));
    let source_tree = tree(source)?;
    flat_2d(&source_tree, "tracked")?;
    flat_2d(&tree(target)?, "target")?;
    let offset = layer_to_comp(&source_tree)?;
    let steps: Vec<_> = (track.points.iter().enumerate())
        .map(|(frame, at)| {
            let time = track.start + frame as f64 / track.fps;
            let value = [at[0] + offset[0], at[1] + offset[1]];
            json!({"command": "prop.addKey", "params": {"comp": comp, "layer": target, "path": "transform/position", "time": time, "timeBase": "comp", "value": value}})
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
            TaskStatus::Done => "Track saved.".into(),
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
