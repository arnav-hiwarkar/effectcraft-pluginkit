use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use craft_plugin::{
    Call, FrameDesc, GpuImage, GpuPass, GpuPlan, Layout, MaskPath, ParamsAt, Permission, Pixel, RenderCx, TaskState, TaskStatus, UiEvent, UiEventKind, UiInput,
    Widget, native_target,
};
use craft_plugin_host::{Access, Bundle, Host, Limits, Pixels, Plugin, PluginError, Rect, Registry, Retained};
use serde_json::{Value, json};

mod package;
mod testing;

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
    open(dir, bundle)
}

fn load_before_regions() -> Loaded {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/probe-before-regions.wasm"), dir.path().join("probe.wasm")).unwrap();
    let manifest = json!({"id": "test.probe-before-regions", "name": "Probe", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}});
    std::fs::write(dir.path().join("plugin.json"), manifest.to_string()).unwrap();
    let bundle = Bundle::open(dir.path()).unwrap();
    open(dir, bundle)
}

fn open(dir: tempfile::TempDir, bundle: Bundle) -> Loaded {
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
    clipboard: String,
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

    fn clipboard(&mut self) -> Result<String, String> {
        Ok(self.clipboard.clone())
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        self.clipboard = text.to_string();
        Ok(())
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
        loaded.plugin.render("invert", &cx, &Layout::neighbourhood(0), None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels, [[0.75, 0.5, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]], "{kind:?}");
    });
}

#[test]
fn fast_maths_gives_the_same_bits_sandboxed_and_native_and_stays_within_its_bounds() {
    let source: Vec<Pixel> = (0..4096).map(|i| [-3.0 + i as f32 * (6.0 / 4096.0), 0.0, 0.0, 1.0]).collect();
    let render = |kind: Kind| {
        let loaded = load(kind, &[]);
        let mut pixels = source.clone();
        loaded.plugin.render("fast-maths", &cx(4096, 1), &Layout::neighbourhood(0), None, &mut pixels, Recorder::default).unwrap();
        pixels
    };
    let (wasm, native) = (render(Kind::Wasm), render(Kind::Native));
    assert!(wasm == native, "the sandboxed and the native render differ");
    let mut worst = [0.0f64; 4];
    for (input, out) in source.iter().zip(&wasm) {
        let x = f64::from(input[0]);
        let exact = [x.exp(), x.sin(), x.atan2(2.0 - x), x.abs().powf(2.4)];
        let relative = [true, false, false, true];
        for c in 0..4 {
            let error = (f64::from(out[c]) - exact[c]).abs();
            worst[c] = worst[c].max(if relative[c] { error / exact[c].max(1e-30) } else { error });
        }
    }
    assert!(worst[0] < 5e-7 && worst[1] < 2e-6 && worst[2] < 5e-6 && worst[3] < 2e-5, "{worst:?}");
    assert!((wasm[3072][0] - 4.481_689).abs() < 1e-5 && (wasm[3072][1] - 0.997_495).abs() < 1e-5, "{:?}", wasm[3072]);
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
        loaded.plugin.render("shift-down", &cx, &Layout::neighbourhood(1), None, &mut banded, Recorder::default).unwrap();
        assert!(banded == expected, "{kind:?} banded render differs from the shifted source");

        let mut tall = source.clone();
        loaded.plugin.render("shift-down", &cx, &Layout::neighbourhood(150), None, &mut tall, Recorder::default).unwrap();
        assert!(tall == expected, "{kind:?} render with a reach taller than an even band differs from the shifted source");

        let mut whole = source.clone();
        loaded.plugin.render("shift-down", &cx, &Layout::whole_frame(), None, &mut whole, Recorder::default).unwrap();
        assert!(whole == expected, "{kind:?} whole-frame render differs from the shifted source");
    });
}

fn numbered(width: usize, height: usize) -> Vec<Pixel> {
    (0..width * height).map(|i| [i as f32, (i / width) as f32, 0.5, 1.0]).collect()
}

