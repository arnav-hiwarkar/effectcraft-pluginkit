use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_plugin::ui::Shape;
use egui::{Pos2, Vec2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::EffectcraftApp;

fn wasm(crate_dir: &Path, name: &str) -> PathBuf {
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(crate_dir.join("Cargo.toml"))
        .status()
        .expect("cargo runs");
    assert!(status.success(), "{name} builds for WebAssembly");
    crate_dir.join(format!("target/wasm32-unknown-unknown/release/{name}.wasm"))
}

fn bundle(wasm: &Path, manifest: Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::copy(wasm, dir.path().join("plugin.wasm")).expect("copy the module");
    std::fs::write(dir.path().join("plugin.json"), manifest.to_string()).expect("write the manifest");
    dir
}

fn session() -> Session {
    static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("temp dir"));
    let mut session = effectcraft_host::session();
    effectcraft_plugin::install(&mut session, Some(config.path()));
    session.execute("comp.new", json!({"name": "Viewer", "width": 320, "height": 180, "frameRate": 24, "duration": 2})).unwrap();
    session
}

fn load(slot: &'static OnceLock<tempfile::TempDir>, make: impl FnOnce() -> tempfile::TempDir) {
    let dir = slot.get_or_init(make);
    static LOADED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    let mut loaded = LOADED.lock().unwrap();
    if !loaded.contains(&dir.path().to_path_buf()) {
        effectcraft_plugin::plugins().expect("plugins are installed").load_for_session(dir.path()).expect("the plugin loads");
        loaded.push(dir.path().to_path_buf());
    }
}

fn halo_session() -> Session {
    static HALO: OnceLock<tempfile::TempDir> = OnceLock::new();
    let mut session = session();
    load(&HALO, || {
        let halo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/halo");
        let manifest: Value = serde_json::from_slice(&std::fs::read(halo.join("plugin.json")).unwrap()).unwrap();
        bundle(&wasm(&halo, "halo"), manifest)
    });
    session.execute("layer.newSolid", json!({"name": "Sky", "color": "#000000", "width": 320, "height": 180})).unwrap();
    session.execute("effect.apply", json!({"layer": "Sky", "effect": "org.craftkit.halo.flare"})).unwrap();
    session
}

fn harness(session: Session) -> Harness<'static, EffectcraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(|_| EffectcraftApp::new(session))
}

fn element(harness: &Harness<'_, EffectcraftApp>, id: &str) -> egui::Rect {
    let found = harness.state().auto.find(id).unwrap_or_else(|| panic!("`{id}` is on screen"));
    egui::Rect::from_min_size(pos2(found.rect[0], found.rect[1]), egui::vec2(found.rect[2], found.rect[3]))
}

fn press_drag_release(harness: &mut Harness<'_, EffectcraftApp>, path: &[Pos2]) {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let (first, last) = (path[0], path[path.len() - 1]);
    let mut events = vec![egui::Event::PointerMoved(first), button(first, true)];
    events.extend(path[1..].iter().map(|at| egui::Event::PointerMoved(*at)));
    events.push(button(last, false));
    for event in events {
        harness.input_mut().events.push(event);
        settle(harness, |h| h.state().plugin_ui.panels.idle());
    }
}

fn shown_frame(harness: &mut Harness<'_, EffectcraftApp>) -> Option<Vec<u8>> {
    harness.state_mut().viewer_pixels().map(|image| image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect())
}

/// The first call into a plugin compiles its module on a worker thread, which takes seconds on a
/// loaded machine, so the wait is a wall-clock deadline, not a count of steps.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(60);

fn settle(harness: &mut Harness<'_, EffectcraftApp>, mut done: impl FnMut(&mut Harness<'_, EffectcraftApp>) -> bool) {
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        harness.step();
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the viewer did not settle");
}

fn stable_frame(harness: &mut Harness<'_, EffectcraftApp>) -> Option<Vec<u8>> {
    let mut last = shown_frame(harness);
    let mut same = 0;
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let now = shown_frame(harness);
        same = if now == last && now.is_some() { same + 1 } else { 0 };
        if same == 20 {
            return now;
        }
        last = now;
    }
    panic!("the viewer frame did not settle");
}

