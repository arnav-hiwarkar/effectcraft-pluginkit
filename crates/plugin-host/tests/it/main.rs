use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use craft_plugin::{
    Call, GpuImage, GpuPass, GpuPlan, MaskPath, ParamsAt, Permission, Pixel, RenderCx, TaskState, TaskStatus, UiEvent, UiEventKind, UiInput, Widget,
    native_target,
};
use craft_plugin_host::{Access, Bundle, Host, Limits, Pixels, Plugin, PluginError, Registry};
use serde_json::{Value, json};

mod package;

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
    picked: Option<PathBuf>,
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

    fn frame(&mut self, request: &craft_plugin::FrameRequest) -> Result<Pixels, String> {
        let width = (4.0 * request.scale) as u32;
        Ok(Pixels { width, height: 1, data: vec![[request.time as f32, 0.0, 0.0, 1.0]; width as usize] })
    }

    fn pick_file(&mut self, _request: &craft_plugin::PickFile) -> Result<Option<PathBuf>, String> {
        Ok(self.picked.clone())
    }

    fn params_at(&mut self, layer_time: f64) -> Option<ParamsAt> {
        let params = [("level".to_string(), craft_plugin::Value::Number(10.0 + 10.0 * layer_time))].into();
        Some(ParamsAt { params, history: Some(7) })
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

        let mut tall = source.clone();
        loaded.plugin.render("shift-down", &cx, Some(150), &mut tall, Recorder::default).unwrap();
        assert!(tall == expected, "{kind:?} render with a reach taller than an even band differs from the shifted source");

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
fn an_effect_reads_its_parameters_at_another_time_through_the_host() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut cx = cx(1, 1);
        cx.params.insert("level".into(), craft_plugin::Value::Number(40.0));
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]];
        loaded.plugin.render("report", &cx, None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels[0], [40.0, 10.0, 0.0, 1.0], "{kind:?}");
    });
}