fn cut(pixels: &[Pixel], width: usize, part: Rect) -> Vec<Pixel> {
    let columns = part.left as usize..part.right as usize;
    (part.top as usize..part.bottom as usize).flat_map(|y| pixels[y * width..(y + 1) * width][columns.clone()].to_vec()).collect()
}

#[test]
fn a_render_of_a_part_of_the_frame_equals_that_part_cut_from_a_whole_render() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let (width, height) = (40usize, 300usize);
    let (source, cx, layout) = (numbered(width, height), cx(width as u32, height as u32), Layout::neighbourhood(3));
    let part = Rect { left: 7, top: 101, right: 29, bottom: 260 };
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut whole = source.clone();
        pool.install(|| loaded.plugin.render("shift-down", &cx, &layout, None, &mut whole, Recorder::default)).unwrap();
        let mut pixels = source.clone();
        let area = pool.install(|| loaded.plugin.render("shift-down", &cx, &layout, Some(part), &mut pixels, Recorder::default)).unwrap();
        assert_eq!(area, part, "{kind:?}");
        assert_eq!(pixels[0], [4007.0, 100.0, 0.5, 1.0], "{kind:?} the first pixel is the source pixel one row up");
        assert!(pixels == cut(&whole, width, part), "{kind:?} the part differs from the whole render");
    });
    let before = load_before_regions();
    let mut pixels = source.clone();
    let area = pool.install(|| before.plugin.render("shift-down", &cx, &layout, Some(part), &mut pixels, Recorder::default)).unwrap();
    assert_eq!(area, Rect { left: 0, right: 40, ..part }, "a plugin built before regions renders whole rows");
    assert!(pixels == cut(&source, width, Rect { top: 100, bottom: 259, ..area }));
}

#[test]
fn an_effect_without_a_cpu_render_leaves_the_part_it_is_asked_for_unchanged() {
    let source = numbered(5, 4);
    let part = Rect { left: 1, top: 1, right: 4, bottom: 3 };
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut pixels = source.clone();
        loaded.plugin.render("plain", &cx(5, 4), &Layout::whole_frame(), Some(part), &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels, cut(&source, 5, part), "{kind:?}");
        assert_eq!(pixels[0], [6.0, 1.0, 0.5, 1.0], "{kind:?}");
    });
}

#[test]
fn an_effect_that_maps_its_input_reads_the_rows_it_names_in_every_band() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let (width, height) = (3usize, 600usize);
    let source = numbered(width, height);
    let mut expected = source[8 * width..].to_vec();
    expected.resize(width * height, [0.0; 4]);
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut pixels = source.clone();
        let layout = Layout::pointwise().maps_input();
        pool.install(|| loaded.plugin.render("lift", &cx(width as u32, height as u32), &layout, None, &mut pixels, Recorder::default)).unwrap();
        assert_eq!(pixels[149 * width], [471.0, 157.0, 0.5, 1.0], "{kind:?} the last row of the first band");
        assert!(pixels == expected, "{kind:?} the lifted frame differs from the source moved up 8 rows");
    });
}

#[test]
fn a_plugin_built_before_frames_had_separate_input_renders_the_same_pixels() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let (before, now) = (load_before_regions(), load(Kind::Wasm, &[]));
    assert!(!before.plugin.contributions().separate_input && now.plugin.contributions().separate_input);
    let (width, height) = (7usize, 700usize);
    let mut cx = cx(width as u32, height as u32);
    cx.params.insert("amount".into(), craft_plugin::Value::Number(0.25));
    cx.params.insert("strength".into(), craft_plugin::Value::Number(0.75));
    let source: Vec<Pixel> = (0..width * height).map(|i| [(i % 13) as f32 / 13.0, (i % 7) as f32 / 7.0, 0.25, 1.0]).collect();
    for (effect, layout) in [("invert", Layout::pointwise()), ("shift-down", Layout::neighbourhood(2)), ("gpu-soften", Layout::whole_frame())] {
        let render = |loaded: &Loaded| {
            let mut pixels = source.clone();
            pool.install(|| loaded.plugin.render(effect, &cx, &layout, None, &mut pixels, Recorder::default)).unwrap();
            pixels
        };
        let (old, new) = (render(&before), render(&now));
        assert!(old == new, "{effect} renders differently from the plugin built before");
        assert!(old != source, "{effect} left the frame unchanged");
        if effect == "invert" {
            assert_eq!(old[0], [0.25, 0.25, 0.375, 1.0]);
        }
    }
}

