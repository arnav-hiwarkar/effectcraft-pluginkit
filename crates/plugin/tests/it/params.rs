use std::collections::BTreeSet;

use effectcraft_engine::render::{EvalCtx, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use serde_json::{Value, json};

use crate::{App, app};

fn render_at(app: &App, time: f64) {
    let session = &app.session;
    let comp = session.active_comp_id().expect("an active comp");
    let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts::default());
    renderer.expr = session.expr.as_deref();
    renderer.comp_frame(comp, Tick::from_seconds_f64(time));
}

fn reports(app: &App) -> Vec<Value> {
    let data = app.plugins.get(&app.id).expect("the plugin runs").data_dir.clone();
    let mut found: Vec<Value> = std::fs::read_dir(&data)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("report-"))
                .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|report| report["instance"].as_str().unwrap_or_default().to_string());
    found
}

fn clear_reports(app: &App) {
    let data = app.plugins.get(&app.id).expect("the plugin runs").data_dir.clone();
    for entry in std::fs::read_dir(&data).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with("report-") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
}

fn layer_key(app: &App, name: &str, time: f64) -> Option<u64> {
    let session = &app.session;
    let comp = session.active_comp().expect("an active comp");
    let ctx = EvalCtx::new(&session.project, session.active_comp_id().unwrap(), comp, Tick::from_seconds_f64(time));
    let layer = comp.layers.iter().find(|layer| layer.name == name).expect("the layer");
    effectcraft_engine::render::cache::layer_key(&ctx, layer, 1.0, false, false)
}

fn report_layer(app: &mut App, name: &str) {
    app.session.execute("layer.newSolid", json!({"name": name, "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": name, "effect": format!("{}.report", app.id)})).unwrap();
}

#[test]
fn an_effect_reads_a_keyframed_parameter_at_an_earlier_time_and_its_cache_key_follows_every_keyframe() {
    let mut app = app("test.params-history", &[]);
    report_layer(&mut app, "Sim");
    for (time, value) in [(0.0, 10.0), (1.0, 20.0)] {
        app.session.execute_checked("prop.addKey", json!({"layer": "Sim", "path": "effects/#1/level", "time": time, "value": value})).unwrap();
    }
    render_at(&app, 1.0);
    let report = &reports(&app)[0];
    assert_eq!((report["now"].as_f64(), report["at_zero"].as_f64(), report["at_zero_time"].as_f64()), (Some(20.0), Some(10.0), Some(0.0)));
    assert!(report["history"].is_string(), "a keyframed parameter without expressions has a history hash: {report}");

    let before = layer_key(&app, "Sim", 1.0).expect("the layer is cacheable");
    let history_before = report["history"].clone();
    app.session.execute_checked("prop.addKey", json!({"layer": "Sim", "path": "effects/#1/level", "time": 0.0, "value": 12.0})).unwrap();
    assert_ne!(layer_key(&app, "Sim", 1.0), Some(before), "a keyframe at another time changes the key of this time");
    clear_reports(&app);
    render_at(&app, 1.0);
    let report = &reports(&app)[0];
    assert_eq!(report["at_zero"].as_f64(), Some(12.0));
    assert_ne!(report["history"], history_before);

    app.session.execute_checked("prop.setExpression", json!({"layer": "Sim", "path": "effects/#1/level", "expression": "time * 3"})).unwrap();
    assert_eq!(layer_key(&app, "Sim", 1.0), None, "an expression's history is not in the key, so the layer is not cached");
}

#[test]
fn every_applied_effect_has_its_own_instance_that_survives_saving_and_reopening() {
    let mut app = app("test.params-instance", &[]);
    report_layer(&mut app, "One");
    report_layer(&mut app, "Two");
    let comp = app.session.active_comp_id().unwrap();
    let layers = &mut std::sync::Arc::make_mut(&mut app.session.project).comp_mut(comp).unwrap().layers;
    let shared_uid = layers[0].effects().unwrap().groups().next().unwrap().uid;
    let other = layers[1].props.sub_mut("effects").unwrap().children[0].as_group_mut().unwrap();
    other.uid = shared_uid;
    render_at(&app, 0.0);
    let instances = |app: &App| -> BTreeSet<String> { reports(app).iter().map(|r| r["instance"].as_str().unwrap().to_string()).collect() };
    let before = instances(&app);
    assert_eq!(before.len(), 2, "two layers with the same effect and the same seed have two instances: {before:?}");
    assert!(!before.contains("0"));

    let saved = tempfile::tempdir().unwrap();
    let path = saved.path().join("instances.ecproj");
    app.session.execute_checked("file.saveAs", json!({"path": path})).unwrap();
    app.session.execute_checked("file.open", json!({"path": path})).unwrap();
    clear_reports(&app);
    render_at(&app, 0.0);
    assert_eq!(instances(&app), before);
}