#[test]
fn a_panel_keeps_its_state_between_frames_and_reacts_to_events() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let frame = |events: Vec<UiEvent>| -> Vec<Widget> {
            let call = Call::Panel { panel: "counter".into(), input: UiInput { events, size: [300.0, 200.0], ..Default::default() } };
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
fn a_sandboxed_plugin_whose_host_call_returns_after_its_time_limit_fails() {
    struct Slow;
    impl Host for Slow {
        fn access(&self, _command: &str, _params: &Value) -> Access {
            Access::Needs(vec![])
        }
        fn command(&mut self, _id: &str, _params: Value) -> Result<Value, String> {
            std::thread::sleep(Duration::from_millis(700));
            Ok(Value::Null)
        }
    }
    let loaded = load(Kind::Wasm, &[]);
    std::fs::write(loaded._dir.path().join("relay.json"), json!({"command": "comp.info", "params": {}}).to_string()).unwrap();
    let late = loaded.plugin.call::<()>(&Call::Command { command: "relay".into() }, None, &mut Slow).unwrap_err();
    assert!(matches!(&late, PluginError::Trap(why) if why.contains("time limit")), "{late:?}");
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
    assert_eq!(registry.mode(&installed.manifest), craft_plugin::RunMode::Unsandboxed);
    assert_eq!(registry.grants(&installed.manifest), None);
    registry.approve(&installed.manifest).unwrap();
    assert_eq!(registry.grants(&installed.manifest), Some(BTreeSet::new()));

    let mut next_version = installed.manifest.clone();
    next_version.version = "1.0.1".into();
    assert_eq!(registry.grants(&next_version), None, "a new native version waits for a new approval");
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
    assert_eq!(listed, std::slice::from_ref(&installed.dir));

    let again = registry.install(source.path()).unwrap();
    assert_eq!(again.dir, installed.dir);
    assert!(!staged.exists(), "the leftover staging folder is cleaned up by the next install");
}

#[test]
fn an_update_whose_entry_file_is_missing_keeps_the_working_install() {
    let (source, _) = bundle(Kind::Wasm, &[]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();

    let broken = tempfile::tempdir().unwrap();
    let manifest = json!({"id": "test.probe", "name": "Probe", "version": "2.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": []});
    std::fs::write(broken.path().join("plugin.json"), manifest.to_string()).unwrap();
    let error = registry.install(broken.path()).unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");

    let kept = Bundle::open(&installed.dir).unwrap();
    assert_eq!(kept.manifest.version, "1.0.0");
    let data = tempfile::tempdir().unwrap();
    let limits = Limits { interactive: Duration::from_millis(400), work: Duration::from_millis(400) };
    assert!(Plugin::load(&kept, BTreeSet::new(), data.path().join("data"), limits).is_ok());
}

#[test]
fn a_plugin_with_file_access_cannot_touch_the_apps_plugin_folders() {
    each_kind(|kind| {
        let (source, _) = bundle(kind, &["files.read", "files.write"]);
        let root = tempfile::tempdir().unwrap();
        let mut registry = Registry::open(root.path());
        let installed = registry.install(source.path()).unwrap();
        registry.approve(&installed.manifest).unwrap();
        let running = registry.running(&installed);
        let grants = registry.grants(&running.manifest).unwrap();
        let limits = Limits { interactive: Duration::from_millis(400), work: Duration::from_millis(400) };
        let plugin = Plugin::load(&running, grants, registry.data_dir("test.probe"), limits).unwrap();
        let compiled = root.path().join(".data/.compiled");
        std::fs::create_dir_all(&compiled).unwrap();
        let consent = root.path().join("state.json");
        let before = std::fs::read_to_string(&consent).unwrap();
        let entry = running.entry_path().unwrap();
        let attempt = |command: &str, target: &Path| {
            std::fs::write(installed.dir.join("user-target.txt"), target.to_str().unwrap()).unwrap();
            plugin.call::<()>(&Call::Command { command: command.into() }, None, &mut Recorder::default()).unwrap_err().to_string()
        };
        for target in [compiled.join("planted.cwasm"), consent.clone(), entry.clone()] {
            let error = attempt("write-user", &target);
            assert!(error.contains("that folder belongs to the app"), "{kind:?} {target:?} {error}");
            let error = attempt("read-user", &target);
            assert!(error.contains("that folder belongs to the app"), "{kind:?} {target:?} {error}");
        }
        assert!(!compiled.join("planted.cwasm").exists(), "{kind:?}");
        assert_eq!(std::fs::read_to_string(&consent).unwrap(), before);
        assert!(std::fs::read(&entry).unwrap().len() > 2, "{kind:?}");
    });
}

fn settle(loaded: &Loaded, id: u64, host: &mut Recorder) -> TaskState {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        loaded.plugin.pump(host);
        let state = loaded.plugin.tasks().into_iter().find(|task| task.id == id).expect("the task is listed");
        if state.status != TaskStatus::Running || std::time::Instant::now() > until {
            return state;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_task_reports_progress_and_finishes_with_its_result_without_a_time_limit() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let id = loaded.plugin.start("count", json!({"steps": 4})).unwrap();
        let state = settle(&loaded, id, &mut Recorder::default());
        assert_eq!(state.status, TaskStatus::Done, "{kind:?} {state:?}");
        assert_eq!(state.result, Some(json!({"counted": 4})));
        assert_eq!((state.progress, state.message.as_str()), (1.0, "step 3"));

        command(&loaded, "start-count", &mut Recorder::default()).unwrap();
        let started: u64 = std::fs::read_to_string(loaded.data.path().join("data/task-id.txt")).unwrap().parse().unwrap();
        assert_eq!(started, id + 1);
        assert_eq!(settle(&loaded, started, &mut Recorder::default()).result, Some(json!({"counted": 3})));
        assert!(loaded.plugin.start("missing", json!({})).unwrap_err().to_string().contains("no task `missing`"));
    });
}

#[test]
fn cancel_stops_a_task_that_never_reports_progress() {
    let loaded = load(Kind::Wasm, &[]);
    let id = loaded.plugin.start("forever", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(loaded.plugin.tasks()[0].status, TaskStatus::Running, "a task has no fixed time limit");
    let asked = std::time::Instant::now();
    assert!(loaded.plugin.cancel(id));
    let state = settle(&loaded, id, &mut Recorder::default());
    assert_eq!(state.status, TaskStatus::Cancelled);
    assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());

    let native = load(Kind::Native, &[]);
    let id = native.plugin.start("patient", json!({})).unwrap();
    native.plugin.cancel(id);
    assert_eq!(settle(&native, id, &mut Recorder::default()).status, TaskStatus::Cancelled);
}

#[test]
fn a_task_edits_the_document_on_the_apps_thread_only_with_the_permission_for_it() {
    each_kind(|kind| {
        let denied = load(kind, &["document.read"]);
        let mut host = Recorder::default();
        let id = denied.plugin.start("edit", json!({})).unwrap();
        let state = settle(&denied, id, &mut host);
        assert_eq!(state.status, TaskStatus::Failed, "{kind:?}");
        assert!(state.error.as_deref().unwrap_or_default().contains("document.write"), "{kind:?} {state:?}");
        assert!(host.commands.is_empty());

        let allowed = load(kind, &["document.write"]);
        let id = allowed.plugin.start("edit", json!({})).unwrap();
        let state = settle(&allowed, id, &mut host);
        assert_eq!(state.result, Some(json!({"echo": {"name": "From Task"}})), "{kind:?}");
        assert_eq!(host.commands, ["layer.newNull"]);
    });
}

#[test]
fn a_plugin_runs_at_most_four_tasks_at_once_and_stopping_it_cancels_them() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let ids: Vec<u64> = (0..4).map(|_| loaded.plugin.start("patient", json!({})).unwrap()).collect();
        let fifth = loaded.plugin.start("patient", json!({})).unwrap_err();
        assert!(fifth.to_string().contains("already runs 4 tasks"), "{kind:?} {fifth}");
        loaded.plugin.cancel_all();
        let states: Vec<TaskStatus> = ids.iter().map(|id| settle(&loaded, *id, &mut Recorder::default()).status).collect();
        assert_eq!(states, [TaskStatus::Cancelled; 4], "{kind:?}");
        assert_eq!(loaded.plugin.running_tasks(), 0);
        loaded.plugin.start("count", json!({"steps": 1})).unwrap();
    });
}