#[test]
fn an_effect_reads_masks_another_layer_and_its_own_earlier_frame_through_the_host() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]; 4];
        loaded.plugin.render("read-inputs", &cx(2, 2), &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap();
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
        loaded.plugin.render("report", &cx, &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels[0], [40.0, 10.0, 0.0, 1.0], "{kind:?}");
    });
}

#[test]
fn a_panel_keeps_its_state_between_frames_and_reacts_to_events() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let frame = |events: Vec<UiEvent>| -> Vec<Widget> {
            let call = Call::Panel { panel: "counter".into(), input: UiInput { events, size: [300.0, 200.0], ..Default::default() } };
            loaded.plugin.call::<craft_plugin::PanelReply>(&call, None, &mut Recorder::default()).unwrap().widgets
        };
        let first = frame(vec![]);
        let (Widget::Button { id: button, .. }, Widget::Slider { id: slider, .. }) = (&first[0], &first[1]) else {
            panic!("{kind:?} unexpected widgets {first:?}")
        };
        frame(vec![UiEvent { id: *button, kind: UiEventKind::Click }]);
        frame(vec![UiEvent { id: *button, kind: UiEventKind::Click }, UiEvent { id: *slider, kind: UiEventKind::Number(4.0) }]);
        let last = frame(vec![]);
        assert!(matches!(&last[2], Widget::Label { text, .. } if text == "2 clicks at speed 4"), "{kind:?} {:?}", last[2]);
    });
}

fn panel_frame(loaded: &Loaded, panel: &str, retained: &mut Retained, now: Duration) -> bool {
    let Some(input) = retained.start([300.0, 200.0], now) else { return false };
    let call = Call::Panel { panel: panel.into(), input: input.clone() };
    let reply = loaded.plugin.call::<craft_plugin::PanelReply>(&call, None, &mut Recorder::default()).map_err(|e| e.to_string());
    retained.finish(reply, now);
    true
}

fn texts(widgets: &[Widget]) -> Vec<String> {
    widgets
        .iter()
        .filter_map(|widget| match widget {
            Widget::Label { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_retained_panel_is_not_called_while_nothing_happens_for_a_hundred_frames() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut counter = Retained::new();
        assert!(panel_frame(&loaded, "counter", &mut counter, Duration::ZERO));
        let called = (1..=100).filter(|frame| panel_frame(&loaded, "counter", &mut counter, Duration::from_millis(16 * frame))).count();
        assert_eq!(called, 0, "{kind:?}");
        let Widget::Button { id, .. } = counter.widgets()[0] else { panic!("{kind:?} button expected") };
        counter.push(vec![UiEvent { id, kind: UiEventKind::Click }], "");
        assert!(panel_frame(&loaded, "counter", &mut counter, Duration::from_millis(1700)));
        assert_eq!(texts(counter.widgets()), vec!["1 clicks at speed 0".to_string(), "calls 2".to_string()], "{kind:?}");
    });
}

#[test]
fn two_clicks_on_one_button_made_during_a_call_are_counted_twice() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut counter = Retained::new();
        panel_frame(&loaded, "counter", &mut counter, Duration::ZERO);
        let Widget::Button { id, .. } = counter.widgets()[0] else { panic!("{kind:?} button expected") };
        counter.invalidate();
        let slow = counter.start([300.0, 200.0], Duration::from_millis(16)).expect("an invalidated panel is called");
        counter.push(vec![UiEvent { id, kind: UiEventKind::Click }, UiEvent { id, kind: UiEventKind::Click }], "");
        let reply = loaded.plugin.call::<craft_plugin::PanelReply>(&Call::Panel { panel: "counter".into(), input: slow }, None, &mut Recorder::default());
        counter.finish(reply.map_err(|e| e.to_string()), Duration::from_millis(16));
        let called = (2..6).filter(|frame| panel_frame(&loaded, "counter", &mut counter, Duration::from_millis(16 * frame))).count();
        assert_eq!(called, 2, "{kind:?}");
        assert_eq!(texts(counter.widgets()), vec!["2 clicks at speed 0".to_string(), "calls 4".to_string()], "{kind:?}");
    });
}

