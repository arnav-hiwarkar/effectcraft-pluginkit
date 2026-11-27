use serde_json::{Value, json};

use crate::{App, app};

fn effect_group<'a>(value: &'a mut Value, effect: &str) -> Option<&'a mut Value> {
    if value.get("match").and_then(Value::as_str) == Some(effect) {
        return Some(value);
    }
    match value {
        Value::Object(map) => map.values_mut().find_map(|inner| effect_group(inner, effect)),
        Value::Array(items) => items.iter_mut().find_map(|item| effect_group(item, effect)),
        _ => None,
    }
}

fn saved(app: &mut App, name: &str) -> (std::path::PathBuf, Value) {
    let path = app.bundle.path().join(name);
    app.session.execute("file.saveAs", json!({"path": path})).unwrap();
    let project = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    (path, project)
}

fn open(app: &mut App, path: &std::path::Path, project: &Value) -> String {
    std::fs::write(path, project.to_string()).unwrap();
    app.session.drain_events();
    app.session.execute("file.open", json!({"path": path})).unwrap();
    format!("{:?}", app.session.drain_events())
}

#[test]
fn a_renamed_parameter_moves_on_open_and_state_from_a_newer_plugin_is_kept_and_reported() {
    let mut app = app("test.state", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Plate", "color": "#000000", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Plate", "effect": "test.state.renamed"})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Plate", "effect": "test.state.invert"})).unwrap();
    let (path, mut project) = saved(&mut app, "current.ecproj");
    assert_eq!(effect_group(&mut project, "test.state.renamed").unwrap()["state_version"], json!(1));
    let invert = effect_group(&mut project, "test.state.invert").unwrap().clone();
    assert_eq!(invert.get("state_version"), None);

    let renamed = effect_group(&mut project, "test.state.renamed").unwrap();
    renamed.as_object_mut().unwrap().remove("state_version");
    let strength = renamed["children"].as_array_mut().unwrap().iter_mut().find(|prop| prop["match"] == "strength").unwrap();
    strength["match"] = json!("amt");
    strength["value"] = json!({"t": "Scalar", "v": 0.25});
    let events = open(&mut app, &path, &project);
    assert!(!events.contains("newer version"), "{events}");
    let (_, mut reopened) = saved(&mut app, "migrated.ecproj");
    let renamed = effect_group(&mut reopened, "test.state.renamed").unwrap();
    assert_eq!(renamed["state_version"], json!(1));
    let props: Vec<(Value, Value)> = renamed["children"].as_array().unwrap().iter().map(|prop| (prop["match"].clone(), prop["value"].clone())).collect();
    assert_eq!(props, [(json!("strength"), json!({"t": "Scalar", "v": 0.25}))]);
    assert_eq!(effect_group(&mut reopened, "test.state.invert").unwrap(), &invert);

    let newer = effect_group(&mut reopened, "test.state.renamed").unwrap();
    newer["state_version"] = json!(7);
    let mut falloff = newer["children"][0].clone();
    falloff["uid"] = json!(9000);
    falloff["match"] = json!("falloff");
    falloff["value"] = json!({"t": "Str", "v": "smooth"});
    newer["children"].as_array_mut().unwrap().push(falloff);
    let written = newer.clone();
    let events = open(&mut app, &path, &reopened);
    assert!(events.contains("Plate: Renamed Parameter was saved by a newer version of its plug-in (state version 7, the installed plug-in has 1)"), "{events}");
    let (_, mut kept) = saved(&mut app, "kept.ecproj");
    assert_eq!(effect_group(&mut kept, "test.state.renamed").unwrap(), &written);
}
