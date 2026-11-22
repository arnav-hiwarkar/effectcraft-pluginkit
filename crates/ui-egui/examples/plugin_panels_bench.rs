use std::path::{Path, PathBuf};
use std::process::Command;

use craft_ui::docking::Action;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::bench::Driver;
use effectcraft_ui_egui::dock::{PanelKind, WorkspaceExt, permissions};
use serde_json::json;

const FRAMES: usize = 200;
const RUNS: usize = 3;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundle(source: &Path, crate_name: &str, manifest: Option<serde_json::Value>, into: &Path) -> PathBuf {
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(source.join("Cargo.toml"))
        .status()
        .expect("cargo runs");
    assert!(status.success(), "{} builds", source.display());
    let dir = into.join(crate_name);
    std::fs::create_dir_all(&dir).expect("bundle folder");
    std::fs::copy(source.join(format!("target/wasm32-unknown-unknown/release/{crate_name}.wasm")), dir.join("plugin.wasm")).expect("copy the module");
    match manifest {
        Some(manifest) => std::fs::write(dir.join("plugin.json"), manifest.to_string()).expect("write the manifest"),
        None => std::fs::copy(source.join("plugin.json"), dir.join("plugin.json")).map(|_| ()).expect("copy the manifest"),
    }
    dir
}

fn main() {
    let work = std::env::temp_dir().join(format!("plugin-panels-bench-{}", std::process::id()));
    let probe = json!({"id": "test.probe", "name": "Probe", "version": "1.0.0", "entry": {"wasm": "plugin.wasm"}, "permissions": []});
    let bundles = [
        bundle(&root().join("plugins/glide"), "glide", None, &work),
        bundle(&root().join("plugins/pulse"), "pulse", None, &work),
        bundle(&root().join("crates/plugin-host/tests/probe"), "probe", Some(probe), &work),
    ];
    let mut session = effectcraft_host::session();
    let plugins = effectcraft_plugin::install(&mut session, Some(&work.join("config")));
    for dir in &bundles {
        plugins.load_for_session(dir).expect("the plugin loads");
    }
    effectcraft_plugin::install(&mut session, Some(&work.join("config")));
    session.execute("comp.new", json!({"name": "Bench", "width": 640, "height": 360, "duration": 2})).expect("comp.new");
    session.execute("layer.newSolid", json!({"color": "#808080"})).expect("layer.newSolid");
    let mut driver = Driver::new(EffectcraftApp::new(session), egui::vec2(1680.0, 1020.0));
    driver.step();
    driver.step();
    for _ in 0..60 {
        driver.step();
    }
    let without = (0..RUNS).map(|_| driver.frames(FRAMES, |_, _| {}).0).fold(f64::INFINITY, f64::min);
    println!("no plugin panels, idle: best mean of {RUNS} runs {without:.3} ms");
    let panels = [("org.craftkit.glide", "glide"), ("org.craftkit.pulse", "pulse"), ("test.probe", "counter")];
    for (index, (plugin, panel)) in panels.iter().enumerate() {
        let kind = PanelKind::plugin(plugin, panel);
        driver.app.show_panel(kind);
        let rect = [40.0 + 420.0 * index as f32, 80.0, 400.0, 700.0];
        driver.app.ui.dock.apply(Action::Float { panel: kind, rect }, permissions).expect("the panel floats");
    }
    for _ in 0..60 {
        driver.step();
    }
    for (plugin, panel) in panels {
        let shown = driver.app.ui.dock.is_visible(PanelKind::plugin(plugin, panel));
        assert!(shown, "{plugin} {panel} is not on screen");
    }
    let runs: Vec<(f64, f64)> = (0..RUNS).map(|_| driver.frames(FRAMES, |_, _| {})).collect();
    let best = runs.iter().map(|(mean, _)| *mean).fold(f64::INFINITY, f64::min);
    for (mean, max) in &runs {
        println!("three plugin panels, idle: mean {mean:.3} ms, max {max:.3} ms over {FRAMES} frames");
    }
    println!("best mean of {RUNS} runs: {best:.3} ms");
    let _ = std::fs::remove_dir_all(&work);
}