#[test]
fn a_task_reads_a_frame_through_the_app_only_with_document_read() {
    each_kind(|kind| {
        let reader = load(kind, &["document.read"]);
        let id = reader.plugin.start("frame", json!({"time": 0.75, "scale": 0.5})).unwrap();
        let state = settle(&reader, id, &mut Recorder::default());
        assert_eq!(state.result, Some(json!({"width": 2, "height": 1, "center": [0.75, 0.0, 0.0, 1.0]})), "{kind:?} {state:?}");

        let blind = load(kind, &[]);
        let id = blind.plugin.start("frame", json!({"time": 0.0})).unwrap();
        let state = settle(&blind, id, &mut Recorder::default());
        assert!(state.error.as_deref().unwrap_or_default().contains("document.read"), "{kind:?} {state:?}");
    });
}

#[test]
fn an_effect_cannot_render_a_frame_from_inside_its_own_render() {
    each_kind(|kind| {
        let loaded = load(kind, &["document.read"]);
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]];
        loaded.plugin.render("recurse", &cx(1, 1), None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels[0][0], 1.0, "{kind:?}");
    });
}

#[test]
fn a_picked_file_becomes_readable_without_the_files_permission_but_not_writable() {
    each_kind(|kind| {
        let outside = tempfile::tempdir().unwrap();
        let chosen = outside.path().join("chosen.txt");
        std::fs::write(&chosen, "chosen by the user").unwrap();
        let loaded = load(kind, &[]);
        let mut host = Recorder { picked: Some(chosen.clone()), ..Default::default() };
        let error = command(&loaded, "pick-and-read", &mut host).unwrap_err();
        assert!(error.to_string().contains("files.write"), "{kind:?} {error}");
        assert_eq!(std::fs::read_to_string(loaded.data.path().join("data/picked.txt")).unwrap(), "chosen by the user");
        assert_eq!(std::fs::read_to_string(&chosen).unwrap(), "chosen by the user");

        let id = loaded.plugin.start("pick", json!({})).unwrap();
        let state = settle(&loaded, id, &mut host);
        assert!(state.error.as_deref().unwrap_or_default().contains("only from a panel or a command"), "{kind:?} {state:?}");
    });
}

const PASS_FRAME: [u32; 2] = [37, 301];

fn ramp() -> Vec<Pixel> {
    let [width, height] = PASS_FRAME;
    (0..height).flat_map(|y| (0..width).map(move |x| [x as f32, y as f32, (x * y % 7) as f32, 1.0])).collect()
}

fn run_plan(kind: Kind, plan: &GpuPlan) -> Result<Vec<Pixel>, PluginError> {
    let loaded = load(kind, &[]);
    let mut pixels = ramp();
    loaded.plugin.run_passes("passes", &cx(PASS_FRAME[0], PASS_FRAME[1]), plan, &mut pixels, Recorder::default)?;
    Ok(pixels)
}

