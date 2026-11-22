use effectcraft_plugin::TaskStatus;
use serde_json::{Value, json};

use crate::{App, app};

static ONE_SESSION_PUMPS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn finished(app: &mut App, task: &str, args: Value) -> Value {
    let id = effectcraft_plugin::start_task(&mut app.session, &app.id, task, args).expect("the task starts");
    let tasks = app.session.execute("plugin.tasks", json!({"wait": true, "timeout": 30})).expect("the tasks finish");
    let state = tasks.as_array().unwrap().iter().find(|t| t["id"] == json!(id) && t["plugin"] == json!(app.id)).cloned().expect("listed");
    assert_eq!(state["status"], json!(TaskStatus::Done), "{state}");
    state["result"].clone()
}

fn solid(app: &mut App, name: &str, color: &str, size: u32) -> u64 {
    let made = app.session.execute("layer.newSolid", json!({"name": name, "color": color, "width": size, "height": size})).unwrap();
    made["layer"].as_u64().unwrap()
}

#[test]
fn a_task_renders_the_comp_or_one_layer_with_its_effects() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-frame", &["document.read"]);
    solid(&mut app, "Red", "#ff0000", 64);
    solid(&mut app, "Green", "#00ff00", 16);

    let comp = finished(&mut app, "frame", json!({"time": 0.0, "scale": 0.5}));
    assert_eq!((comp["width"].clone(), comp["height"].clone()), (json!(32), json!(18)));
    assert_eq!(comp["center"], json!([0.0, 1.0, 0.0, 1.0]));

    let red = finished(&mut app, "frame", json!({"time": 0.0, "scale": 0.5, "layer": "Red"}));
    assert_eq!(red["center"], json!([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn a_task_reads_the_comp_mix_or_one_layer_of_audio() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-audio", &["document.read"]);
    let loud = solid(&mut app, "Loud", "#808080", 8);
    solid(&mut app, "Quiet", "#808080", 8);
    app.session.execute("effect.apply", json!({"layers": [loud], "effect": "ec.audio.tone"})).unwrap();
    app.session.execute("layer.setSwitch", json!({"layers": [loud], "switch": "audio", "value": true})).unwrap();

    let mix = finished(&mut app, "audio", json!({"duration": 0.5}));
    assert_eq!((mix["channels"].clone(), mix["rate"].clone(), mix["len"].clone()), (json!(2), json!(8000), json!(8000)));
    assert!(mix["peak"].as_f64().unwrap() > 0.2, "{mix}");
    assert_eq!(finished(&mut app, "audio", json!({"duration": 0.5, "layer": "Loud"}))["peak"], mix["peak"]);
    assert_eq!(finished(&mut app, "audio", json!({"duration": 0.5, "layer": "Quiet"}))["peak"], json!(0.0));
}

#[test]
fn a_task_cannot_ask_for_a_frame_larger_than_the_host_will_render() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-huge", &["document.read"]);
    let mut failure = |width: u32, height: u32, scale: f64| {
        app.session.execute("comp.new", json!({"name": format!("Wide {width}"), "width": width, "height": height, "frameRate": 24, "duration": 1})).unwrap();
        let id = effectcraft_plugin::start_task(&mut app.session, &app.id, "frame", json!({"time": 0.0, "scale": scale})).unwrap();
        let tasks = app.session.execute("plugin.tasks", json!({"wait": true, "timeout": 30})).unwrap();
        let state = tasks.as_array().unwrap().iter().find(|t| t["id"] == json!(id) && t["plugin"] == json!(app.id)).cloned().unwrap();
        (state["status"].clone(), state["error"].as_str().unwrap_or_default().to_string())
    };
    let (status, error) = failure(4200, 4, 4.0);
    assert_eq!(status, json!(TaskStatus::Failed), "{error}");
    assert!(error.contains("16800×16 frame is too large"), "{error}");
    let (status, error) = failure(2100, 2100, 4.0);
    assert_eq!(status, json!(TaskStatus::Failed), "{error}");
    assert!(error.contains("too large"), "{error}");
}

#[test]
fn a_task_started_in_one_project_cannot_edit_the_project_opened_after_it() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-closed", &["document.write"]);
    let id = effectcraft_plugin::start_task(&mut app.session, &app.id, "edit", json!({})).unwrap();
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    let tasks = app.session.execute("plugin.tasks", json!({"wait": true, "timeout": 30})).unwrap();
    let state = tasks.as_array().unwrap().iter().find(|t| t["id"] == json!(id) && t["plugin"] == json!(app.id)).cloned().unwrap();
    assert_eq!(state["status"], json!(TaskStatus::Cancelled), "{state}");
    assert_eq!(app.layer_names(), Vec::<String>::new());
}

#[test]
fn a_frame_of_one_layer_keeps_its_track_matte() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-solo", &["document.read"]);
    let plate = solid(&mut app, "Plate", "#0000ff", 64);
    let matte = solid(&mut app, "Matte", "#ff0000", 8);
    app.session.execute("prop.set", json!({"layer": matte, "path": "transform/position", "value": [4, 4]})).unwrap();
    app.session.execute("layer.setTrackMatte", json!({"layer": plate, "matte": matte, "kind": "alpha"})).unwrap();
    app.session.execute("layer.setSwitch", json!({"layers": [matte], "switch": "video", "value": true})).unwrap();

    let corner = finished(&mut app, "frame", json!({"time": 0.0, "scale": 1.0}));
    assert_eq!(corner["center"], json!([0.0, 0.0, 0.0, 0.0]));
    let alone = finished(&mut app, "frame", json!({"time": 0.0, "scale": 1.0, "layer": "Plate"}));
    assert_eq!(alone["center"], json!([0.0, 0.0, 0.0, 0.0]));
}

#[test]
fn a_frame_of_one_3d_layer_is_lit_by_the_comp_lights() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-solo-lit", &["document.read"]);
    let plate = solid(&mut app, "Plate", "#ffffff", 64);
    app.session.execute("layer.setSwitch", json!({"layers": [plate], "switch": "threeD", "value": true})).unwrap();
    app.session.execute("layer.newLight", json!({"kind": "Ambient", "intensity": 25})).unwrap();
    solid(&mut app, "Corner", "#ff0000", 4);

    let lit = finished(&mut app, "frame", json!({"time": 0.0, "scale": 1.0, "layer": "Plate"}));
    let quarter = 64.0f32 / 255.0;
    assert_eq!(lit["center"], json!([quarter, quarter, quarter, 1.0]));
}

#[test]
fn a_frame_of_one_layer_whose_video_is_off_shows_its_pixels() {
    let _alone = ONE_SESSION_PUMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = app("test.media-solo-hidden", &["document.read"]);
    let hidden = solid(&mut app, "Hidden", "#00ff00", 64);
    app.session.execute("layer.setSwitch", json!({"layers": [hidden], "switch": "video", "value": false})).unwrap();
    assert_eq!(finished(&mut app, "frame", json!({"time": 0.0, "scale": 1.0}))["center"], json!([0.0, 0.0, 0.0, 0.0]));
    assert_eq!(finished(&mut app, "frame", json!({"time": 0.0, "scale": 1.0, "layer": "Hidden"}))["center"], json!([0.0, 1.0, 0.0, 1.0]));
}