#[test]
fn a_panel_timer_fires_ten_times_in_a_simulated_second_and_a_one_shot_once() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let mut ticker = Retained::new();
        for frame in 0..=60 {
            panel_frame(&loaded, "ticker", &mut ticker, Duration::from_secs_f64(f64::from(frame) / 60.0));
        }
        assert_eq!(texts(ticker.widgets()), vec!["ticks 10 once true".to_string()], "{kind:?}");
    });
}

#[test]
fn a_plugin_reads_and_writes_the_clipboard_only_with_the_clipboard_permission() {
    each_kind(|kind| {
        let mut host = Recorder { clipboard: "hello".into(), ..Default::default() };
        let refused = command(&load(kind, &[]), "swap-clipboard", &mut host).unwrap_err().to_string();
        assert!(refused.contains("does not have `clipboard`"), "{kind:?} {refused}");
        command(&load(kind, &["clipboard"]), "swap-clipboard", &mut host).unwrap();
        assert_eq!(host.clipboard, "probe saw hello", "{kind:?}");
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
    let stopped = loaded.plugin.render("spin", &cx(2, 2), &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap_err();
    assert!(matches!(stopped, PluginError::Trap(_)), "{stopped:?}");
    let mut pixels: Vec<Pixel> = vec![[0.25, 0.25, 0.25, 1.0]];
    let mut cx = cx(1, 1);
    cx.params.insert("amount".into(), craft_plugin::Value::Number(1.0));
    loaded.plugin.render("invert", &cx, &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap();
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
        pool.install(|| {
            loaded.plugin.render("shift-down", &cx(width as u32, height as u32), &Layout::neighbourhood(1), None, &mut rendered, Recorder::default)
        })
        .unwrap();
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
            loaded
                .plugin
                .render("invert", &cx, &Layout::neighbourhood(0), None, &mut none, Recorder::default)
                .unwrap_or_else(|e| panic!("{kind:?} {size:?}: {e}"));
        }
    });
}

#[test]
fn an_effect_whose_last_band_fails_leaves_the_frame_as_it_was() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        for (size, reach) in [([2, 2], None), ([3, 512], Some(0)), ([3, 512], Some(2))] {
            let source: Vec<Pixel> = (0..size[0] * size[1]).map(|i| [i as f32, 0.2, 0.3, 1.0]).collect();
            let mut pixels = source.clone();
            let failed = pool
                .install(|| {
                    loaded.plugin.render(
                        "half-done",
                        &cx(size[0], size[1]),
                        &reach.map_or(Layout::whole_frame(), Layout::neighbourhood),
                        None,
                        &mut pixels,
                        Recorder::default,
                    )
                })
                .unwrap_err();
            assert!(failed.to_string().contains("gave up half way"), "{kind:?} {reach:?}: {failed}");
            assert!(pixels == source, "{kind:?} {reach:?} changed the frame although a band failed");
        }
    });
}

#[test]
fn a_neighbourhood_effect_renders_in_bands_that_hold_their_reach() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]; 1024];
        pool.install(|| loaded.plugin.render("mark-band", &cx(1, 1024), &Layout::neighbourhood(2), None, &mut pixels, Recorder::default)).unwrap();
        assert_eq!([pixels[0], pixels[255]], [[0.0, 258.0, 0.0, 1.0], [0.0, 258.0, 255.0, 1.0]], "{kind:?} first band");
        assert_eq!([pixels[256], pixels[600]], [[254.0, 260.0, 256.0, 1.0], [510.0, 260.0, 600.0, 1.0]], "{kind:?} middle bands");
        assert_eq!(pixels[1023], [766.0, 258.0, 1023.0, 1.0], "{kind:?} last band");
    });
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