fn halve_and_restore(reach: [Option<u32>; 2]) -> GpuPlan {
    let mut plan = GpuPlan::new();
    let half = plan.scratch(PASS_FRAME.map(|side| side.div_ceil(2)));
    let mut down = GpuPass::new("probe_down", GpuImage::Source, half);
    let mut up = GpuPass::new("probe_up", half, GpuImage::Output);
    (down.reach, up.reach) = (reach[0], reach[1]);
    plan.pass(down).pass(up)
}

fn at(pixels: &[Pixel], x: usize, y: usize) -> Pixel {
    pixels[y * PASS_FRAME[0] as usize + x]
}

#[test]
fn a_two_pass_plan_gives_the_same_pixels_in_bands_as_in_one_call() {
    each_kind(|kind| {
        let banded = run_plan(kind, &halve_and_restore([Some(2), Some(1)])).unwrap();
        let whole = run_plan(kind, &halve_and_restore([None, None])).unwrap();
        assert_eq!(banded, whole, "{kind:?}");
        assert_eq!(at(&banded, 10, 100), [10.5, 101.5, 3.5, 1.0], "{kind:?}");
        assert_eq!(at(&banded, 10, 75)[..2], [10.5, 75.5], "{kind:?}");
        assert_eq!(at(&banded, 10, 76)[..2], [10.5, 77.5], "{kind:?}");
        assert_eq!(at(&banded, 36, 300), [4.5, 37.5, 0.75, 0.125], "{kind:?}");
    });
}

#[test]
fn a_pass_reads_its_reach_across_band_edges() {
    each_kind(|kind| {
        let tall = |reach| GpuPlan::new().pass(GpuPass { reach, ..GpuPass::new("probe_tall", GpuImage::Source, GpuImage::Output) });
        let banded = run_plan(kind, &tall(Some(3))).unwrap();
        assert_eq!(banded, run_plan(kind, &tall(None)).unwrap(), "{kind:?}");
        assert_eq!(at(&banded, 10, 0)[..2], [220.0, 38.0], "{kind:?}");
        assert_eq!(at(&banded, 10, 75), [280.0, 2128.0, 70.0, 28.0], "{kind:?}");
        assert_eq!(at(&banded, 10, 76)[..2], [280.0, 2156.0], "{kind:?}");
        assert_eq!(at(&banded, 10, 300)[..2], [100.0, 2990.0], "{kind:?}");
    });
}

#[test]
fn a_pass_with_too_little_reach_is_missing_rows_at_band_edges() {
    let short = GpuPlan::new().pass(GpuPass::new("probe_tall", GpuImage::Source, GpuImage::Output).reach(2));
    let pixels = run_plan(Kind::Wasm, &short).unwrap();
    assert_eq!(at(&pixels, 10, 74)[..2], [280.0, 2100.0]);
    assert_eq!(at(&pixels, 10, 75)[..2], [210.0, 1582.0]);
}

#[test]
fn a_plan_naming_an_image_it_does_not_declare_is_refused() {
    each_kind(|kind| {
        let writes = GpuPlan::new().pass(GpuPass::new("probe_tall", GpuImage::Source, GpuImage::Scratch(3)));
        assert_eq!(run_plan(kind, &writes), Err(PluginError::Failed("pass 0 writes Scratch(3), which the plan does not declare".into())));
        let reads = GpuPlan::new().pass(GpuPass::new("probe_tall", GpuImage::Scratch(1), GpuImage::Output));
        assert_eq!(run_plan(kind, &reads), Err(PluginError::Failed("pass 0 reads Scratch(1), which the plan does not declare".into())));
    });
}

#[test]
fn a_pass_writing_the_image_it_reads_is_refused() {
    let tall = |src| GpuPass::new("probe_tall", src, GpuImage::Output).reach(3);
    let plan = GpuPlan::new().pass(tall(GpuImage::Source)).pass(tall(GpuImage::Output));
    assert_eq!(run_plan(Kind::Wasm, &plan), Err(PluginError::Failed("pass 1 reads the image it writes".into())));
}

#[test]
fn a_failing_pass_fails_the_render() {
    each_kind(|kind| {
        let plan = GpuPlan::new().pass(GpuPass::new("nope", GpuImage::Source, GpuImage::Output).reach(0));
        assert_eq!(run_plan(kind, &plan), Err(PluginError::Failed("no probe kernel `nope`".into())));
    });
}
