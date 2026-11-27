use effectcraft_effects::dynamic::{self, DynamicEffect, DynamicParam};
use effectcraft_keyframe::Value as KV;
use effectcraft_project::{LayerId, ParamUi};
use effectcraft_time::Tick;
use serde_json::{Value, json};

use crate::{KeyRef, Session};

const OWNER: &str = "test.greyed";
const EFFECT: &str = "test.greyed.manual";

fn register() {
    dynamic::register(DynamicEffect {
        owner: OWNER.into(),
        version: "1.0.0".into(),
        id: EFFECT.into(),
        name: "Greyed Manual".into(),
        category: "Test".into(),
        params: vec![
            DynamicParam { id: "manual".into(), name: "Manual".into(), default: KV::Bool(false), ui: ParamUi::Checkbox },
            DynamicParam { id: "amount".into(), name: "Amount".into(), default: KV::Scalar(1.0), ui: ParamUi::default() },
        ],
        render: Box::new(|ctx, buf| ctx.unchanged(buf)),
        shader: None,
        gpu: None,
        time_dependent: false,
        reads_other_frames: false,
        reads_comp_scene: false,
        reads_param_history: false,
        presets: Vec::new(),
        revision: 1,
        files: None,
        margin: None,
        state_version: 0,
        migrate: None,
        param_state: Some(Box::new(|param, value| [true, param != "amount" || value("manual").is_some_and(|v| v.as_bool())])),
        aux: Vec::new(),
    });
}

fn setup() -> (Session, u64, u64) {
    register();
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"width": 64, "height": 64, "frameRate": 30, "duration": 4})).unwrap();
    let layer = s.execute_checked("layer.newSolid", json!({"name": "S", "color": "#000"})).unwrap()["layer"].as_u64().unwrap();
    s.execute_checked("effect.apply", json!({"layer": layer, "effect": EFFECT})).unwrap();
    for (time, on) in [(0.0, true), (1.0, false)] {
        s.execute_checked("prop.addKey", json!({"layer": layer, "path": "effects/#1/manual", "time": time, "value": on})).unwrap();
    }
    s.execute_checked("prop.addKey", json!({"layer": layer, "path": "effects/#1/amount", "time": 0.0, "value": 1.0})).unwrap();
    let amount = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("effects/#1/amount").unwrap().uid;
    (s, layer, amount)
}

fn manual_at(s: &mut Session, layer: u64, time: f64) -> Value {
    s.execute_checked("prop.get", json!({"layer": layer, "path": "effects/#1/manual", "time": time})).unwrap()["value"].clone()
}

fn amount_keys(s: &Session, layer: u64, amount: u64) -> Vec<(f64, f64)> {
    let pr = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(amount).unwrap();
    pr.keys.iter().map(|k| (k.time.seconds(), k.value.as_f64())).collect()
}

fn set_at(s: &mut Session, layer: u64, playhead: f64, time: f64, value: f64) -> Result<Value, String> {
    s.execute_checked("time.set", json!({"time": playhead})).unwrap();
    s.execute_checked("prop.set", json!({"layer": layer, "path": "effects/#1/amount", "time": time, "value": value})).map_err(|e| e.to_string())
}

#[test]
fn a_greyed_parameter_is_judged_at_the_time_each_write_targets() {
    let (mut s, layer, amount) = setup();
    assert_eq!([manual_at(&mut s, layer, 0.5), manual_at(&mut s, layer, 2.0)], [json!(true), json!(false)]);

    let late = set_at(&mut s, layer, 0.5, 2.0, 3.0).unwrap_err();
    assert!(late.contains("`Amount` is turned off by another control of its effect at 2 s"), "{late}");
    assert_eq!(set_at(&mut s, layer, 2.0, 0.5, 4.0).unwrap(), json!(4.0));
    assert_eq!(amount_keys(&s, layer, amount), [(0.0, 1.0), (0.5, 4.0)]);

    s.state.selected_keys = vec![KeyRef { layer: LayerId(layer), prop: amount, time: Tick::ZERO }];
    s.execute_checked("keys.copy", json!({})).unwrap();
    let pasted = s.execute_checked("keys.paste", json!({"layer": layer, "prop": amount, "time": 2.0})).unwrap_err().to_string();
    assert!(pasted.contains("`Amount` is turned off by another control of its effect at 2 s"), "{pasted}");
    assert_eq!(s.execute_checked("keys.paste", json!({"layer": layer, "prop": amount, "time": 0.25})).unwrap(), json!(1));

    s.state.selected_keys = vec![KeyRef { layer: LayerId(layer), prop: amount, time: Tick::from_seconds_f64(0.5) }];
    let moved = s.execute_checked("keys.move", json!({"delta": 1.5})).unwrap_err().to_string();
    assert!(moved.contains("`Amount` is turned off by another control of its effect at 2 s"), "{moved}");
    assert_eq!(amount_keys(&s, layer, amount), [(0.0, 1.0), (0.25, 1.0), (0.5, 4.0)]);
}