#[test]
fn choosing_a_preset_sets_its_parameters_in_one_undoable_step() {
    let mut app = app("test.params-preset", &[]);
    report_layer(&mut app, "Sim");
    let listed = app.session.execute_checked("effect.presets", json!({"layer": "Sim", "effect": 1})).unwrap();
    assert_eq!(listed, json!({"presets": ["Loud"]}));

    let value = |app: &mut App, path: &str| app.session.execute_checked("prop.get", json!({"layer": "Sim", "path": path})).unwrap()["value"].clone();
    app.session.execute_checked("effect.applyPreset", json!({"layer": "Sim", "effect": 1, "preset": "Loud"})).unwrap();
    assert_eq!((value(&mut app, "effects/#1/level"), value(&mut app, "effects/#1/spot")), (json!(90.0), json!([16.0, 27.0])));

    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!((value(&mut app, "effects/#1/level"), value(&mut app, "effects/#1/spot")), (json!(10.0), json!([32.0, 18.0])));
    assert!(app.session.execute_checked("effect.applyPreset", json!({"layer": "Sim", "effect": 1, "preset": "Quiet"})).is_err());
}

#[test]
fn a_slider_shows_its_soft_range_in_log_scale_and_a_percent_takes_a_range() {
    let mut app = app("test.params-ranges", &[]);
    let listed = app.session.execute_checked("effect.list", json!({"filter": "test.params-ranges.ranges"})).unwrap();
    let params = &listed[0]["params"];
    let param = |id: &str| params.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap().clone();
    let gain = param("gain");
    assert_eq!(
        (&gain["min"], &gain["max"], &gain["sliderMin"], &gain["sliderMax"], &gain["log"]),
        (&json!(0.0), &json!(1000.0), &json!(0.01), &json!(10.0), &json!(true))
    );
    let mix = param("mix");
    assert_eq!(
        (&mix["min"], &mix["max"], &mix["sliderMin"], &mix["sliderMax"], &mix["percent"]),
        (&json!(-100.0), &json!(300.0), &json!(-100.0), &json!(300.0), &json!(true))
    );
    let plain = param("plain");
    assert_eq!((&plain["sliderMax"], plain.get("log"), plain.get("percent")), (&json!(20.0), None, None));
}

#[test]
fn a_value_an_expression_pushes_outside_the_hard_range_reaches_the_plugin_inside_it() {
    let mut app = app("test.params-clamp", &[]);
    report_layer(&mut app, "Sim");
    app.session.execute_checked("prop.setExpression", json!({"layer": "Sim", "path": "effects/#1/level", "expression": "150"})).unwrap();
    render_at(&app, 0.0);
    assert_eq!(reports(&app)[0]["now"].as_f64(), Some(100.0));
    app.session.execute_checked("prop.setExpression", json!({"layer": "Sim", "path": "effects/#1/level", "expression": "-20"})).unwrap();
    clear_reports(&app);
    render_at(&app, 0.0);
    assert_eq!(reports(&app)[0]["now"].as_f64(), Some(0.0));
}

#[test]
fn a_group_named_with_spaces_shows_its_name_and_a_project_saved_under_the_old_id_keeps_its_value() {
    use effectcraft_engine::project::{LayerId, Node};
    let mut app = app("test.params-groups", &[]);
    let solid =
        app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap()["layer"].as_u64().unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.params-groups.tuned"})).unwrap();
    let detail = json!({"layer": solid, "path": "effects/#1/fine_tuning/detail"});
    assert_eq!(app.session.execute("prop.get", detail.clone()).unwrap()["value"], json!(2.0));
    let comp = app.session.active_comp_id().unwrap();
    let instance = |app: &mut App| -> effectcraft_engine::project::PropGroup {
        let layer = std::sync::Arc::make_mut(&mut app.session.project).comp_mut(comp).unwrap().layer_mut(LayerId(solid)).unwrap();
        let Some(Node::Group(instance)) = layer.props.sub_mut("effects").unwrap().children.first_mut() else { panic!("no instance") };
        instance.clone()
    };
    let group = instance(&mut app).sub("fine_tuning").map(|group| group.name.clone());
    assert_eq!(group.as_deref(), Some("Fine Tuning"));

    app.session.execute("prop.set", json!({"layer": solid, "path": "effects/#1/fine_tuning/detail", "value": 7.0})).unwrap();
    let layer = std::sync::Arc::make_mut(&mut app.session.project).comp_mut(comp).unwrap().layer_mut(LayerId(solid)).unwrap();
    let Some(Node::Group(saved)) = layer.props.sub_mut("effects").unwrap().children.first_mut() else { panic!("no instance") };
    saved.sub_mut("fine_tuning").unwrap().match_id = "Fine Tuning".into();
    assert!(app.session.execute("prop.get", detail.clone()).is_err());

    app.session.execute("plugin.load", json!({"path": app.bundle.path()})).unwrap();
    assert_eq!(app.session.execute("prop.get", detail).unwrap()["value"], json!(7.0));
    assert!(instance(&mut app).sub("Fine Tuning").is_none());
}