#[test]
fn time_spent_in_a_file_dialog_does_not_count_against_the_call_limit() {
    struct SlowPicker(PathBuf);
    impl Host for SlowPicker {
        fn pick_file(&mut self, _request: &craft_plugin::PickFile) -> Result<Option<PathBuf>, String> {
            std::thread::sleep(Duration::from_millis(900));
            Ok(Some(self.0.clone()))
        }
    }
    let outside = tempfile::tempdir().unwrap();
    let chosen = outside.path().join("chosen.txt");
    std::fs::write(&chosen, "chosen slowly").unwrap();
    let loaded = load(Kind::Wasm, &["files.write"]);
    loaded.plugin.call::<()>(&Call::Command { command: "pick-and-read".into() }, None, &mut SlowPicker(chosen.clone())).unwrap();
    assert_eq!(std::fs::read_to_string(loaded.data.path().join("data/picked.txt")).unwrap(), "chosen slowly");
    assert_eq!(std::fs::read_to_string(&chosen).unwrap(), "overwritten");
}

#[test]
fn a_cache_put_outside_a_render_changes_the_cache_generation_and_one_inside_does_not() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let start = loaded.plugin.cache_generation();
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]];
        loaded.plugin.render("memo", &cx(1, 1), &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap();
        assert_eq!(loaded.plugin.cached("memo").as_deref(), Some(&[1u8, 2, 3][..]), "{kind:?}");
        assert_eq!(loaded.plugin.cache_generation(), start, "{kind:?}");
        command(&loaded, "put-table", &mut Recorder::default()).unwrap();
        assert_eq!(loaded.plugin.cached("table").as_deref(), Some(&[4u8, 5, 6][..]), "{kind:?}");
        assert!(loaded.plugin.cache_generation() > start, "{kind:?}");
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
        let id = loaded.plugin.start("count", json!({"steps": 4}), &Recorder::default()).unwrap();
        let state = settle(&loaded, id, &mut Recorder::default());
        assert_eq!(state.status, TaskStatus::Done, "{kind:?} {state:?}");
        assert_eq!(state.result, Some(json!({"counted": 4})));
        assert_eq!((state.progress, state.message.as_str()), (1.0, "step 3"));

        command(&loaded, "start-count", &mut Recorder::default()).unwrap();
        let started: u64 = std::fs::read_to_string(loaded.data.path().join("data/task-id.txt")).unwrap().parse().unwrap();
        assert_eq!(started, id + 1);
        assert_eq!(settle(&loaded, started, &mut Recorder::default()).result, Some(json!({"counted": 3})));
        assert!(loaded.plugin.start("missing", json!({}), &Recorder::default()).unwrap_err().to_string().contains("no task `missing`"));
    });
}

#[test]
fn cancel_stops_a_task_that_never_reports_progress() {
    let loaded = load(Kind::Wasm, &[]);
    let id = loaded.plugin.start("forever", json!({}), &Recorder::default()).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(loaded.plugin.tasks()[0].status, TaskStatus::Running, "a task has no fixed time limit");
    let asked = std::time::Instant::now();
    assert!(loaded.plugin.cancel(id));
    let state = settle(&loaded, id, &mut Recorder::default());
    assert_eq!(state.status, TaskStatus::Cancelled);
    assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());

    let native = load(Kind::Native, &[]);
    let id = native.plugin.start("patient", json!({}), &Recorder::default()).unwrap();
    native.plugin.cancel(id);
    assert_eq!(settle(&native, id, &mut Recorder::default()).status, TaskStatus::Cancelled);
}

#[test]
fn a_task_edits_the_document_on_the_apps_thread_only_with_the_permission_for_it() {
    each_kind(|kind| {
        let denied = load(kind, &["document.read"]);
        let mut host = Recorder::default();
        let id = denied.plugin.start("edit", json!({}), &Recorder::default()).unwrap();
        let state = settle(&denied, id, &mut host);
        assert_eq!(state.status, TaskStatus::Failed, "{kind:?}");
        assert!(state.error.as_deref().unwrap_or_default().contains("document.write"), "{kind:?} {state:?}");
        assert!(host.commands.is_empty());

        let allowed = load(kind, &["document.write"]);
        let id = allowed.plugin.start("edit", json!({}), &Recorder::default()).unwrap();
        let state = settle(&allowed, id, &mut host);
        assert_eq!(state.result, Some(json!({"echo": {"name": "From Task"}})), "{kind:?}");
        assert_eq!(host.commands, ["layer.newNull"]);
    });
}

