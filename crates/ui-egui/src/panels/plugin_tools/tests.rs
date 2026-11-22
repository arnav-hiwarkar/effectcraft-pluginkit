use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_plugin::ui::Shape;
use egui::{Pos2, Vec2, pos2};
use egui_kittest::Harness;
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

fn shown_frame(harness: &Harness<'_, EffectcraftApp>) -> Option<Vec<u8>> {
    harness.state().viewer_image.as_ref().map(|image| image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect())
}

fn settle(harness: &mut Harness<'_, EffectcraftApp>, done: impl Fn(&Harness<'_, EffectcraftApp>) -> bool) {
    for _ in 0..600 {
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
    for _ in 0..600 {
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

#[test]
fn a_plugin_tool_receives_its_gesture_in_comp_coordinates_at_half_and_double_zoom() {
    static PROBE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let session = session();
    load(&PROBE, || {
        let probe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin-host/tests/probe");
        let manifest = json!({"id": "test.viewer-tool-ui", "name": "Probe", "version": "1.0.0", "entry": {"wasm": "plugin.wasm"}, "permissions": ["document.read", "document.write"]});
        bundle(&wasm(&probe, "probe"), manifest)
    });
    let mut harness = harness(session);
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
