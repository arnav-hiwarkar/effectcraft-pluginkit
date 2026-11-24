use effectcraft_engine::render::{LayerCache, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use serde_json::{Value, json};

use crate::{App, app};

fn plate(app: &mut App) -> u64 {
    app.session.execute("layer.newSolid", json!({"name": "Plate", "color": "#000000", "width": 64, "height": 36})).unwrap();
    app.session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Plate").unwrap().id.0
}

fn write(app: &mut App, request: Value) -> Result<(), String> {
    std::fs::write(app.bundle.path().join("data.json"), request.to_string()).unwrap();
    app.run("data")
}

fn on_plate(app: &App) -> Option<Vec<u8>> {
    let layer = app.session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Plate").unwrap();
    layer.plugin_data.get(&app.id)?.get("track").cloned()
}

fn saved_data(path: &std::path::Path) -> Vec<Value> {
    fn collect(value: &Value, found: &mut Vec<Value>) {
        match value {
            Value::Object(map) => map.iter().for_each(|(key, inner)| if key == "plugin_data" { found.push(inner.clone()) } else { collect(inner, found) }),
            Value::Array(items) => items.iter().for_each(|item| collect(item, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    collect(&serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(), &mut found);
    found
}

#[test]
fn with_one_undo_level_a_command_that_writes_twice_is_still_one_undo_step() {
    let mut app = app("test.data-one-level", &["document.read", "document.write"]);
    let layer = plate(&mut app);
    app.session.prefs.general.undo_levels = 1;
    write(&mut app, json!({"place": {"layer": layer}, "key": "track", "value": "points", "null": true})).unwrap();
    assert_eq!(app.layer_names(), ["Marker", "Plate"]);
    assert_eq!(app.session.history.undo.len(), 1);
    assert!(app.session.undo());
    assert_eq!((on_plate(&app), app.layer_names()), (None, vec!["Plate".to_string()]));
}

#[test]
fn a_command_writes_layer_data_as_one_undo_step_and_saving_keeps_it_with_or_without_the_plugin() {
    let mut app = app("test.data-undo", &["document.read", "document.write"]);
    let layer = plate(&mut app);
    write(&mut app, json!({"place": {"layer": layer}, "key": "track", "value": "points", "null": true})).unwrap();
    assert_eq!(on_plate(&app), Some(b"points".to_vec()));
    assert_eq!(app.layer_names(), ["Marker", "Plate"]);
    let read = std::fs::read_to_string(app.plugins.get(&app.id).unwrap().data_dir.join("data-read.json")).unwrap();
    assert_eq!(read, "\"points\"");
    assert!(app.session.undo());
    assert_eq!((on_plate(&app), app.layer_names()), (None, vec!["Plate".to_string()]));
    assert!(app.session.redo());
    assert_eq!(on_plate(&app), Some(b"points".to_vec()));
    write(&mut app, json!({"place": "project", "key": "track", "value": "settings"})).unwrap();

    let first = app.bundle.path().join("first.ecproj");
    app.session.execute("file.saveAs", json!({"path": first})).unwrap();
    app.session.execute("file.open", json!({"path": first})).unwrap();
    assert_eq!(on_plate(&app), Some(b"points".to_vec()));
    let id = app.id.clone();
    let mut found = saved_data(&first);
    found.sort_by_key(Value::to_string);
    assert_eq!(found, [json!({&id: {"track": "c2V0dGluZ3M="}}), json!({&id: {"track": "cG9pbnRz"}})]);

    let module = std::fs::read(app.bundle.path().join("probe.wasm")).unwrap();
    std::fs::remove_file(app.bundle.path().join("probe.wasm")).unwrap();
    assert!(app.plugins.load_for_session(app.bundle.path()).is_err());
    app.session.execute("file.open", json!({"path": first})).unwrap();
    let second = app.bundle.path().join("second.ecproj");
    app.session.execute("file.saveAs", json!({"path": second})).unwrap();
    assert_eq!(saved_data(&second), saved_data(&first));
    std::fs::write(app.bundle.path().join("probe.wasm"), module).unwrap();
}

#[test]
fn an_effect_reading_its_layer_data_redraws_through_the_layer_cache_when_the_data_changes() {
    let mut app = app("test.data-render", &["document.read", "document.write"]);
    let layer = plate(&mut app);
    app.session.execute("effect.apply", json!({"layer": "Plate", "effect": "test.data-render.tinted"})).unwrap();
    let cache = LayerCache::default();
    let red = |app: &App| {
        let comp = app.session.active_comp_id().unwrap();
        let mut renderer = Renderer::new(&app.session.project, app.session.footage.as_ref(), RenderOpts::default());
        renderer.cache = Some(&cache);
        let image = renderer.comp_frame(comp, Tick::from_seconds_f64(0.0));
        image.data[(image.height / 2 * image.width + image.width / 2) as usize][0]
    };
    assert_eq!(red(&app), 0.0);
    write(&mut app, json!({"place": {"layer": layer}, "key": "tint", "value": "3"})).unwrap();
    assert!((red(&app) - 51.0 / 255.0).abs() < 1e-3, "tint 51 gives red {}", red(&app));
    write(&mut app, json!({"place": {"layer": layer}, "key": "tint", "value": "f"})).unwrap();
    assert!((red(&app) - 102.0 / 255.0).abs() < 1e-3, "tint 102 gives red {}", red(&app));
}

#[test]
fn data_past_the_limits_or_without_the_write_permission_is_refused_with_a_message() {
    let mut app = app("test.data-limits", &["document.read", "document.write"]);
    let layer = plate(&mut app);
    let big = write(&mut app, json!({"place": {"layer": layer}, "key": "big", "size": 256 * 1024 + 1})).unwrap_err();
    assert!(big.contains("one data value holds at most 256 KiB"), "{big}");
    for n in 0..15 {
        let place = if n % 2 == 0 { json!("project") } else { json!({"layer": layer}) };
        write(&mut app, json!({"place": place, "key": format!("part{n}"), "size": 256 * 1024})).unwrap();
    }
    let full = write(&mut app, json!({"place": "project", "key": "part15", "size": 256 * 1024})).unwrap_err();
    assert!(full.contains("a plugin keeps at most 4 MiB in one project"), "{full}");
    let missing = write(&mut app, json!({"place": {"layer": 999_999}, "key": "k", "value": "v"})).unwrap_err();
    assert!(missing.contains("no layer 999999"), "{missing}");

    let mut reader = crate::app("test.data-reader", &["document.read"]);
    let denied = write(&mut reader, json!({"place": "project", "key": "k", "value": "v"})).unwrap_err();
    assert!(denied.contains("document.write"), "{denied}");
}