#[test]
fn a_plugin_runs_at_most_four_tasks_at_once_and_stopping_it_cancels_them() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let ids: Vec<u64> = (0..4).map(|_| loaded.plugin.start("patient", json!({}), &Recorder::default()).unwrap()).collect();
        let fifth = loaded.plugin.start("patient", json!({}), &Recorder::default()).unwrap_err();
        assert!(fifth.to_string().contains("already runs 4 tasks"), "{kind:?} {fifth}");
        loaded.plugin.cancel_all();
        let states: Vec<TaskStatus> = ids.iter().map(|id| settle(&loaded, *id, &mut Recorder::default()).status).collect();
        assert_eq!(states, [TaskStatus::Cancelled; 4], "{kind:?}");
        assert_eq!(loaded.plugin.running_tasks(), 0);
        let after = loaded.plugin.start("count", json!({"steps": 1}), &Recorder::default()).unwrap_err();
        assert!(after.to_string().contains("the plugin was stopped"), "{kind:?} {after}");
    });
}

#[test]
fn a_task_reads_a_frame_through_the_app_only_with_document_read() {
    each_kind(|kind| {
        let reader = load(kind, &["document.read"]);
        let id = reader.plugin.start("frame", json!({"time": 0.75, "scale": 0.5}), &Recorder::default()).unwrap();
        let state = settle(&reader, id, &mut Recorder::default());
        assert_eq!(state.result, Some(json!({"width": 2, "height": 1, "center": [0.75, 0.0, 0.0, 1.0]})), "{kind:?} {state:?}");

        let blind = load(kind, &[]);
        let id = blind.plugin.start("frame", json!({"time": 0.0}), &Recorder::default()).unwrap();
        let state = settle(&blind, id, &mut Recorder::default());
        assert!(state.error.as_deref().unwrap_or_default().contains("document.read"), "{kind:?} {state:?}");
    });
}

#[test]
fn an_effect_cannot_render_a_frame_from_inside_its_own_render() {
    each_kind(|kind| {
        let loaded = load(kind, &["document.read"]);
        let mut pixels: Vec<Pixel> = vec![[0.0; 4]];
        loaded.plugin.render("recurse", &cx(1, 1), &Layout::whole_frame(), None, &mut pixels, Recorder::default).unwrap();
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

        let id = loaded.plugin.start("pick", json!({}), &Recorder::default()).unwrap();
        let state = settle(&loaded, id, &mut host);
        assert!(state.error.as_deref().unwrap_or_default().contains("only from a panel or a command"), "{kind:?} {state:?}");
    });
}

#[test]
fn a_dropped_file_is_readable_like_a_picked_one_but_not_writable_and_its_folder_stays_closed() {
    each_kind(|kind| {
        let outside = tempfile::tempdir().unwrap();
        let (dropped, sibling) = (outside.path().join("dropped.txt"), outside.path().join("sibling.txt"));
        std::fs::write(&dropped, "dropped by the user").unwrap();
        std::fs::write(&sibling, "next to it").unwrap();
        let loaded = load(kind, &[]);
        let target = |path: &Path| std::fs::write(loaded._dir.path().join("user-target.txt"), path.to_str().unwrap()).unwrap();
        let mut host = Recorder::default();
        loaded.plugin.accept_drop(&dropped).unwrap();
        target(&dropped);
        command(&loaded, "read-user", &mut host).unwrap();
        assert_eq!(std::fs::read_to_string(loaded.data.path().join("data/read.txt")).unwrap(), "dropped by the user", "{kind:?}");
        let error = command(&loaded, "write-user", &mut host).unwrap_err().to_string();
        assert!(error.contains("files.write"), "{kind:?} {error}");
        target(&sibling);
        let error = command(&loaded, "read-user", &mut host).unwrap_err().to_string();
        assert!(error.contains("files.read"), "{kind:?} {error}");
        assert_eq!(std::fs::read_to_string(&dropped).unwrap(), "dropped by the user", "{kind:?}");
    });
}

