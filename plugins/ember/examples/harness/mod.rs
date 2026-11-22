use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use craft_plugin_host::{Bundle, Call, Host, Layout, Limits, MaskPath, Pixel, Pixels, Plugin, RenderCx, Scene, Value};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Wasm,
    Native,
}

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

pub fn bundle(kind: Kind) -> PathBuf {
    let root = Path::new(MANIFEST);
    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "--release", "--quiet", "--manifest-path"]).arg(root.join("Cargo.toml"));
    if kind == Kind::Wasm {
        build.args(["--target", "wasm32-unknown-unknown"]);
    }
    assert!(build.status().expect("cargo runs").success(), "Ember builds");
    let name = if kind == Kind::Wasm { "wasm" } else { "native" };
    let dir = std::env::temp_dir().join(format!("ember-bundle-{}-{name}", std::process::id())).join("org.craftkit.ember");
    std::fs::create_dir_all(&dir).expect("bundle dir");
    let manifest = std::fs::read_to_string(root.join("plugin.json")).expect("manifest");
    match kind {
        Kind::Wasm => {
            std::fs::copy(root.join("target/wasm32-unknown-unknown/release/ember.wasm"), dir.join("plugin.wasm")).expect("copy the wasm");
            std::fs::write(dir.join("plugin.json"), manifest).expect("write the manifest");
        }
        Kind::Native => {
            let library = format!("{}ember{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
            std::fs::copy(root.join("target/release").join(&library), dir.join(&library)).expect("copy the library");
            let entry = format!(r#""entry": {{ "native": {{ "{}": "{library}" }} }}"#, craft_plugin::native_target());
            std::fs::write(dir.join("plugin.json"), manifest.replace(r#""entry": { "wasm": "plugin.wasm" }"#, &entry)).expect("write the manifest");
        }
    }
    dir
}

pub fn camera(size: [f64; 2], turn: f64, zoom: f64) -> Scene {
    let (sin, cos) = turn.sin_cos();
    let centre = [size[0] / 2.0, size[1] / 2.0];
    let w = [-sin, 0.0, cos, sin * centre[0] + zoom];
    let x = [zoom * cos, 0.0, zoom * sin, -zoom * cos * centre[0]];
    let y = [0.0, zoom, 0.0, -zoom * centre[1]];
    let add = |a: [f64; 4], k: f64| std::array::from_fn(|i| a[i] + k * w[i]);
    Scene { projection: Some([add(x, centre[0]), add(y, centre[1]), w]), camera_layer: true, ..Default::default() }
}

#[derive(Clone)]
pub struct Scenario {
    pub size: [u32; 2],
    pub params: BTreeMap<String, Value>,
    pub masks: Vec<MaskPath>,
    pub scene: Option<Scene>,
}

struct Inputs<'a>(&'a Scenario);

impl Host for Inputs<'_> {
    fn masks(&mut self) -> Vec<MaskPath> {
        self.0.masks.clone()
    }

    fn scene(&mut self) -> Option<Scene> {
        Some(self.0.scene.clone().unwrap_or_default())
    }

    fn source(&mut self, _layer_time: f64) -> Option<Pixels> {
        None
    }
}

pub struct Loaded<'a> {
    scenario: &'a Scenario,
    plugin: Plugin,
}

impl Scenario {
    pub fn new(size: [u32; 2]) -> Scenario {
        Scenario { size, params: BTreeMap::new(), masks: Vec::new(), scene: None }
    }

    pub fn param(mut self, id: &str, value: f64) -> Scenario {
        self.params.insert(id.into(), Value::Number(value));
        self
    }

    pub fn list(mut self, id: &str, value: &[f64]) -> Scenario {
        self.params.insert(id.into(), Value::List(value.to_vec()));
        self
    }

    pub fn scene(mut self, scene: Scene) -> Scenario {
        self.scene = Some(scene);
        self
    }

    pub fn load(&self, bundle: &Path) -> Loaded<'_> {
        let bundle = Bundle::open(bundle).expect("bundle opens");
        let data = std::env::temp_dir().join(format!("ember-data-{}", std::process::id()));
        let plugin = Plugin::load(&bundle, bundle.manifest.permissions.clone(), data, Limits::default()).expect("Ember loads");
        Loaded { scenario: self, plugin }
    }
}

impl Loaded<'_> {
    fn cx(&self, frame: u32) -> RenderCx {
        let [w, h] = self.scenario.size;
        let spec = self.plugin.contributions().effects.iter().find(|e| e.id == "ember").expect("the ember effect");
        let defaults = spec.params.iter().filter_map(|p| {
            use craft_plugin_host::ParamKind as K;
            let value = match &p.kind {
                K::Slider { default, .. } | K::Percent { default } | K::Angle { default } => Value::Number(*default),
                K::Checkbox { default } => Value::Bool(*default),
                K::Popup { default, .. } => Value::Number(f64::from(*default)),
                K::Point3 { default } => Value::List(vec![default[0] * f64::from(w), default[1] * f64::from(h), default[2]]),
                K::Color { default } => Value::List(default.to_vec()),
                _ => return None,
            };
            Some((p.id.clone(), value))
        });
        RenderCx {
            params: defaults.chain(self.scenario.params.clone()).collect(),
            time: f64::from(frame) / 30.0,
            comp_time: f64::from(frame) / 30.0,
            frame_rate: 30.0,
            scale: 1.0,
            layer_size: [f64::from(w), f64::from(h)],
            size: [w, h],
            instance: 1,
            ..Default::default()
        }
    }

    pub fn timed(&self, frame: u32) -> (Vec<Pixel>, [f64; 2]) {
        let cx = self.cx(frame);
        let start = Instant::now();
        let layout: Layout = self.plugin.call(&Call::Layout { effect: "ember".into(), cx: cx.clone() }, None, &mut Inputs(self.scenario)).expect("layout");
        let layout_ms = start.elapsed().as_secs_f64() * 1000.0;
        let [w, h] = self.scenario.size;
        let mut pixels = vec![[0.02, 0.02, 0.05, 1.0]; (w * h) as usize];
        let start = Instant::now();
        self.plugin.render("ember", &cx, layout.reach, &mut pixels, || Inputs(self.scenario)).expect("render");
        (pixels, [layout_ms, start.elapsed().as_secs_f64() * 1000.0])
    }

    pub fn frame(&self, frame: u32) -> Vec<Pixel> {
        self.timed(frame).0
    }
}