fn comp_point(harness: &Harness<'_, EffectcraftApp>, comp: [f32; 2]) -> Pos2 {
    crate::panels::viewer::comp_to_screen(&harness.ctx, comp).expect("the viewer published its mapping")
}

#[test]
fn dragging_the_halo_centre_handle_moves_the_light_in_one_undo_step_and_renders_a_new_frame() {
    let mut session = halo_session();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    session.state.selected_props = vec![(LayerId(layer.id.0), effect)];
    let mut harness = harness(session);
    harness.state_mut().ui.viewer.zoom = Some(1.0);
    settle(&mut harness, |h| shown_frame(h).is_some() && h.state().auto.find(&format!("viewer.pluginHandle.{effect}.0")).is_some());
    let before = stable_frame(&mut harness);
    let centre = element(&harness, &format!("viewer.pluginHandle.{effect}.0")).center();
    assert!((centre - comp_point(&harness, [96.0, 54.0])).length() < 0.5, "the centre handle sits on Light Position");
    assert!(
        harness.state().auto.find(&format!("viewer.effectPoint.{}", layer.effects().unwrap().groups().next().unwrap().get("light").unwrap().uid)).is_none()
    );

    press_drag_release(&mut harness, &[centre, centre + Vec2::new(20.0, 10.0), centre + Vec2::new(40.0, 20.0)]);
    let light = |h: &mut Harness<'_, EffectcraftApp>| {
        h.state_mut().session.execute("prop.get", json!({"layer": "Sky", "path": "effects/#1/light"})).unwrap()["value"].clone()
    };
    assert_eq!(light(&mut harness), json!([136.0, 74.0]));
    settle(&mut harness, |h| shown_frame(h).is_some() && shown_frame(h) != before);

    harness.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(light(&mut harness), json!([96.0, 54.0]), "the whole drag is one undo step");
}

#[test]
fn undo_reaches_the_app_while_a_dragged_handle_keeps_keyboard_focus() {
    let mut session = halo_session();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    session.state.selected_props = vec![(LayerId(layer.id.0), effect)];
    let mut harness = harness(session);
    harness.state_mut().ui.viewer.zoom = Some(1.0);
    settle(&mut harness, |h| shown_frame(h).is_some() && h.state().auto.find(&format!("viewer.pluginHandle.{effect}.0")).is_some());
    stable_frame(&mut harness);
    let centre = element(&harness, &format!("viewer.pluginHandle.{effect}.0")).center();
    press_drag_release(&mut harness, &[centre, centre + Vec2::new(20.0, 10.0), centre + Vec2::new(40.0, 20.0)]);
    let light = |h: &mut Harness<'_, EffectcraftApp>| {
        h.state_mut().session.execute("prop.get", json!({"layer": "Sky", "path": "effects/#1/light"})).unwrap()["value"].clone()
    };
    assert_eq!(light(&mut harness), json!([136.0, 74.0]));
    assert!(harness.ctx.memory(|m| m.focused()).is_some(), "the handle keeps keyboard focus after the drag");

    let z = |pressed| egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND };
    harness.input_mut().events.push(z(true));
    harness.step();
    harness.input_mut().events.push(z(false));
    harness.step();
    assert_eq!(light(&mut harness), json!([96.0, 54.0]), "Cmd+Z undoes the drag without Escape first");
}

#[test]
fn the_ring_and_point_handles_take_tab_focus_and_arrow_keys() {
    let mut session = halo_session();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    session.state.selected_props = vec![(LayerId(layer.id.0), effect)];
    let mut harness = harness(session);
    settle(&mut harness, |h| h.state().auto.find(&format!("viewer.pluginHandle.{effect}.0")).is_some());
    let centre = element(&harness, &format!("viewer.pluginHandle.{effect}.0")).center();
    press_drag_release(&mut harness, &[centre]);
    harness.step();
    let key = |h: &mut Harness<'_, EffectcraftApp>, key: egui::Key, modifiers: egui::Modifiers| {
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        h.step();
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
        h.step();
    };
    key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    let value = |h: &mut Harness<'_, EffectcraftApp>, path: &str| {
        h.state_mut().session.execute("prop.get", json!({"layer": "Sky", "path": path})).unwrap()["value"].clone()
    };
    assert_eq!(value(&mut harness, "effects/#1/light"), json!([97.0, 64.0]));
    key(&mut harness, egui::Key::Tab, egui::Modifiers::NONE);
    key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    assert_eq!(value(&mut harness, "effects/#1/light"), json!([97.0, 64.0]), "Tab moved the focus to the next handle");
    let size = value(&mut harness, "effects/#1/Glow/size").as_f64().unwrap();
    assert!(size > 100.0, "the arrow key moved the ring handle outwards: {size}");
}

