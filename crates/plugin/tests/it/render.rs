use effectcraft_engine::render::{RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use serde_json::json;

use crate::{App, app};

fn centre_pixel(app: &App) -> [f32; 4] {
    let session = &app.session;
    let comp = session.active_comp_id().expect("an active comp");
    let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts::default());
    renderer.expr = session.expr.as_deref();
    let image = renderer.comp_frame(comp, Tick::from_seconds_f64(0.0));
    image.data[(image.height / 2 * image.width + image.width / 2) as usize]
}

fn close(found: [f32; 4], wanted: [f32; 4]) -> bool {
    found.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.02)
}

#[test]
fn a_plugin_effect_applied_to_a_layer_changes_the_rendered_composition() {
    let mut app = app("test.render-invert", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "the plain solid is {:?}", centre_pixel(&app));

    app.session.execute("effect.apply", json!({"effect": "test.render-invert.invert"})).unwrap();
    assert!(close(centre_pixel(&app), [0.0, 0.8, 1.0, 1.0]), "inverted it is {:?}", centre_pixel(&app));
}

#[test]
fn a_plugin_effect_parameter_is_a_property_that_can_be_set_and_undone() {
    let mut app = app("test.render-param", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-param.invert"})).unwrap();

    app.session.execute("prop.set", json!({"path": "effects/#1/amount", "value": 0.0})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "amount 0 gives {:?}", centre_pixel(&app));

    app.session.execute("edit.undo", json!({})).unwrap();
    assert!(close(centre_pixel(&app), [0.0, 0.8, 1.0, 1.0]), "after undo it is {:?}", centre_pixel(&app));
}

#[test]
fn stopping_a_plugin_removes_its_effects_and_the_layer_renders_unchanged() {
    let mut app = app("test.render-stop", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-stop.invert"})).unwrap();
    assert!(effectcraft_engine::effects::find("test.render-stop.invert").is_some());

    std::fs::remove_file(app.bundle.path().join("probe.wasm")).unwrap();
    assert!(app.plugins.load_for_session(app.bundle.path()).is_err(), "a bundle without its module fails to load");
    assert!(effectcraft_engine::effects::find("test.render-stop.invert").is_none());
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "without the plugin it is {:?}", centre_pixel(&app));
}

#[test]
fn reloading_a_plugin_with_new_code_changes_the_effect_revision_so_cached_frames_redraw() {
    let app = app("test.render-revision", &[]);
    let effect = "test.render-revision.invert";
    let first = effectcraft_engine::effects::revision(effect);
    assert_ne!(first, 0);

    app.plugins.load_for_session(app.bundle.path()).unwrap();
    assert_eq!(effectcraft_engine::effects::revision(effect), first, "the same code keeps its revision");

    let module = app.bundle.path().join("probe.wasm");
    let mut bytes = std::fs::read(&module).unwrap();
    bytes.extend_from_slice(&[0, 4, 1, b'x', 7, 7]);
    std::fs::write(&module, bytes).unwrap();
    app.plugins.load_for_session(app.bundle.path()).unwrap();
    assert_ne!(effectcraft_engine::effects::revision(effect), first, "new code gets a new revision");
}

fn reader_key(app: &App) -> Option<u64> {
    let session = &app.session;
    let comp_id = session.active_comp_id().expect("an active comp");
    let comp = session.active_comp().expect("an active comp");
    let ctx = effectcraft_engine::render::EvalCtx::new(&session.project, comp_id, comp, Tick::from_seconds_f64(0.0));
    let reader = comp.layers.iter().find(|layer| layer.name == "Reader").expect("the reader layer");
    effectcraft_engine::render::cache::layer_key(&ctx, reader, 1.0, false, false)
}

#[test]
fn a_layer_that_samples_a_plugin_layer_changes_its_cache_key_when_the_plugin_changes() {
    let mut app = app("test.render-dependent", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Source", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Source", "effect": "test.render-dependent.invert"})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Reader", "color": "#00ff00", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Reader", "effect": "ec.channel.setmatte"})).unwrap();
    let source = app.session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Source").unwrap().id.0;
    app.session.execute_checked("prop.set", json!({"layer": "Reader", "path": "effects/#1/takeMatteFromLayer", "value": source})).unwrap();
    let before = reader_key(&app).expect("the reader is cacheable");
    let comp = app.session.active_comp_id().unwrap();
    let content = |app: &App| effectcraft_engine::render::disk_cache::comp_content_key(&app.session.project, comp);
    let content_before = content(&app);

    let manifest = app.bundle.path().join("plugin.json");
    let changed = std::fs::read_to_string(&manifest).unwrap().replace("\"1.0.0\"", "\"1.0.1\"");
    std::fs::write(&manifest, changed).unwrap();
    app.plugins.load_for_session(app.bundle.path()).expect("the changed plugin loads");

    assert_ne!(reader_key(&app).expect("the reader is cacheable"), before);
    assert_ne!(content(&app), content_before, "the comp's thumbnail and disk frames are keyed on the plugin too");
}

#[test]
fn a_plugin_effect_changes_its_cache_key_when_a_file_in_its_data_folder_changes() {
    let mut app = app("test.render-data", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Reader", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Reader", "effect": "test.render-data.invert"})).unwrap();
    let before = reader_key(&app).expect("the layer is cacheable");
    assert_eq!(reader_key(&app), Some(before), "the key is stable while nothing changes");

    let data = app.plugins.get("test.render-data").expect("the plugin runs").data_dir.clone();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("palette.json"), "[1, 2, 3]").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_ne!(reader_key(&app).expect("the layer is cacheable"), before);
}
