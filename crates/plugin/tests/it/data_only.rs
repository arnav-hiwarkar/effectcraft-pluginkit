use std::path::{Path, PathBuf};

use effectcraft_engine::effects::contributed;
use effectcraft_engine::render::{RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use effectcraft_plugin::{PluginState, Plugins};
use serde_json::{Value, json};

const SWAP_RED_AND_BLUE: &str = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";

fn pack(id: &str, luts: &[(&str, &str)], presets: Value) -> (tempfile::TempDir, PathBuf) {
    let source = tempfile::tempdir().unwrap();
    let dir = source.path().join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let names: Vec<&str> = luts.iter().map(|(name, _)| *name).collect();
    let manifest = json!({"id": id, "name": "Looks", "version": "1.0.0", "contributes": {"luts": names, "presets": presets}});
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    luts.iter().for_each(|(name, text)| std::fs::write(dir.join(name), text).unwrap());
    (source, dir)
}

fn center(app: &super::App) -> [f32; 4] {
    let session = &app.session;
    let comp = session.active_comp_id().unwrap();
    let image = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts::default()).comp_frame(comp, Tick::from_seconds_f64(0.0));
    image.data[(image.height / 2 * image.width + image.width / 2) as usize]
}

fn value(app: &mut super::App, path: &str) -> Value {
    app.session.execute_checked("prop.get", json!({"layer": "Solid", "path": path})).unwrap()["value"].clone()
}

fn lut_path(owner: &str) -> String {
    contributed::luts().into_iter().find(|lut| lut.owner == owner).unwrap().path
}