#[cfg(unix)]
#[test]
fn a_picked_file_replaced_by_a_link_is_refused_and_a_new_save_target_is_not_followed() {
    each_kind(|kind| {
        let outside = tempfile::tempdir().unwrap();
        let (chosen, secret) = (outside.path().join("chosen.txt"), outside.path().join("secret.txt"));
        std::fs::write(&chosen, "chosen by the user").unwrap();
        std::fs::write(&secret, "not for the plugin").unwrap();
        let loaded = load(kind, &[]);
        let mut host = Recorder { picked: Some(chosen.clone()), ..Default::default() };
        command(&loaded, "pick-and-read", &mut host).unwrap_err();
        std::fs::write(loaded._dir.path().join("user-target.txt"), chosen.to_str().unwrap()).unwrap();
        command(&loaded, "read-user", &mut host).unwrap();
        assert_eq!(std::fs::read_to_string(loaded.data.path().join("data/read.txt")).unwrap(), "chosen by the user", "{kind:?}");

        std::fs::remove_file(&chosen).unwrap();
        std::os::unix::fs::symlink(&secret, &chosen).unwrap();
        let error = command(&loaded, "read-user", &mut host).unwrap_err().to_string();
        assert!(error.contains("is no longer the file the user picked"), "{kind:?} {error}");
        assert_eq!(std::fs::read_to_string(loaded.data.path().join("data/read.txt")).unwrap(), "chosen by the user", "{kind:?}");

        let target = outside.path().join("new.txt");
        let saver = load(kind, &[]);
        let mut host = Recorder { picked: Some(target.clone()), ..Default::default() };
        saver.plugin.call::<()>(&Call::Command { command: "pick-and-save".into() }, None, &mut host).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "saved", "{kind:?}");
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(&secret, &target).unwrap();
        std::fs::write(saver._dir.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
        let error = saver.plugin.call::<()>(&Call::Command { command: "write-user".into() }, None, &mut host).unwrap_err().to_string();
        assert!(error.contains("is no longer the file the user picked"), "{kind:?} {error}");
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "not for the plugin", "{kind:?}");
    });
}

#[cfg(unix)]
#[test]
fn a_new_save_target_is_created_in_the_folder_the_user_picked_even_if_that_folder_is_moved_and_replaced_by_a_link() {
    each_kind(|kind| {
        let outside = tempfile::tempdir().unwrap();
        let (chosen, moved, elsewhere) = (outside.path().join("chosen"), outside.path().join("moved"), outside.path().join("elsewhere"));
        std::fs::create_dir(&chosen).unwrap();
        std::fs::create_dir(&elsewhere).unwrap();
        let target = chosen.join("new.txt");
        let loaded = load(kind, &[]);
        let mut host = Recorder { picked: Some(target.clone()), ..Default::default() };
        command(&loaded, "pick-save-target", &mut host).unwrap();
        std::fs::rename(&chosen, &moved).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &chosen).unwrap();
        std::fs::write(loaded._dir.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
        command(&loaded, "write-user", &mut host).unwrap();
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0, "{kind:?}");
        assert_eq!(std::fs::read_to_string(moved.join("new.txt")).unwrap(), "written by plugin", "{kind:?}");
    });
}

#[test]
fn a_save_target_picked_inside_the_apps_folders_is_refused() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let planted = loaded.data.path().join("planted.json");
        let mut host = Recorder { picked: Some(planted.clone()), ..Default::default() };
        let error = command(&loaded, "pick-and-save", &mut host).unwrap_err().to_string();
        assert!(error.contains("that folder belongs to the app"), "{kind:?} {error}");
        assert!(!planted.exists(), "{kind:?}");
    });
}

