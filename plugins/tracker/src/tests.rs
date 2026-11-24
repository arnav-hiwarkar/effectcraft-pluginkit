use craft_plugin::{FrameRequest, Place};
use craft_plugin_host::testing::{Picture, TestPlugin, json};
use serde_json::Value;

use crate::track::{KEY, Track};

const FPS: f64 = 10.0;
const FRAMES: usize = 12;
const SIDE: f64 = 16.0;
const COMP: u64 = 5;
const LAYER: u64 = 7;
const TARGET: u64 = 9;

fn truth(frame: f64) -> [f64; 2] {
    [60.0 + 2.3 * frame, 50.0 + 1.7 * frame]
}

fn cover(low: f64, high: f64, pixel: f64) -> f64 {
    (high.min(pixel + 1.0) - low.max(pixel)).clamp(0.0, 1.0)
}

fn square_at(time: f64) -> Picture {
    let centre = truth(time * FPS);
    let [left, top] = centre.map(|c| c - SIDE / 2.0);
    let (width, height) = (160, 120);
    let pixels = (0..width * height)
        .map(|i| {
            let (x, y) = (f64::from(i % width), f64::from(i / width));
            let v = (0.1 + 0.8 * cover(left, left + SIDE, x) * cover(top, top + SIDE, y)) as f32;
            [v, v, v, 1.0]
        })
        .collect();
    Picture { width, height, pixels }
}

fn square(request: &FrameRequest) -> Result<Picture, String> {
    let pinned = request.ids && request.source && request.comp == Some(COMP.to_string()) && request.layer == Some(LAYER.to_string());
    if !pinned {
        return Err(format!("the tracker must read layer {LAYER} of comp {COMP} by id before its effects, not {request:?}"));
    }
    Ok(square_at(request.time))
}

fn args() -> Value {
    let [x, y] = truth(0.0);
    json!({"comp": COMP, "layer": LAYER, "min": [x - 12.0, y - 12.0], "max": [x + 12.0, y + 12.0], "search": 6.0, "first": 0, "frames": FRAMES, "fps": FPS})
}

fn tracked() -> (TestPlugin, Track) {
    let plugin = TestPlugin::of(super::plugin);
    plugin.frames(square);
    let track: Track = serde_json::from_value(plugin.task("track", args()).unwrap()).unwrap();
    (plugin, track)
}

fn centroid(picture: &Picture) -> [f64; 2] {
    let mut sum = [0.0; 3];
    for (i, pixel) in picture.pixels.iter().enumerate() {
        let weight = (f64::from(pixel[0]) - 0.1).max(0.0);
        let (x, y) = ((i as u32 % picture.width) as f64 + 0.5, (i as u32 / picture.width) as f64 + 0.5);
        sum = [sum[0] + weight * x, sum[1] + weight * y, sum[2] + weight];
    }
    [sum[0] / sum[2], sum[1] / sum[2]]
}

fn old_track() -> Vec<u8> {
    serde_json::to_vec(&Track { version: 1, start: 0.0, fps: FPS, points: vec![[1.0, 2.0]] }).unwrap()
}

#[test]
fn a_moving_square_is_tracked_within_a_quarter_pixel_and_stored_on_the_layer() {
    let (plugin, track) = tracked();
    assert_eq!(track.points.len(), FRAMES);
    for (frame, point) in track.points.iter().enumerate() {
        let error = (0..2).map(|i| (point[i] - truth(frame as f64)[i]).abs()).fold(0.0, f64::max);
        assert!(error < 0.25, "frame {frame}: tracked {point:?}, truth {:?}", truth(frame as f64));
    }
    assert_eq!(plugin.data(Place::Layer(LAYER), KEY), Some(serde_json::to_vec(&track).unwrap()));
}

#[test]
fn cancel_or_a_failed_frame_keeps_the_saved_track() {
    let plugin = TestPlugin::of(super::plugin);
    plugin.set_data(Place::Layer(LAYER), KEY, &old_track()).unwrap();
    plugin.frames(square);
    let started = plugin.start("track", args()).unwrap();
    assert!(plugin.cancel(started));
    assert_eq!(plugin.wait(started), Err(format!("task {started} was cancelled")));
    assert_eq!(plugin.data(Place::Layer(LAYER), KEY), Some(old_track()));
    plugin.frames(|request| if request.time > 0.25 { Err("the layer is gone".into()) } else { square(request) });
    assert_eq!(plugin.task("track", args()), Err("the layer is gone".into()));
    assert_eq!(plugin.data(Place::Layer(LAYER), KEY), Some(old_track()));
}

#[test]
fn a_box_that_rounds_to_nothing_or_leaves_the_layer_is_refused() {
    let plugin = TestPlugin::of(super::plugin);
    plugin.frames(square);
    for (min, max) in [([60.2, 40.0], [60.4, 60.0]), ([150.0, 40.0], [170.0, 60.0])] {
        let error = plugin.task("track", json!({"comp": COMP, "layer": LAYER, "min": min, "max": max, "search": 6.0, "first": 0, "frames": 3, "fps": FPS}));
        assert!(error.unwrap_err().contains("at least one pixel wide and tall inside the 160×120 layer"));
    }
    assert_eq!(plugin.data(Place::Layer(LAYER), KEY), None);
}

