use effectcraft_plugin::TaskStatus;
use serde_json::{Value, json};

use crate::{App, app};

fn finished(app: &mut App, task: &str, args: Value) -> Value {
    let id = effectcraft_plugin::start_task(&app.id, task, args).expect("the task starts");
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
    let mut app = app("test.media-audio", &["document.read"]);
    let loud = solid(&mut app, "Loud", "#808080", 8);
    solid(&mut app, "Quiet", "#808080", 8);
    app.session.execute("effect.apply", json!({"layers": [loud], "effect": "ec.audio.tone"})).unwrap();
    app.session.execute("layer.switch", json!({"layer": loud, "switch": "audio", "value": true})).ok();

    let mix = finished(&mut app, "audio", json!({"duration": 0.5}));
    assert_eq!((mix["channels"].clone(), mix["rate"].clone(), mix["len"].clone()), (json!(2), json!(8000), json!(8000)));
    assert!(mix["peak"].as_f64().unwrap() > 0.2, "{mix}");
    assert_eq!(finished(&mut app, "audio", json!({"duration": 0.5, "layer": "Loud"}))["peak"], mix["peak"]);
    assert_eq!(finished(&mut app, "audio", json!({"duration": 0.5, "layer": "Quiet"}))["peak"], json!(0.0));
}
