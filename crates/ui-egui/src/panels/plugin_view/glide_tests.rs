use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::EffectcraftApp;

const GLIDE: &str = "org.craftkit.glide";

fn plugin_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins").join(name)
}

pub(crate) fn bundle(name: &str) -> &'static Path {
    static BUNDLES: OnceLock<std::sync::Mutex<Vec<(String, &'static tempfile::TempDir)>>> = OnceLock::new();
    let bundles = BUNDLES.get_or_init(Default::default);
    let mut bundles = bundles.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, dir)) = bundles.iter().find(|(built, _)| built == name) {
        return dir.path();
    }
    let source = plugin_dir(name);
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(source.join("Cargo.toml"))
        .status()
        .expect("cargo runs");
    assert!(status.success(), "{name} builds");
    let dir: &'static tempfile::TempDir = Box::leak(Box::new(tempfile::tempdir().expect("temp dir")));
    let wasm = source.join("target/wasm32-unknown-unknown/release").join(format!("{}.wasm", name.replace('-', "_")));
    std::fs::copy(wasm, dir.path().join("plugin.wasm")).expect("copy the module");
    std::fs::copy(source.join("plugin.json"), dir.path().join("plugin.json")).expect("copy the manifest");
    bundles.push((name.to_string(), dir));
    dir.path()
}

pub(crate) fn session_with(plugins: &[&str]) -> effectcraft_engine::Session {
    static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("temp dir"));
    let mut session = effectcraft_host::session();
    let installed = effectcraft_plugin::install(&mut session, Some(config.path()));
    for name in plugins {
        installed.load_for_session(bundle(name)).expect("the plugin loads");
    }
    effectcraft_plugin::install(&mut session, Some(config.path()));
    session
}

fn step_until(harness: &mut Harness<'_, EffectcraftApp>, what: &str, done: impl Fn(&Harness<'_, EffectcraftApp>) -> bool) {
    for _ in 0..500 {
        harness.step();
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("{what} never happened; the app shows {:?}", texts(harness));
}

fn texts(harness: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    harness.query_all(egui_kittest::kittest::by()).filter_map(|node| node.accesskit_node().label().or(node.accesskit_node().value())).collect()
}

fn shows(harness: &Harness<'_, EffectcraftApp>, text: &str) -> bool {
    texts(harness).iter().any(|shown| shown == text)
}

#[test]
fn glides_panel_follows_the_keyframe_selection_and_its_auto_apply_box_works_from_the_keyboard() {
    let mut session = session_with(&["glide"]);
    session.execute("comp.new", json!({"name": "Ease", "width": 64, "height": 36, "duration": 3})).unwrap();
    session.execute("layer.newSolid", json!({"color": "#808080"})).unwrap();
    for (time, value) in [(0.0, 0.0), (2.0, 100.0)] {
        session.execute("prop.addKey", json!({"layer": 1, "path": "transform/opacity", "time": time, "value": value})).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_context_setup(crate::framework::install_headless)
        .build_eframe(|_| EffectcraftApp::new(session));
    harness.step();
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": GLIDE, "panel": "glide"})).unwrap();
    step_until(&mut harness, "the idle status", |h| shows(h, "Select keyframes, shape the curve, then Apply."));

    let keys = json!([{"layer": 1, "path": "transform/opacity", "time": 0.0}, {"layer": 1, "path": "transform/opacity", "time": 2.0}]);
    harness.state_mut().session.execute("keys.select", json!({"keys": keys})).unwrap();
    step_until(&mut harness, "the curve read from the selection", |h| shows(h, "Read cubic-bezier(0.167, 0.167, 0.833, 0.833) from Opacity"));

    let auto_apply = |h: &Harness<'_, EffectcraftApp>| h.get_by_label("Auto Apply").accesskit_node().toggled();
    assert_eq!(auto_apply(&harness), Some(egui::accesskit::Toggled::False));
    for _ in 0..400 {
        if harness.get_by_label("Auto Apply").is_focused() {
            break;
        }
        harness.key_press(egui::Key::Tab);
        harness.step();
    }
    assert!(harness.get_by_label("Auto Apply").is_focused(), "Tab never reached Auto Apply");
    harness.key_press(egui::Key::Space);
    step_until(&mut harness, "Auto Apply turning on", |h| auto_apply(h) == Some(egui::accesskit::Toggled::True));
}
