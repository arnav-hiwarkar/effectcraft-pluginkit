use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use craft_plugin::{Call, MaskPath, Permission, Pixel, RenderCx, UiEvent, UiEventKind, UiInput, Widget, native_target};
use craft_plugin_host::{Access, Bundle, Host, Limits, Pixels, Plugin, PluginError, Registry};
use serde_json::{Value, json};

fn probe_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/probe")
}

fn build(extra: &[&str]) {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "--quiet", "--manifest-path"])
        .arg(probe_dir().join("Cargo.toml"))
        .args(extra)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the probe plugin builds");
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Wasm,
    Native,
}

fn bundle(kind: Kind, permissions: &[&str]) -> (tempfile::TempDir, Bundle) {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        build(&["--target", "wasm32-unknown-unknown"]);
        build(&[]);
    });
    let dir = tempfile::tempdir().unwrap();
    let release = probe_dir().join("target");
    let native_file = libloading::library_filename("probe").into_string().unwrap();
    let (entry, file, built) = match kind {
        Kind::Wasm => (json!({"wasm": "probe.wasm"}), "probe.wasm", release.join("wasm32-unknown-unknown/release/probe.wasm")),
        Kind::Native => (json!({"native": {native_target(): native_file}}), native_file.as_str(), release.join("release").join(&native_file)),
    };
    std::fs::copy(&built, dir.path().join(file)).unwrap();
    let manifest = json!({"id": "test.probe", "name": "Probe", "version": "1.0.0", "entry": entry, "permissions": permissions});
    std::fs::write(dir.path().join("plugin.json"), manifest.to_string()).unwrap();
    let bundle = Bundle::open(dir.path()).unwrap();
    (dir, bundle)
}

struct Loaded {
    _dir: tempfile::TempDir,
    data: tempfile::TempDir,
    plugin: Plugin,
}

fn load(kind: Kind, permissions: &[&str]) -> Loaded {
    let (dir, bundle) = bundle(kind, permissions);
    let data = tempfile::tempdir().unwrap();
    let grants = bundle.manifest.permissions.clone();
    let limits = Limits { interactive: Duration::from_millis(400), work: Duration::from_millis(400) };
    let plugin = Plugin::load(&bundle, grants, data.path().join("data"), limits).unwrap();
    Loaded { _dir: dir, data, plugin }
}

#[derive(Default)]
struct Recorder {
    commands: Vec<String>,
}

impl Host for Recorder {
    fn access(&self, command: &str, _params: &Value) -> Access {
        match command {
            "comp.info" => Access::Needs(vec![Permission::DocumentRead]),
            "script.run" => Access::Refused("plugins cannot run scripts".into()),
            _ => Access::Needs(vec![Permission::DocumentWrite]),
        }
    }

    fn command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.commands.push(id.to_string());
        Ok(json!({"echo": params}))
    }

    fn layer(&mut self, param: &str, _comp_time: Option<f64>) -> Option<Pixels> {
        (param == "matte").then(|| Pixels { width: 2, height: 1, data: vec![[0.0; 4], [0.625, 0.0, 0.0, 1.0]] })
    }

    fn source(&mut self, layer_time: f64) -> Option<Pixels> {
        Some(Pixels { width: 1, height: 1, data: vec![[0.0, 0.0, 0.0, layer_time as f32]] })
    }

    fn masks(&mut self) -> Vec<MaskPath> {
        vec![MaskPath { name: "m".into(), points: vec![[0.0, 0.0], [42.0, 7.0]], closed: true, inverted: false }]
    }
}

fn cx(width: u32, height: u32) -> RenderCx {
    RenderCx { size: [width, height], scale: 1.0, time: 3.0, ..Default::default() }
}

fn command(loaded: &Loaded, id: &str, host: &mut Recorder) -> Result<(), PluginError> {
    loaded.plugin.call(&Call::Command { command: id.into() }, None, host)
}

fn each_kind(test: impl Fn(Kind)) {
    test(Kind::Wasm);
    test(Kind::Native);
}

#[test]
fn a_plugin_describes_its_effects_panels_and_commands() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let found = loaded.plugin.contributions();
        let effects: Vec<&str> = found.effects.iter().map(|e| e.id.as_str()).collect();
        assert!(["invert", "shift-down", "read-inputs", "spin"].iter().all(|id| effects.contains(id)), "{kind:?} {effects:?}");
        assert_eq!(found.panels[0].title, "Counter");
        assert!(found.commands.iter().any(|c| c.id == "edit"));
    });
}

#[test]
fn an_effect_changes_the_pixels_it_is_given_using_its_parameters() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut cx = cx(2, 1);
        cx.params.insert("amount".into(), craft_plugin::Value::Number(1.0));
        let mut pixels: Vec<Pixel> = vec![[0.25, 0.5, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]];
        loaded.plugin.render("invert", &cx, Some(0), &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels, [[0.75, 0.5, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]], "{kind:?}");
    });
}