#[cfg(unix)]
#[test]
fn a_sandboxed_call_whose_file_write_stalls_returns_only_after_the_write_landed() {
    let outside = tempfile::tempdir().unwrap();
    let stalled = outside.path().join("stalled");
    assert!(Command::new("mkfifo").arg(&stalled).status().unwrap().success());
    let loaded = load(Kind::Wasm, &["files.read", "files.write"]);
    std::fs::write(loaded._dir.path().join("user-target.txt"), stalled.to_str().unwrap()).unwrap();
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(700));
            let released = std::time::Instant::now();
            let mut written = String::new();
            std::io::Read::read_to_string(&mut std::fs::File::open(&stalled).unwrap(), &mut written).unwrap();
            (released, written)
        });
        let error = command(&loaded, "write-user", &mut Recorder::default()).unwrap_err().to_string();
        let returned = std::time::Instant::now();
        let (released, written) = reader.join().unwrap();
        assert!(error.contains("the plugin passed its time limit while the host served a call"), "{error}");
        assert!(returned > released, "the call returned {:?} before the disk let the write through", released - returned);
        assert_eq!(written, "written by plugin");
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
fn aux_reads_its_own_reach_across_band_edges() {
    let plan = |pass: GpuPass| GpuPlan::new().pass(pass);
    let tall_aux = GpuPass::new("probe_tall_aux", GpuImage::Source, GpuImage::Output).aux(GpuImage::Source);
    each_kind(|kind| {
        let banded = run_plan(kind, &plan(tall_aux.clone().reach(0).aux_reach(3))).unwrap();
        assert_eq!(banded, run_plan(kind, &plan(tall_aux.clone())).unwrap(), "{kind:?}");
        assert_eq!(at(&banded, 10, 75), [290.0, 2203.0, 71.0, 29.0], "{kind:?}");
    });
    let short = run_plan(Kind::Wasm, &plan(tall_aux.reach(0))).unwrap();
    assert_eq!(at(&short, 10, 75)[..2], [110.0, 815.0]);
}

#[test]
fn host_kernel_passes_run_in_the_app_between_plugin_passes() {
    each_kind(|kind| {
        let mut plan = GpuPlan::new();
        let tall = plan.scratch(PASS_FRAME);
        let swizzle = [[0.5, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let plan = plan.pass(GpuPass::new("probe_tall", GpuImage::Source, tall).reach(3)).pass(GpuPass::matrix(tall, GpuImage::Output, swizzle));
        assert_eq!(at(&run_plan(kind, &plan).unwrap(), 10, 75), [140.0, 28.0, 70.0, 28.0], "{kind:?}");
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
fn a_pass_that_writes_nothing_leaves_its_image_transparent() {
    each_kind(|kind| {
        let mut plan = GpuPlan::new();
        let scratch = plan.scratch(PASS_FRAME);
        let plan = plan.pass(GpuPass::new("probe_red", GpuImage::Source, scratch)).pass(GpuPass::new("probe_nothing", scratch, GpuImage::Output));
        let pixels = run_plan(kind, &plan).unwrap();
        assert_eq!(pixels, vec![[0.0; 4]; pixels.len()], "{kind:?}");
    });
}

#[test]
fn a_render_call_through_the_generic_entry_point_is_refused_and_writes_nothing() {
    each_kind(|kind| {
        let loaded = load(kind, &[]);
        let frame = FrameDesc { width: 2, height: 1, origin: [0, 0], ..Default::default() };
        let call = Call::Render { effect: "passes".into(), cx: cx(2, 1), frame };
        let mut pixels = vec![[0.25; 4]; 2];
        let error = loaded.plugin.call::<()>(&call, Some(&mut pixels), &mut Recorder::default()).unwrap_err();
        assert_eq!(error, PluginError::Failed("a render call stages its frame through Plugin::render; call render".into()), "{kind:?}");
        assert_eq!(pixels, vec![[0.25; 4]; 2], "{kind:?}");
    });
}

#[test]
fn a_failing_pass_fails_the_render() {
    each_kind(|kind| {
        let plan = GpuPlan::new().pass(GpuPass::new("nope", GpuImage::Source, GpuImage::Output).reach(0));
        assert_eq!(run_plan(kind, &plan), Err(PluginError::Failed("no probe kernel `nope`".into())));
    });
}