#[test]
fn the_handles_of_an_effect_on_a_moved_adjustment_layer_sit_where_the_effect_draws_in_comp_space() {
    let mut session = halo_session();
    session.execute("layer.newAdjustment", json!({"name": "Adjust"})).unwrap();
    session.execute("effect.apply", json!({"layer": "Adjust", "effect": "org.craftkit.halo.flare"})).unwrap();
    session.execute("prop.set", json!({"layer": "Adjust", "path": "transform/position", "value": [200.0, 110.0]})).unwrap();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    session.state.selected_props = vec![(LayerId(layer.id.0), effect)];
    let mut harness = harness(session);
    harness.state_mut().ui.viewer.zoom = Some(1.0);
    settle(&mut harness, |h| shown_frame(h).is_some() && h.state().auto.find(&format!("viewer.pluginHandle.{effect}.0")).is_some());
    stable_frame(&mut harness);
    let centre = element(&harness, &format!("viewer.pluginHandle.{effect}.0")).center();
    let at = harness.state_mut().session.execute("prop.get", json!({"layer": "Adjust", "path": "effects/#1/light"})).unwrap()["value"].clone();
    let at: [f32; 2] = serde_json::from_value(at).unwrap();
    let light = comp_point(&harness, at);
    assert!((centre - light).length() < 0.5, "the centre handle sits on Light Position {at:?} in comp pixels, {light:?}, not at {centre:?}");
}