#[test]
fn rendering_in_parallel_bands_gives_the_same_picture_as_one_pass() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let (width, height) = (7usize, 1024usize);
        let source: Vec<Pixel> = (0..width * height).map(|i| [i as f32, (i / width) as f32, 0.5, 1.0]).collect();
        let mut expected = vec![[0.0; 4]; width];
        expected.extend_from_slice(&source[..width * (height - 1)]);

        let cx = cx(width as u32, height as u32);
        let mut banded = source.clone();
        loaded.plugin.render("shift-down", &cx, Some(1), &mut banded, Recorder::default).unwrap();
        assert!(banded == expected, "{kind:?} banded render differs from the shifted source");

        let mut whole = source.clone();
        loaded.plugin.render("shift-down", &cx, None, &mut whole, Recorder::default).unwrap();
        assert!(whole == expected, "{kind:?} whole-frame render differs from the shifted source");
    });
}

#[test]
fn an_effect_reads_masks_another_layer_and_its_own_earlier_frame_through_the_host() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]; 4];
        loaded.plugin.render("read-inputs", &cx(2, 2), None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels[0], [1.0, 42.0, 0.625, 2.0], "{kind:?}");
    });
}

#[test]
fn a_panel_keeps_its_state_between_frames_and_reacts_to_events() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let frame = |events: Vec<UiEvent>| -> Vec<Widget> {
            let call = Call::Panel { panel: "counter".into(), input: UiInput { events, size: [300.0, 200.0] } };
            loaded.plugin.call(&call, None, &mut Recorder::default()).unwrap()
        };
        let first = frame(vec![]);
        let (Widget::Button { id: button, .. }, Widget::Slider { id: slider, .. }) = (&first[0], &first[1]) else {
            panic!("{kind:?} unexpected widgets {first:?}")
        };
        frame(vec![UiEvent { id: *button, kind: UiEventKind::Click }]);
        frame(vec![UiEvent { id: *button, kind: UiEventKind::Click }, UiEvent { id: *slider, kind: UiEventKind::Number(4.0) }]);
        let last = frame(vec![]);
        assert_eq!(last[2], Widget::Label { text: "2 clicks at speed 4".into(), style: craft_plugin::TextStyle::Body });
    });
}

#[test]
fn a_command_edits_the_document_only_with_the_permission_for_it() {
    each_kind(|kind| {
        let denied = load(kind, &["document.read"]);
        let mut host = Recorder::default();
        let error = command(&denied, "edit", &mut host).unwrap_err();
        assert!(error.to_string().contains("document.write"), "{kind:?} {error}");
        assert!(host.commands.is_empty(), "{kind:?} the app command ran without permission");
        command(&denied, "read", &mut host).unwrap();
        assert_eq!(host.commands, ["comp.info"]);

        let allowed = load(kind, &["document.write"]);
        let mut host = Recorder::default();
        command(&allowed, "edit", &mut host).unwrap();
        assert_eq!(host.commands, ["layer.newNull"]);
        let saved = std::fs::read_to_string(allowed.data.path().join("data/last-reply.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), json!({"echo": {"name": "From Plugin"}}));
    });
}

#[test]
fn a_command_the_app_refuses_stays_refused_whatever_was_granted() {
    each_kind(|kind| {
        let loaded = load(kind, &["document.read", "document.write", "files.read", "files.write"]);
        let mut host = Recorder::default();
        let error = command(&loaded, "run-script", &mut host).unwrap_err();
        assert!(error.to_string().contains("refused"), "{kind:?} {error}");
        assert!(host.commands.is_empty());
    });
}

#[test]
fn files_outside_the_plugins_own_folders_need_the_files_permission() {
    each_kind(|kind| {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("out.txt");
        let attempt = |permissions: &[&str]| {
            let loaded = load(kind, permissions);
            std::fs::write(loaded._dir.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
            command(&loaded, "write-user", &mut Recorder::default())
        };
        let error = attempt(&[]).unwrap_err();
        assert!(error.to_string().contains("files.write"), "{kind:?} {error}");
        assert!(!target.exists(), "{kind:?} wrote a user file without permission");
        attempt(&["files.write"]).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "written by plugin");
    });
}

#[test]
fn the_private_data_folder_cannot_be_escaped_and_the_bundle_is_read_only() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut host = Recorder::default();
        assert!(command(&loaded, "escape-data", &mut host).is_err(), "{kind:?}");
        assert!(!loaded.data.path().join("outside.txt").exists());
        let error = command(&loaded, "write-bundle", &mut host).unwrap_err();
        assert!(error.to_string().contains("read-only"), "{kind:?} {error}");
    });
}