#[test]
fn a_bundle_with_presets_and_a_lut_and_no_code_installs_runs_nothing_and_needs_no_approval() {
    let id = "test.data-looks";
    let mut app = super::app("test.data-only-host", &[]);
    let (_source, dir) = pack(
        id,
        &[("swap.cube", SWAP_RED_AND_BLUE)],
        json!([{"effect": "ec.color.tint", "name": "Warm", "values": {"amount": 35.0, "black": [0.1, 0.0, 0.0, 1.0]}}]),
    );
    let info = app.plugins.install(&dir).unwrap();
    assert_eq!((info.state, info.error, info.permissions.len(), info.sandboxed), (PluginState::Running, None, 0, true));
    assert!(app.plugins.get(id).is_none());

    app.session.execute("layer.newSolid", json!({"name": "Solid", "color": "#ff0000", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Solid", "effect": "ec.color.tint"})).unwrap();
    let listed = app.session.execute_checked("effect.presets", json!({"layer": "Solid", "effect": 1})).unwrap();
    assert_eq!(listed, json!({"presets": ["Warm"]}));
    app.session.execute_checked("effect.applyPreset", json!({"layer": "Solid", "effect": 1, "preset": "Warm"})).unwrap();
    assert_eq!((value(&mut app, "effects/#1/amount"), value(&mut app, "effects/#1/black")), (json!(35.0), json!([0.1, 0.0, 0.0, 1.0])));

    let luts = contributed::luts();
    let lut = luts.iter().find(|lut| lut.owner == id).unwrap();
    assert_eq!((lut.name.as_str(), lut.path.as_str()), ("Looks / swap", dir_of(&info.path, "swap.cube").as_str()));
    assert_eq!(effectcraft_engine::effects::load_lut(&lut.path).unwrap().apply([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);

    app.session.execute("effect.remove", json!({"layer": "Solid", "effect": 1})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Solid", "effect": "ec.utility.applylut"})).unwrap();
    app.session.execute_checked("prop.set", json!({"layer": "Solid", "path": "effects/#1/lut", "value": lut.path})).unwrap();
    let [r, g, b, _] = center(&app);
    assert!(r < 0.02 && g < 0.02 && b > 0.98, "{r} {g} {b}");

    app.plugins.uninstall(id).unwrap();
    assert!(contributed::luts().iter().all(|lut| lut.owner != id));
    assert!(effectcraft_engine::effects::presets("ec.color.tint").is_empty());
}

fn dir_of(dir: &Path, file: &str) -> String {
    dir.join(file).to_string_lossy().into_owned()
}

#[test]
fn a_data_only_bundle_survives_a_restart_without_asking_again() {
    let _exclusive = super::exclusive();
    let id = "test.data-restart";
    let root = tempfile::tempdir().unwrap();
    let (_source, dir) = pack(id, &[("swap.cube", SWAP_RED_AND_BLUE)], json!([]));
    Plugins::open(root.path(), false).install(&dir).unwrap();
    contributed::remove(id);
    let restarted = Plugins::open(root.path(), false);
    assert!(lut_path(id).ends_with("swap.cube"));
    restarted.uninstall(id).unwrap();
}

#[test]
fn a_malformed_data_only_bundle_is_refused_with_the_reason_and_installs_nothing() {
    let app = super::app("test.data-only-refuse", &[]);
    let refused = |name: &str, luts: &[(&str, &str)], presets: Value| {
        let (_source, dir) = pack(&format!("test.refused-{name}"), luts, presets);
        let error = app.plugins.install(&dir).unwrap_err();
        assert!(app.plugins.list().iter().all(|info| info.id != format!("test.refused-{name}")), "{error}");
        error
    };
    let preset = |effect: &str, values: Value| json!([{"effect": effect, "name": "Look", "values": values}]);
    assert_eq!(
        refused("effect", &[], preset("ec.color.nothing", json!({}))),
        "preset `Look` is for effect `ec.color.nothing`, which is not a built-in effect or an installed plugin's effect"
    );
    assert_eq!(
        refused("param", &[], preset("ec.color.tint", json!({"strength": 1.0}))),
        "preset `Look` sets `strength`, but effect `ec.color.tint` has no such parameter; it has black, white, amount"
    );
    assert_eq!(refused("type", &[], preset("ec.color.tint", json!({"amount": "lots"}))), "preset `Look`: `amount` of effect `ec.color.tint` needs a number");
    let cube = refused("cube", &[("broken.cube", "LUT_3D_SIZE 2\n0 0 0\n")], json!([]));
    assert!(cube.starts_with("LUT `broken.cube` is not a valid .cube file: "), "{cube}");
    assert_eq!(refused("nothing", &[], json!([])), "plugin.json field `entry` is empty");
}

#[test]
fn a_preset_may_name_only_files_inside_its_own_bundle() {
    let app = super::app("test.data-preset-files", &[]);
    let lut = |value: &str| json!([{"effect": "ec.utility.applylut", "name": "Look", "values": {"lut": value}}]);
    let outside = tempfile::tempdir().unwrap();
    let elsewhere = outside.path().join("other.cube");
    std::fs::write(&elsewhere, SWAP_RED_AND_BLUE).unwrap();
    for (name, value) in [("absolute", elsewhere.to_string_lossy().into_owned()), ("parent", "../other.cube".into()), ("missing", "gone.cube".into())] {
        let (_source, dir) = pack(&format!("test.preset-file-{name}"), &[("swap.cube", SWAP_RED_AND_BLUE)], lut(&value));
        let error = app.plugins.install(&dir).unwrap_err();
        assert!(error.starts_with("preset `Look`: `lut`: "), "{name}: {error}");
    }
    let (_source, dir) = pack("test.preset-file-inside", &[("swap.cube", SWAP_RED_AND_BLUE)], lut("swap.cube"));
    app.plugins.install(&dir).unwrap();
    let presets = effectcraft_engine::effects::presets("ec.utility.applylut");
    let (_, values) = presets.iter().find(|(name, _)| name == "Look").unwrap();
    let path = std::fs::canonicalize(
        effectcraft_plugin::Plugins::list(app.plugins).iter().find(|info| info.id == "test.preset-file-inside").unwrap().path.join("swap.cube"),
    )
    .unwrap();
    assert_eq!(values[0].1, effectcraft_engine::keyframe::Value::Str(path.to_string_lossy().into_owned()));
    app.plugins.uninstall("test.preset-file-inside").unwrap();
}

#[test]
fn reload_removes_a_pack_deleted_from_disk_from_the_menus() {
    let id = "test.data-removed";
    let app = super::app_exclusive("test.data-removed-host", &[]);
    let (_source, dir) =
        pack(id, &[("swap.cube", SWAP_RED_AND_BLUE)], json!([{"effect": "ec.color.brightnesscontrast", "name": "Gone", "values": {"brightness": 1.0}}]));
    let info = app.plugins.install(&dir).unwrap();
    assert!(effectcraft_engine::effects::presets("ec.color.brightnesscontrast").iter().any(|(name, _)| name == "Gone"));
    std::fs::remove_dir_all(&info.path).unwrap();
    app.plugins.reload();
    assert!(effectcraft_engine::effects::presets("ec.color.brightnesscontrast").iter().all(|(name, _)| name != "Gone"));
    assert!(contributed::luts().iter().all(|lut| lut.owner != id));
}