fn tool_texts(harness: &mut Harness<'_, EffectcraftApp>) -> Vec<String> {
    let app = harness.state_mut();
    let overlay = app.plugin_ui.panels.tool_overlay(&mut app.session).cloned().unwrap_or_default();
    overlay
        .marks
        .into_iter()
        .filter_map(|mark| match mark.shape {
            Shape::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

static STAMP_TOOL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn probe_session() -> Session {
    static PROBE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let session = session();
    load(&PROBE, || {
        let probe = craft_plugin_host::testing::probe_dir();
        let manifest = json!({"id": "test.viewer-tool-ui", "name": "Probe", "version": "1.0.0", "entry": {"wasm": "plugin.wasm"}, "permissions": ["document.read", "document.write"]});
        bundle(&wasm(probe, "probe"), manifest)
    });
    session
}

#[test]
fn a_plugin_tool_receives_its_gesture_in_comp_coordinates_at_half_and_double_zoom() {
    let _one_at_a_time = STAMP_TOOL.lock().unwrap_or_else(|e| e.into_inner());
    let mut harness = harness(probe_session());
    settle(&mut harness, |h| h.state().auto.find("tools.plugin.test.viewer-tool-ui.stamp").is_some());
    let button = element(&harness, "tools.plugin.test.viewer-tool-ui.stamp").center();
    press_drag_release(&mut harness, &[button]);
    assert!(super::active(harness.state()).is_some_and(|pick| pick.tool == "stamp"), "a click on its toolbar button picks the tool");
    for zoom in [0.5, 2.0] {
        harness.state_mut().ui.viewer.zoom = Some(zoom);
        harness.step();
        harness.step();
        let path = [comp_point(&harness, [40.0, 30.0]), comp_point(&harness, [50.0, 35.0]), comp_point(&harness, [60.0, 45.0])];
        press_drag_release(&mut harness, &path);
        settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
        assert_eq!(tool_texts(&mut harness), ["Down 40.0 30.0", "Move 50.0 35.0", "Move 60.0 45.0", "Up 60.0 45.0"], "at {zoom}x");
    }
    harness.state_mut().session.execute("edit.undo", json!({})).unwrap();
    let names: Vec<String> = harness.state().session.active_comp().unwrap().layers.iter().map(|layer| layer.name.clone()).collect();
    assert_eq!(names, ["Stamp"], "each gesture is one undo step");
}

#[test]
fn a_plugin_menu_command_is_queued_and_the_menu_returns_before_it_runs() {
    let mut harness = harness(probe_session());
    harness.step();
    let ctx = harness.ctx.clone();
    let app = harness.state_mut();
    let ran = crate::menus::invoke(app, &ctx, "plugin.run", json!({"plugin": "test.viewer-tool-ui", "command": "edit"}));
    let names =
        |h: &Harness<'_, EffectcraftApp>| -> Vec<String> { h.state().session.active_comp().unwrap().layers.iter().map(|layer| layer.name.clone()).collect() };
    let before = names(&harness);
    settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
    assert_eq!((ran, before, names(&harness)), (Ok(Value::Null), vec![], vec!["From Plugin".to_string()]));
}

#[test]
fn a_handle_drag_whose_last_move_arrives_with_the_release_ends_where_the_pointer_was_released() {
    let mut session = halo_session();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    session.state.selected_props = vec![(LayerId(layer.id.0), effect)];
    let mut harness = harness(session);
    harness.state_mut().ui.viewer.zoom = Some(1.0);
    settle(&mut harness, |h| shown_frame(h).is_some() && h.state().auto.find(&format!("viewer.pluginHandle.{effect}.0")).is_some());
    stable_frame(&mut harness);
    let centre = element(&harness, &format!("viewer.pluginHandle.{effect}.0")).center();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let (middle, end) = (centre + Vec2::new(20.0, 10.0), centre + Vec2::new(40.0, 20.0));
    let frames = [
        vec![egui::Event::PointerMoved(centre)],
        vec![button(centre, true)],
        vec![egui::Event::PointerMoved(middle)],
        vec![egui::Event::PointerMoved(end), button(end, false)],
    ];
    for events in frames {
        harness.input_mut().events.extend(events);
        settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
    }
    let light = harness.state_mut().session.execute("prop.get", json!({"layer": "Sky", "path": "effects/#1/light"})).unwrap()["value"].clone();
    assert_eq!(light, json!([136.0, 74.0]));
}

#[test]
fn clicking_a_plugin_gradient_strip_in_effect_controls_opens_the_gradient_editor_on_it() {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Ramp", "color": "#000000", "width": 320, "height": 180})).unwrap();
    session.execute("effect.apply", json!({"layer": "Ramp", "effect": "test.viewer-tool-ui.ramp"})).unwrap();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let ramp = layer.effects().unwrap().groups().next().unwrap().get("ramp").unwrap().uid;
    session.state.selected_layers = vec![layer.id];
    let mut harness = harness(session);
    harness.state_mut().show_panel(crate::dock::PanelKind::EffectControls);
    settle(&mut harness, |h| h.state().auto.find(&format!("effectControls.prop.{ramp}.value")).is_some());
    let strip = element(&harness, &format!("effectControls.prop.{ramp}.value")).center();
    press_drag_release(&mut harness, &[strip]);
    harness.run_steps(2);
    assert_eq!(harness.state().dialog, Some(crate::Dialog::GradientEditor));
    let target = harness.state().gradient_draft().target.as_ref().expect("the editor edits a property");
    assert_eq!((target.layer, target.prop), (layer.id.0, ramp));
}

fn control(harness: &mut Harness<'_, EffectcraftApp>, method: &str, params: Value) -> crate::control::Outcome {
    let ctx = harness.ctx.clone();
    let (request, _reply) = crate::control::ControlRequest::new(method, params);
    crate::control::handle(harness.state_mut(), &ctx, &request, 0.0)
}

fn failure(outcome: crate::control::Outcome) -> String {
    match outcome {
        crate::control::Outcome::Done(value) => value["error"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

#[test]
fn ui_set_and_the_tool_command_pick_a_plugin_tool_and_a_drag_through_the_control_path_reaches_it() {
    let _one_at_a_time = STAMP_TOOL.lock().unwrap_or_else(|e| e.into_inner());
    let mut harness = harness(probe_session());
    harness.run_steps(2);
    let missing = failure(control(&mut harness, "ui.set", json!({"tool": "test.viewer-tool-ui/nope"})));
    assert!(missing.contains("unknown plugin tool `test.viewer-tool-ui/nope`") && missing.contains("test.viewer-tool-ui/stamp"), "{missing}");
    assert!(super::active(harness.state()).is_none());

    control(&mut harness, "ui.set", json!({"tool": "test.viewer-tool-ui/stamp"}));
    assert!(super::active(harness.state()).is_some_and(|pick| pick.tool == "stamp"), "ui.set picks the plugin tool");
    control(&mut harness, "ui.set", json!({"tool": "hand"}));
    assert!(super::active(harness.state()).is_none(), "a built-in tool puts the plugin tool away");
    control(&mut harness, "engine.execute", json!({"command": "tool.test.viewer-tool-ui/stamp"}));
    assert!(super::active(harness.state()).is_some_and(|pick| pick.tool == "stamp"), "the tool command picks it too");

    harness.state_mut().ui.viewer.zoom = Some(1.0);
    harness.run_steps(2);
    let (a, b) = (comp_point(&harness, [40.0, 30.0]), comp_point(&harness, [60.0, 45.0]));
    control(&mut harness, "ui.drag", json!({"from": {"x": a.x, "y": a.y}, "to": {"x": b.x, "y": b.y}, "steps": 2}));
    let queued = std::mem::take(&mut harness.state_mut().synthetic);
    for event in queued {
        harness.input_mut().events.push(event);
        settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
    }
    let texts = tool_texts(&mut harness);
    assert!(texts.first().is_some_and(|t| t.starts_with("Down 40.0 30.0")) && texts.last().is_some_and(|t| t.starts_with("Up 60.0 45.0")), "{texts:?}");
}

fn lookout_harness() -> (Harness<'static, EffectcraftApp>, LayerId) {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Watched", "color": "#000000", "width": 320, "height": 180})).unwrap();
    session.execute("effect.apply", json!({"layer": "Watched", "effect": "test.viewer-tool-ui.lookout"})).unwrap();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap();
    let rows = ["selection", "ticks"].map(|param| format!("effectControls.prop.{}.control", effect.get(param).unwrap().uid));
    session.state.selected_layers = vec![layer.id];
    let mut harness = harness(session);
    harness.state_mut().show_panel(crate::dock::PanelKind::EffectControls);
    settle(&mut harness, |h| rows.iter().all(|row| h.state().plugin_ui.panels.calls(row) > 0) && h.state().plugin_ui.panels.idle());
    (harness, layer.id)
}

#[test]
fn a_plugin_control_row_subscribed_to_the_selection_draws_again_when_the_selection_changes() {
    let (mut harness, layer) = lookout_harness();
    assert!(!harness.state().session.state.selected_props.is_empty(), "applying the effect selected it");
    harness.state_mut().session.state.selected_props.clear();
    let shown = format!("selected [{}]", layer.0);
    settle(&mut harness, |h| h.query_all_by_value(&shown).next().is_some());
}

#[test]
fn a_plugin_control_row_with_a_timer_draws_again_without_any_input() {
    let (mut harness, _) = lookout_harness();
    settle(&mut harness, |h| h.query_all_by_value("ticks 3").next().is_some());
}

#[test]
fn a_control_row_of_a_plugin_built_against_the_sdk_before_row_events_still_draws_in_effect_controls() {
    static CURVES: OnceLock<tempfile::TempDir> = OnceLock::new();
    let mut session = session();
    load(&CURVES, || {
        let curves = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/curves");
        let manifest: Value = serde_json::from_slice(&std::fs::read(curves.join("plugin.json")).unwrap()).unwrap();
        bundle(&wasm(&curves, "curves"), manifest)
    });
    session.execute("layer.newSolid", json!({"name": "Graded", "color": "#000000", "width": 320, "height": 180})).unwrap();
    session.execute("effect.apply", json!({"layer": "Graded", "effect": "org.craftkit.curves.curves"})).unwrap();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let row = format!("effectControls.prop.{}.control", layer.effects().unwrap().groups().next().unwrap().get("curves").unwrap().uid);
    session.state.selected_layers = vec![layer.id];
    let mut harness = harness(session);
    harness.state_mut().show_panel(crate::dock::PanelKind::EffectControls);
    settle(&mut harness, |h| h.state().plugin_ui.panels.calls(&row) > 0 && h.state().plugin_ui.panels.idle());
    harness.run_steps(2);
    let hint = "Click to add a point, drag to move it, drag it off or Alt-click to remove it. Arrow keys move the chosen point, Delete removes it.";
    assert_eq!(harness.query_all_by_value(hint).count(), 1, "the row draws the plugin's own hint");
}

static PROBE_FORM: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn asked_title(harness: &Harness<'_, EffectcraftApp>) -> Option<String> {
    effectcraft_plugin::asked_form(&harness.state().session, |asked| asked.form.title.clone())
}

fn answered(harness: &mut Harness<'_, EffectcraftApp>) -> String {
    settle(harness, |h| asked_title(h).is_none() && h.state().plugin_ui.panels.idle());
    let data = effectcraft_plugin::plugins().unwrap().get("test.viewer-tool-ui").unwrap().data_dir.join("form.json");
    std::fs::read_to_string(data).unwrap()
}

#[test]
fn a_plugin_command_runs_from_its_free_default_shortcut_and_the_palette_but_never_takes_a_built_in_one() {
    let _one_at_a_time = PROBE_FORM.lock().unwrap_or_else(|e| e.into_inner());
    let mut session = probe_session();
    session.execute("plugin.list", json!({})).unwrap();
    let mine = session.plugin_items.iter().filter(|item| item.plugin == "test.viewer-tool-ui").cloned().collect();
    session.set_plugin_items(mine);
    let mut harness = harness(session);
    harness.step();
    let params = |id: &str| effectcraft_engine::shortcuts::plugin_command_params("test.viewer-tool-ui", id);
    let shortcuts = harness.state().session.shortcuts();
    assert_eq!(shortcuts.shortcut_of("plugin.run", &params("ask")), Some("Alt+Shift+F11"));
    assert_eq!(shortcuts.shortcut_of("plugin.run", &params("read")), None, "Cmd+Shift+J stays with View ▸ Resolution ▸ Half");
    assert_eq!(shortcuts.shortcut_of("view.res.half", &Value::Null), Some("Cmd+Shift+J"));

    harness.key_press_modifiers(egui::Modifiers { alt: true, shift: true, ..Default::default() }, egui::Key::F11);
    settle(&mut harness, |h| asked_title(h).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "plugin.answerForm", json!({"cancel": true})).unwrap();
    assert_eq!(answered(&mut harness), "cancelled");

    crate::menus::invoke(harness.state_mut(), &ctx, "app.commandPalette", json!({"query": "probe ask for settings"})).unwrap();
    harness.step();
    harness.key_press(egui::Key::Enter);
    settle(&mut harness, |h| asked_title(h).is_some());
    crate::menus::invoke(harness.state_mut(), &ctx, "plugin.answerForm", json!({"cancel": true})).unwrap();
    assert_eq!(answered(&mut harness), "cancelled");
}

#[test]
fn a_plugin_form_returns_the_values_entered_on_ok_nothing_on_cancel_and_automation_can_answer_it() {
    let _one_at_a_time = PROBE_FORM.lock().unwrap_or_else(|e| e.into_inner());
    let mut harness = harness(probe_session());
    harness.step();
    let ctx = harness.ctx.clone();
    let ask = |harness: &mut Harness<'_, EffectcraftApp>| {
        crate::menus::invoke(harness.state_mut(), &ctx, "plugin.run", json!({"plugin": "test.viewer-tool-ui", "command": "ask"})).unwrap();
        settle(harness, |h| asked_title(h).is_some() && h.state().auto.find("plugin.form.ok").is_some());
        (0..3).for_each(|_| harness.step());
    };
    let click = |harness: &mut Harness<'_, EffectcraftApp>, id: &str| {
        let at = element(harness, id).center();
        for event in [
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ] {
            harness.input_mut().events.push(event);
            harness.step();
        }
    };

    ask(&mut harness);
    click(&mut harness, "plugin.form.soft");
    click(&mut harness, "plugin.form.ok");
    let values: Value = serde_json::from_str(&answered(&mut harness)).unwrap();
    assert_eq!(values, json!({"radius": 10.0, "soft": true, "mode": 0.0, "label": "none"}));

    ask(&mut harness);
    click(&mut harness, "plugin.form.soft");
    click(&mut harness, "plugin.form.cancel");
    assert_eq!(answered(&mut harness), "cancelled");

    ask(&mut harness);
    let shown = crate::menus::invoke(harness.state_mut(), &ctx, "plugin.form", json!({})).unwrap();
    assert_eq!((shown["form"]["title"].clone(), shown["values"]["radius"].clone()), (json!("Settings"), json!(10.0)));
    let wrong = crate::menus::invoke(harness.state_mut(), &ctx, "plugin.answerForm", json!({"values": {"soft": "yes"}})).unwrap_err();
    assert!(wrong.contains("parameter `soft` takes true or false"), "{wrong}");
    crate::menus::invoke(harness.state_mut(), &ctx, "plugin.answerForm", json!({"values": {"radius": 42.0, "mode": 1.0}})).unwrap();
    let values: Value = serde_json::from_str(&answered(&mut harness)).unwrap();
    assert_eq!(values, json!({"radius": 42.0, "soft": false, "mode": 1.0, "label": "none"}));
}

#[test]
fn a_plugin_popup_shows_its_hidden_slider_and_its_button_runs_two_commands_in_one_undo_step_by_pointer_keyboard_and_automation() {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Switched", "color": "#000000", "width": 320, "height": 180})).unwrap();
    session.execute("effect.apply", json!({"layer": "Switched", "effect": "test.viewer-tool-ui.switches"})).unwrap();
    let layer = session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap();
    let [mode, softness, reset_uid, effect_amount] = ["mode", "softness", "reset", "amount"].map(|param| effect.get(param).unwrap().uid);
    session.state.selected_layers = vec![layer.id];
    let mut harness = harness(session);
    harness.state_mut().show_panel(crate::dock::PanelKind::EffectControls);
    let button = format!("effectControls.prop.{reset_uid}.button");
    settle(&mut harness, |h| h.state().auto.find(&button).is_some());
    let softness_row = format!("effectControls.row.{softness}");
    assert!(harness.state().auto.find(&softness_row).is_none(), "Softness is hidden while Mode is Plain");
    let popup = element(&harness, &format!("effectControls.prop.{mode}.value")).center();
    press_drag_release(&mut harness, &[popup]);
    harness.run_steps(2);
    harness.get_by_label("Soft").click();
    settle(&mut harness, |h| h.state().auto.find(&softness_row).is_some());
    harness.run_steps(2);

    let values = |h: &mut Harness<'_, EffectcraftApp>| {
        ["softness", "amount"].map(|param| {
            h.state_mut().session.execute("prop.get", json!({"layer": "Switched", "path": format!("effects/#1/{param}")})).unwrap()["value"].clone()
        })
    };
    let reset = |h: &mut Harness<'_, EffectcraftApp>| {
        settle(h, |h| h.state().plugin_ui.panels.idle() && values(h) == [json!(0.0), json!(0.0)]);
        h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(values(h), [json!(4.0), json!(1.0)], "one undo step takes back the whole press");
        h.run_steps(2);
    };
    let at = element(&harness, &button);
    press_drag_release(&mut harness, &[at.center()]);
    reset(&mut harness);

    let amount = element(&harness, &format!("effectControls.prop.{}.value", effect_amount)).center();
    press_drag_release(&mut harness, &[amount, amount + Vec2::new(30.0, 0.0), amount + Vec2::new(60.0, 0.0)]);
    assert_eq!(values(&mut harness)[1], json!(1.0), "Amount is turned off until Manual is on");

    let focused = harness.state().auto.find(&button).and_then(|element| element.response_id).expect("the button registers its response");
    harness.ctx.memory_mut(|memory| memory.request_focus(focused));
    harness.step();
    harness.key_press(egui::Key::Enter);
    reset(&mut harness);

    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "plugin.press", json!({"layer": layer.id.0, "prop": reset_uid})).unwrap();
    reset(&mut harness);
    harness.state_mut().session.execute("prop.set", json!({"layer": "Switched", "path": "effects/#1/mode", "value": 0})).unwrap();
    let hidden = crate::menus::invoke(harness.state_mut(), &ctx, "plugin.press", json!({"layer": layer.id.0, "prop": softness})).unwrap_err();
    assert!(hidden.contains("hidden or turned off"), "{hidden}");
}