#[test]
fn a_sandboxed_plugin_that_never_returns_is_stopped_and_the_next_call_still_works() {
    let loaded = load(Kind::Wasm, &[]);
    let mut pixels: Vec<Pixel> = vec![[0.0; 4]; 4];
    let stopped = loaded.plugin.render("spin", &cx(2, 2), None, &mut pixels, Recorder::default).unwrap_err();
    assert!(matches!(stopped, PluginError::Trap(_)), "{stopped:?}");
    let mut pixels: Vec<Pixel> = vec![[0.25, 0.25, 0.25, 1.0]];
    let mut cx = cx(1, 1);
    cx.params.insert("amount".into(), craft_plugin::Value::Number(1.0));
    loaded.plugin.render("invert", &cx, None, &mut pixels, Recorder::default).unwrap();
    assert_eq!(pixels[0], [0.75, 0.75, 0.75, 1.0]);
}

#[test]
fn an_installed_plugin_runs_only_after_the_user_approves_exactly_what_it_asks_for() {
    let (source, bundle) = bundle(Kind::Wasm, &["document.read"]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    assert_eq!(registry.grants(&installed.manifest), None, "installed plugins start disabled");

    registry.approve(&installed.manifest).unwrap();
    let reopened = Registry::open(root.path());
    assert_eq!(reopened.grants(&installed.manifest), Some(BTreeSet::from([Permission::DocumentRead])));

    let mut greedier = bundle.manifest.clone();
    greedier.permissions.insert(Permission::FilesWrite);
    assert_eq!(reopened.grants(&greedier), None, "an update asking for more waits for a new approval");

    registry.disable("test.probe").unwrap();
    assert_eq!(registry.grants(&installed.manifest), None);
    registry.uninstall("test.probe").unwrap();
    assert!(registry.bundles().is_empty());
}

#[test]
fn a_native_plugin_needs_explicit_trust_beyond_its_permissions() {
    let (source, _) = bundle(Kind::Native, &[]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    assert!(!installed.manifest.sandboxed());
    assert_eq!(registry.grants(&installed.manifest), None);
    registry.approve(&installed.manifest).unwrap();
    assert!(registry.consent("test.probe").is_some_and(|consent| consent.unsandboxed));
    assert_eq!(registry.grants(&installed.manifest), Some(BTreeSet::new()));
}

#[test]
fn bands_cover_every_row_whatever_the_thread_count_and_frame_height() {
    let loaded = load(Kind::Wasm, &[]);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(67).build().unwrap();
    for (width, height) in [(3usize, 4289usize), (5, 129), (2, 64), (1, 1)] {
        let source: Vec<Pixel> = (0..width * height).map(|i| [i as f32, 0.25, 0.5, 1.0]).collect();
        let mut expected = vec![[0.0; 4]; width];
        expected.extend_from_slice(&source[..width * (height - 1)]);
        let mut rendered = source.clone();
        pool.install(|| loaded.plugin.render("shift-down", &cx(width as u32, height as u32), Some(1), &mut rendered, Recorder::default)).unwrap();
        assert!(rendered == expected, "{width}x{height} differs from the shifted source");
    }
}

#[test]
fn an_empty_frame_renders_to_nothing_without_failing() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut none: Vec<Pixel> = Vec::new();
        for size in [[0, 0], [0, 720], [1280, 0]] {
            let cx = RenderCx { size, scale: 1.0, ..Default::default() };
            loaded.plugin.render("invert", &cx, Some(0), &mut none, Recorder::default).unwrap_or_else(|e| panic!("{kind:?} {size:?}: {e}"));
        }
    });
}

#[test]
fn a_sandboxed_effect_that_fails_leaves_the_frame_as_it_was() {
    let loaded = load(Kind::Wasm, &[]);
    let mut pixels: Vec<Pixel> = vec![[0.1, 0.2, 0.3, 1.0]; 4];
    let failed = loaded.plugin.render("half-done", &cx(2, 2), None, &mut pixels, Recorder::default).unwrap_err();
    assert!(failed.to_string().contains("gave up half way"), "{failed}");
    assert_eq!(pixels, vec![[0.1, 0.2, 0.3, 1.0]; 4]);
}

#[cfg(unix)]
#[test]
fn a_link_in_the_data_folder_cannot_redirect_a_write_outside_it() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("planted.txt");
        let data = loaded.data.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::os::unix::fs::symlink(&target, data.join("link.txt")).unwrap();
        assert!(command(&loaded, "write-link", &mut Recorder::default()).is_err(), "{kind:?}");
        assert!(!target.exists(), "{kind:?} wrote through the link");
    });
}

#[test]
fn a_half_finished_install_is_not_treated_as_an_installed_plugin() {
    let (source, _) = bundle(Kind::Wasm, &[]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    registry.approve(&installed.manifest).unwrap();

    let staged = root.path().join(".installing-test.probe");
    std::fs::create_dir_all(&staged).unwrap();
    for file in ["plugin.json", "probe.wasm"] {
        std::fs::copy(source.path().join(file), staged.join(file)).unwrap();
    }
    let listed: Vec<_> = Registry::open(root.path()).bundles().into_iter().flatten().map(|b| b.dir).collect();
    assert_eq!(listed, [installed.dir.clone()]);

    let again = registry.install(source.path()).unwrap();
    assert_eq!(again.dir, installed.dir);
    assert!(!staged.exists(), "the leftover staging folder is cleaned up by the next install");
}
