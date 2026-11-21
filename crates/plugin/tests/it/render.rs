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
