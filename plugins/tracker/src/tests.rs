use craft_plugin::{FrameRequest, Place};
use craft_plugin_host::testing::{Picture, TestPlugin, json};

use crate::track::{KEY, Track};

const FPS: f64 = 10.0;
const FRAMES: usize = 12;
const SIDE: f64 = 16.0;
const LAYER: u64 = 7;

fn truth(frame: f64) -> [f64; 2] {
    [60.0 + 2.3 * frame, 50.0 + 1.7 * frame]
}

fn cover(low: f64, high: f64, pixel: f64) -> f64 {
    (high.min(pixel + 1.0) - low.max(pixel)).clamp(0.0, 1.0)
}

fn square(request: &FrameRequest) -> Picture {
    let centre = truth(request.time * FPS);
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

fn args() -> serde_json::Value {
    let [x, y] = truth(0.0);
    json!({"layer": LAYER, "min": [x - 12.0, y - 12.0], "max": [x + 12.0, y + 12.0], "search": 6.0, "from": 0.0, "to": FRAMES as f64 / FPS, "fps": FPS})
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
fn cancel_stops_the_track_before_it_writes() {
    let plugin = TestPlugin::of(super::plugin);
    plugin.frames(square);
    let started = plugin.start("track", args()).unwrap();
    assert!(plugin.cancel(started));
    assert_eq!(plugin.wait(started), Err(format!("task {started} was cancelled")));
    assert_eq!(plugin.data(Place::Layer(LAYER), KEY), None);
}

#[test]
fn the_stabilised_sequence_holds_the_square_within_half_a_pixel() {
    let (plugin, track) = tracked();
    plugin.set_data(Place::OwnLayer, KEY, &serde_json::to_vec(&track).unwrap());
    let still = centroid(&square(&FrameRequest { comp: None, layer: None, time: 0.0, scale: 1.0 }));
    for frame in 0..FRAMES {
        let time = frame as f64 / FPS;
        let input = square(&FrameRequest { comp: None, layer: None, time, scale: 1.0 });
        let held = centroid(&plugin.render("stabilise").input(input).time(time).run().unwrap());
        let error = (0..2).map(|i| (held[i] - still[i]).abs()).fold(0.0, f64::max);
        assert!(error < 0.5, "frame {frame}: square at {held:?}, first frame {still:?}");
    }
}

#[test]
fn apply_writes_one_position_keyframe_per_tracked_frame_in_one_batch() {
    let (plugin, track) = tracked();
    let mut panel = plugin.panel("tracker").unwrap();
    panel.app_event(craft_plugin::AppEvent::Selection { layers: vec![LAYER, 9], properties: vec![], keyframes: 0 }).unwrap();
    panel.click("Apply to Layer").unwrap();
    let (command, batch) = plugin.commands().pop().unwrap();
    assert_eq!((command.as_str(), &batch["label"]), ("engine.batch", &json!("Tracker: Apply Track")));
    let steps = batch["steps"].as_array().unwrap();
    assert_eq!(steps.len(), FRAMES);
    for (frame, (step, point)) in steps.iter().zip(&track.points).enumerate() {
        let expected = json!({"layer": 9, "path": "transform/position", "time": frame as f64 / FPS, "timeBase": "comp", "value": point});
        assert_eq!((&step["command"], &step["params"]), (&json!("prop.addKey"), &expected));
    }
    assert!(panel.texts().contains(&format!("Wrote {FRAMES} position keyframes. One undo removes them.")), "{:?}", panel.texts());
}

#[test]
fn track_reads_the_box_from_the_effect_on_the_selected_layer() {
    let plugin = TestPlugin::of(super::plugin);
    plugin.frames(square);
    plugin.answer("comp.info", json!({"frameRate": FPS, "workArea": [0.0, 0.3]}));
    plugin.reply("prop.get", |params| match params["path"].as_str() {
        Some("effects/Track and Stabilise/min") => json!({"value": [48.0, 38.0]}),
        Some("effects/Track and Stabilise/max") => json!({"value": [72.0, 62.0]}),
        _ => json!({"value": 6.0}),
    });
    let mut panel = plugin.panel("tracker").unwrap();
    panel.app_event(craft_plugin::AppEvent::Selection { layers: vec![LAYER], properties: vec![], keyframes: 0 }).unwrap();
    panel.click("Track").unwrap();
    let track: Track = serde_json::from_value(plugin.wait(1).unwrap()).unwrap();
    assert_eq!((track.points.len(), track.points[0]), (3, [60.0, 50.0]));
}
