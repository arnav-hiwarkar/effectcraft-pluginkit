use std::sync::{Arc, Mutex, MutexGuard};

use effectcraft_effects::dynamic::{self, DynamicEffect, DynamicParam};
use effectcraft_keyframe::Value as KV;
use effectcraft_project::ParamUi;
use serde_json::{Value, json};

use crate::Session;
use crate::rq_tests::MockExporter;

const OWNER: &str = "test.refs.glow";
const EFFECT: &str = "test.refs.glow.glow";

fn register() -> MutexGuard<'static, ()> {
    static REGISTERED: Mutex<()> = Mutex::new(());
    let serial = REGISTERED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    dynamic::register(DynamicEffect {
        owner: OWNER.into(),
        version: "2.1.0".into(),
        id: EFFECT.into(),
        name: "Refs Glow".into(),
        category: "Test".into(),
        params: vec![DynamicParam { id: "amount".into(), name: "Amount".into(), default: KV::Scalar(1.0), ui: ParamUi::default() }],
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
        aux: Vec::new(),
    });
    serial
}

fn effect_group(path: &std::path::Path) -> Value {
    let text: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut found = Vec::new();
    collect(&text, &mut found);
    assert_eq!(found.len(), 1, "{text}");
    found.remove(0)
}

fn collect(v: &Value, found: &mut Vec<Value>) {
    match v {
        Value::Object(m) if m.get("kind").is_some_and(|k| k["effect"] == EFFECT) => found.push(v.clone()),
        Value::Object(m) => m.values().for_each(|c| collect(c, found)),
        Value::Array(a) => a.iter().for_each(|c| collect(c, found)),
        _ => {}
    }
}

#[test]
fn a_project_names_its_missing_plugin_refuses_a_final_render_and_keeps_the_values() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("first.ecproj"), dir.path().join("second.ecproj"));
    let _registered = register();
    let mut s = Session::new();
    s.execute("comp.new", json!({"name": "Main"})).unwrap();
    let layer = s.execute("layer.newSolid", json!({})).unwrap()["layer"].clone();
    s.execute("effect.apply", json!({"layer": layer, "effect": EFFECT})).unwrap();
    s.execute("prop.set", json!({"layer": layer, "path": "effects/#1/amount", "value": 0.123456789})).unwrap();
    s.execute("renderQueue.add", json!({"output": dir.path().join("out.mp4")})).unwrap();
    s.execute("file.saveAs", json!({"path": first})).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&first).unwrap()).unwrap();
    assert_eq!(saved["plugins"], json!([{"id": OWNER, "version": "2.1.0"}]));

    dynamic::unregister(OWNER);
    let mut s = Session::new();
    s.exporter = Some(Arc::new(MockExporter { log: Default::default() }));
    let opened = s.execute("file.open", json!({"path": first})).unwrap();
    assert_eq!(opened["plugins"], json!([{"id": OWNER, "version": "2.1.0", "installed": null, "status": "missing"}]));
    let refused = s.execute("renderQueue.render", json!({})).unwrap_err().to_string();
    assert!(refused.contains(OWNER), "{refused}");
    let rendered = s.execute("renderQueue.render", json!({"withoutMissingPlugins": true})).unwrap();
    assert_eq!(rendered["items"][0]["status"], "Done", "{rendered}");

    s.execute("file.saveAs", json!({"path": second})).unwrap();
    assert_eq!(effect_group(&second), effect_group(&first));
    let resaved: Value = serde_json::from_str(&std::fs::read_to_string(&second).unwrap()).unwrap();
    assert_eq!(resaved["plugins"], saved["plugins"]);
}

#[test]
fn an_old_project_without_a_plugin_record_is_refused_for_a_missing_plugin_effect_and_renders_with_the_option() {
    let dir = tempfile::tempdir().unwrap();
    let (saved, old) = (dir.path().join("saved.ecproj"), dir.path().join("old.ecproj"));
    let _registered = register();
    let mut s = Session::new();
    s.execute("comp.new", json!({"name": "Main"})).unwrap();
    let layer = s.execute("layer.newSolid", json!({})).unwrap()["layer"].clone();
    s.execute("effect.apply", json!({"layer": layer, "effect": EFFECT})).unwrap();
    s.execute("renderQueue.add", json!({"output": dir.path().join("out.mp4")})).unwrap();
    s.execute("file.saveAs", json!({"path": saved})).unwrap();
    let mut text: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert!(text.as_object_mut().unwrap().remove("plugins").is_some());
    std::fs::write(&old, text.to_string()).unwrap();

    dynamic::unregister(OWNER);
    let mut s = Session::new();
    s.exporter = Some(Arc::new(MockExporter { log: Default::default() }));
    let opened = s.execute("file.open", json!({"path": old})).unwrap();
    assert_eq!(opened["plugins"], json!([]));
    let refused = s.execute("renderQueue.render", json!({})).unwrap_err().to_string();
    assert!(refused.contains(EFFECT) && refused.contains("withoutMissingPlugins"), "{refused}");
    let rendered = s.execute("renderQueue.render", json!({"withoutMissingPlugins": true})).unwrap();
    assert_eq!(rendered["items"][0]["status"], "Done", "{rendered}");
}

#[test]
fn a_missing_effect_counts_only_in_the_queued_comps_and_the_comps_they_nest() {
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("saved.ecproj");
    let _registered = register();
    let mut s = Session::new();
    let inner = s.execute("comp.new", json!({"name": "Inner"})).unwrap()["comp"].clone();
    let layer = s.execute("layer.newSolid", json!({})).unwrap()["layer"].clone();
    s.execute("effect.apply", json!({"layer": layer, "effect": EFFECT})).unwrap();
    let outer = s.execute("comp.new", json!({"name": "Outer"})).unwrap()["comp"].clone();
    s.execute("layer.addItem", json!({"comp": outer, "item": inner})).unwrap();
    let clean = s.execute("comp.new", json!({"name": "Clean"})).unwrap()["comp"].clone();
    s.execute("layer.newSolid", json!({"comp": clean})).unwrap();
    s.execute("file.saveAs", json!({"path": saved})).unwrap();

    dynamic::unregister(OWNER);
    let mut s = Session::new();
    s.exporter = Some(Arc::new(MockExporter { log: Default::default() }));
    s.execute("file.open", json!({"path": saved})).unwrap();
    s.execute("renderQueue.add", json!({"comp": clean, "output": dir.path().join("clean.mp4")})).unwrap();
    let rendered = s.execute("renderQueue.render", json!({})).unwrap();
    assert_eq!(rendered["items"][0]["status"], "Done", "{rendered}");

    s.execute("renderQueue.add", json!({"comp": outer, "output": dir.path().join("outer.mp4")})).unwrap();
    let refused = s.execute("renderQueue.render", json!({})).unwrap_err().to_string();
    assert!(refused.contains(EFFECT), "{refused}");
}