#[test]
fn the_stabilised_sequence_holds_the_square_within_half_a_pixel() {
    let (plugin, track) = tracked();
    plugin.set_data(Place::OwnLayer, KEY, &serde_json::to_vec(&track).unwrap()).unwrap();
    let still = centroid(&square_at(0.0));
    for frame in 0..FRAMES {
        let time = frame as f64 / FPS;
        let held = centroid(&plugin.render("stabilise").input(square_at(time)).time(time).run().unwrap());
        let error = (0..2).map(|i| (held[i] - still[i]).abs()).fold(0.0, f64::max);
        assert!(error < 0.5, "frame {frame}: square at {held:?}, first frame {still:?}");
    }
}

fn tree(parent: Value, three_d: bool, transform: Value) -> Value {
    json!({"parent": parent, "switches": {"three_d": three_d}, "properties": {"children": [{"match": "transform", "children": transform}]}})
}

fn flat() -> Value {
    json!([{"match": "anchor", "value": [80.0, 60.0, 0.0]}, {"match": "position", "value": [90.0, 55.0, 0.0]}, {"match": "scale", "value": [100.0, 100.0, 100.0]}, {"match": "rotation", "value": 0.0}])
}

fn apply_with(target: Value) -> (TestPlugin, Track, Vec<String>) {
    let (plugin, track) = tracked();
    plugin.answer("comp.info", json!({"id": COMP}));
    plugin.reply("layer.tree", move |params| Ok(if params["layer"] == TARGET { target.clone() } else { tree(Value::Null, false, flat()) }));
    let mut panel = plugin.panel("tracker").unwrap();
    panel.app_event(craft_plugin::AppEvent::Selection { layers: vec![LAYER, TARGET], properties: vec![], keyframes: 0 }).unwrap();
    panel.click("Apply to Layer").unwrap();
    let texts = panel.texts();
    drop(panel);
    (plugin, track, texts)
}

#[test]
fn apply_writes_one_comp_space_position_keyframe_per_tracked_frame_in_one_batch() {
    let (plugin, track, texts) = apply_with(tree(Value::Null, false, flat()));
    let (command, batch) = plugin.commands().pop().unwrap();
    assert_eq!((command.as_str(), &batch["label"]), ("engine.batch", &json!("Tracker: Apply Track")));
    let steps = batch["steps"].as_array().unwrap();
    assert_eq!(steps.len(), FRAMES);
    for (frame, (step, point)) in steps.iter().zip(&track.points).enumerate() {
        let value = [point[0] + 10.0, point[1] - 5.0];
        let expected = json!({"comp": COMP, "layer": TARGET, "path": "transform/position", "time": frame as f64 / FPS, "timeBase": "comp", "value": value});
        assert_eq!((&step["command"], &step["params"]), (&json!("prop.addKey"), &expected));
    }
    assert!(texts.contains(&format!("Wrote {FRAMES} position keyframes. One undo removes them.")), "{texts:?}");
}

#[test]
fn apply_refuses_separated_3d_or_parented_targets_before_writing() {
    let mut separated = flat();
    separated.as_array_mut().unwrap().push(json!({"match": "positionX", "value": 0.0}));
    for target in [tree(Value::Null, false, separated), tree(Value::Null, true, flat()), tree(json!(3), false, flat())] {
        let (plugin, _, texts) = apply_with(target);
        assert!(plugin.commands().iter().all(|(command, _)| command != "engine.batch"));
        assert!(texts.iter().any(|text| text.contains("the target layer must be a 2D layer with no parent and one Position property")), "{texts:?}");
    }
}

#[test]
fn track_samples_the_comp_frame_grid_and_reads_the_box_at_its_first_frame() {
    let plugin = TestPlugin::of(super::plugin);
    plugin.frames(square);
    plugin.answer("comp.info", json!({"id": COMP, "frameRate": FPS, "workArea": [0.06, 0.24]}));
    plugin.reply("prop.get", |params| {
        Ok(match params["path"].as_str() {
            Some("effects/Track and Stabilise/min") => json!({"value": [50.3, 39.7]}),
            Some("effects/Track and Stabilise/max") => json!({"value": [74.3, 63.7]}),
            _ => json!({"value": 6.0}),
        })
    });
    let mut panel = plugin.panel("tracker").unwrap();
    panel.app_event(craft_plugin::AppEvent::Selection { layers: vec![LAYER], properties: vec![], keyframes: 0 }).unwrap();
    panel.click("Track").unwrap();
    let track: Track = serde_json::from_value(plugin.wait(1).unwrap()).unwrap();
    assert_eq!((track.start, track.points.len()), (0.1, 2));
    let reads: Vec<_> = plugin
        .commands()
        .into_iter()
        .filter(|(command, _)| command == "prop.get")
        .map(|(_, params)| (params["comp"].clone(), params["time"].clone()))
        .collect();
    assert!(reads.iter().all(|read| *read == (json!(COMP), json!(0.1))), "{reads:?}");
}
