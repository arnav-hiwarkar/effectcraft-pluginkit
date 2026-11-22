use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use craft_plugin::{
    AppEvent, Call, EffectSpec, FrameRequest, Layout, MaskPath, Overlay, PanelReply, ParamKind, ParamsAt, Pixel, RenderCx, Scene, TaskStatus, ToolEvent,
    ToolEventKind, UiEvent, UiEventKind, UiInput, Value, Widget,
};
pub use serde_json::json;

use crate::{Access, Bundle, Host, Limits, Pixels, Plugin};

pub const MAX_PADDING: u32 = 4096;
const TASK_WAIT: Duration = Duration::from_secs(60);

#[macro_export]
macro_rules! assert_frame {
    ($plugin:expr, $effect:expr, { $($params:tt)* }, $name:expr) => {
        $crate::testing::assert_frame($name, $plugin.render($effect).with($crate::testing::json!({ $($params)* })))
    };
    ($render:expr, $name:expr) => {
        $crate::testing::assert_frame($name, $render)
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Pixel>,
}

impl Picture {
    pub fn new(width: u32, height: u32) -> Picture {
        Picture { width, height, pixels: vec![[0.0; 4]; width as usize * height as usize] }
    }

    pub fn at(&self, x: u32, y: u32) -> Pixel {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    pub fn open(path: &Path) -> Result<Picture, String> {
        let image = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
        let pixels = image
            .pixels()
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                [f32::from(p[0]) / 255.0 * a, f32::from(p[1]) / 255.0 * a, f32::from(p[2]) / 255.0 * a, a]
            })
            .collect();
        Ok(Picture { width: image.width(), height: image.height(), pixels })
    }

    pub fn rgba8(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        self.pixels
            .iter()
            .flat_map(|p| {
                let a = p[3].clamp(0.0, 1.0);
                let straight = |c: f32| if a > 0.0 { byte(c / a) } else { 0 };
                [straight(p[0]), straight(p[1]), straight(p[2]), byte(a)]
            })
            .collect()
    }

    pub fn over_black(&self) -> Picture {
        Picture { pixels: self.pixels.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect(), ..*self }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        image::save_buffer(path, &self.rgba8(), self.width, self.height, image::ColorType::Rgba8).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn padded(&self, [left, top, right, bottom]: [u32; 4]) -> Picture {
        let mut out = Picture::new(self.width + left + right, self.height + top + bottom);
        let width = self.width as usize;
        for (y, row) in self.pixels.chunks_exact(width.max(1)).enumerate() {
            let at = (y + top as usize) * out.width as usize + left as usize;
            out.pixels[at..at + width].copy_from_slice(row);
        }
        out
    }

    fn pixels(&self) -> Pixels {
        Pixels { width: self.width, height: self.height, data: self.pixels.clone() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Photo,
    Gradient,
    Checker,
    Shapes,
    Text,
    Empty,
}

impl Pattern {
    pub const NAMES: [&str; 6] = ["photo", "gradient", "checker", "shapes", "text", "empty"];

    pub fn named(name: &str) -> Option<Pattern> {
        let all = [Pattern::Photo, Pattern::Gradient, Pattern::Checker, Pattern::Shapes, Pattern::Text, Pattern::Empty];
        Pattern::NAMES.iter().position(|known| *known == name).map(|at| all[at])
    }

    pub fn picture(self, width: u32, height: u32) -> Picture {
        let (w, h) = (width as f32, height as f32);
        let mut picture = Picture::new(width, height);
        if self == Pattern::Text {
            draw_text(&mut picture);
            return picture;
        }
        for (i, pixel) in picture.pixels.iter_mut().enumerate() {
            let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
            *pixel = match self {
                Pattern::Photo => photo(x / w, y / h, w / h),
                Pattern::Gradient => [x / w, y / h, 1.0 - x / w, 1.0],
                Pattern::Checker => {
                    let cell = (h / 8.0).max(1.0);
                    let on = ((x / cell) as u32 + (y / cell) as u32).is_multiple_of(2);
                    if on { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }
                }
                Pattern::Shapes => shapes(x, y, w, h),
                Pattern::Text | Pattern::Empty => [0.0; 4],
            };
        }
        picture
    }
}

fn photo(u: f32, v: f32, aspect: f32) -> Pixel {
    let sky = [0.25 + 0.35 * v, 0.45 + 0.3 * v, 0.95 - 0.25 * v];
    let sun = ((((u - 0.72) * aspect).powi(2) + (v - 0.28).powi(2)).sqrt() < 0.09) as u8 as f32;
    let ridge = 0.62 + 0.06 * (u * 9.0).sin() + 0.03 * (u * 23.0 + 1.0).sin();
    let near = 0.8 + 0.04 * (u * 5.0 + 2.0).cos();
    let rgb = if v > near {
        let shade = 0.55 + 0.25 * ((u * 40.0).sin() * (v * 31.0).cos()).abs();
        [0.18 * shade, 0.42 * shade, 0.12 * shade]
    } else if v > ridge {
        let depth = (v - ridge) / (near - ridge).max(1e-3);
        [0.32 - 0.1 * depth, 0.38 - 0.05 * depth, 0.46 - 0.2 * depth]
    } else {
        std::array::from_fn(|c| sky[c] + sun * ([1.0, 0.92, 0.7][c] - sky[c]))
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

fn shapes(x: f32, y: f32, w: f32, h: f32) -> Pixel {
    let disc = ((x - w * 0.3).powi(2) + (y - h * 0.45).powi(2)).sqrt() < h * 0.14;
    let bar = (x - w * 0.62).abs() < w * 0.012 && (y - h * 0.5).abs() < h * 0.3;
    let block = (x - w * 0.8).abs() < w * 0.06 && (y - h * 0.3).abs() < h * 0.08;
    let dim = (x - w * 0.8).abs() < w * 0.06 && (y - h * 0.7).abs() < h * 0.08;
    match (disc, bar, block, dim) {
        (true, ..) => [1.0, 1.0, 1.0, 1.0],
        (_, true, ..) => [0.1, 0.6, 1.0, 1.0],
        (_, _, true, _) => [1.0, 0.35, 0.05, 1.0],
        (_, _, _, true) => [0.25, 0.25, 0.25, 1.0],
        _ => [0.0; 4],
    }
}

fn draw_text(picture: &mut Picture) {
    let (width, height) = (picture.width, picture.height);
    let stroke = (height / 40).max(2) as i64;
    let glyphs: [&[(f32, f32, f32, f32)]; 3] = [
        &[(0.0, 0.0, 0.0, 1.0), (0.0, 0.0, 1.0, 0.0), (0.0, 0.5, 0.8, 0.5)],
        &[(0.0, 0.0, 1.0, 1.0), (1.0, 0.0, 0.0, 1.0)],
        &[(0.0, 0.0, 0.0, 1.0), (0.0, 1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (1.0, 0.0, 0.0, 0.0)],
    ];
    let (glyph_w, glyph_h) = (width as f32 * 0.16, height as f32 * 0.4);
    for (n, glyph) in glyphs.iter().enumerate() {
        let (left, top) = (width as f32 * (0.18 + 0.24 * n as f32), height as f32 * 0.3);
        for (x0, y0, x1, y1) in glyph.iter() {
            for step in 0..=400 {
                let t = step as f32 / 400.0;
                let (cx, cy) = (left + (x0 + (x1 - x0) * t) * glyph_w, top + (y0 + (y1 - y0) * t) * glyph_h);
                for dy in -stroke..=stroke {
                    for dx in -stroke..=stroke {
                        let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                        if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                            picture.pixels[(y as u32 * width + x as u32) as usize] = [1.0; 4];
                        }
                    }
                }
            }
        }
    }
}

pub fn pointer(kind: ToolEventKind, comp: [f64; 2]) -> ToolEvent {
    ToolEvent {
        kind,
        comp,
        layer: Some(comp),
        layer_id: None,
        modifiers: Default::default(),
        pressure: 1.0,
        scroll: [0.0; 2],
        key: String::new(),
        zoom: 1.0,
        time: 0.0,
    }
}

pub trait IntoValue {
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::Number(self)
    }
}

impl IntoValue for f32 {
    fn into_value(self) -> Value {
        Value::Number(f64::from(self))
    }
}

impl IntoValue for i32 {
    fn into_value(self) -> Value {
        Value::Number(f64::from(self))
    }
}

impl IntoValue for u32 {
    fn into_value(self) -> Value {
        Value::Number(f64::from(self))
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::Text(self.into())
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::Text(self)
    }
}

impl<const N: usize> IntoValue for [f64; N] {
    fn into_value(self) -> Value {
        Value::List(self.to_vec())
    }
}

impl<const N: usize> IntoValue for [f32; N] {
    fn into_value(self) -> Value {
        Value::List(self.iter().map(|v| f64::from(*v)).collect())
    }
}

pub fn value_of_json(value: &serde_json::Value) -> Option<Value> {
    Some(match value {
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64()?),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(serde_json::Value::as_f64).collect::<Option<_>>()?),
        serde_json::Value::Null | serde_json::Value::Object(_) => return None,
    })
}

pub fn default_value(kind: &ParamKind, layer_size: [f64; 2]) -> Option<Value> {
    let [w, h] = layer_size;
    Some(match kind {
        ParamKind::Slider { default, .. } | ParamKind::Percent { default, .. } | ParamKind::Angle { default } => Value::Number(*default),
        ParamKind::Checkbox { default } => Value::Bool(*default),
        ParamKind::Popup { default, .. } => Value::Number(f64::from(*default)),
        ParamKind::Point { default } => Value::List(vec![default[0] * w, default[1] * h]),
        ParamKind::Point3 { default } => Value::List(vec![default[0] * w, default[1] * h, default[2]]),
        ParamKind::Color { default } => Value::List(default.to_vec()),
        ParamKind::Text { default } | ParamKind::Custom { default } => Value::Text(default.clone()),
        ParamKind::Layer => Value::Bool(false),
        ParamKind::Mask => Value::Number(0.0),
    })
}

#[derive(Default)]
struct Recorder {
    commands: Vec<(String, serde_json::Value)>,
    answers: BTreeMap<String, serde_json::Value>,
    notes: Vec<String>,
    clipboard: String,
}

struct TestHost<'a> {
    recorder: &'a Mutex<Recorder>,
    source: Option<&'a Picture>,
    layers: Option<&'a BTreeMap<String, Picture>>,
    masks: &'a [MaskPath],
    params: Option<&'a BTreeMap<String, Value>>,
    layer_size: [f64; 2],
}

impl<'a> TestHost<'a> {
    fn bare(recorder: &'a Mutex<Recorder>) -> TestHost<'a> {
        TestHost { recorder, source: None, layers: None, masks: &[], params: None, layer_size: [0.0; 2] }
    }
}

impl Host for TestHost<'_> {
    fn access(&self, _command: &str, _params: &serde_json::Value) -> Access {
        Access::Free
    }

    fn command(&mut self, id: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut recorder = self.recorder.lock().map_err(|_| "the test host panicked earlier")?;
        recorder.commands.push((id.to_string(), params));
        Ok(recorder.answers.get(id).cloned().unwrap_or(serde_json::Value::Null))
    }

    fn notify(&mut self, message: &str) {
        if let Ok(mut recorder) = self.recorder.lock() {
            recorder.notes.push(message.to_string());
        }
    }

    fn layer(&mut self, param: &str, _comp_time: Option<f64>) -> Option<Pixels> {
        self.layers?.get(param).map(Picture::pixels)
    }

    fn source(&mut self, _layer_time: f64) -> Option<Pixels> {
        self.source.map(Picture::pixels)
    }

    fn masks(&mut self) -> Vec<MaskPath> {
        self.masks.to_vec()
    }

    fn frame(&mut self, request: &FrameRequest) -> Result<Pixels, String> {
        let scale = request.scale.clamp(1.0 / 64.0, 1.0);
        let [width, height] = [320.0, 180.0].map(|side: f64| (side * scale).round().max(1.0) as u32);
        Ok(Pattern::Photo.picture(width, height).pixels())
    }

    fn scene(&mut self) -> Option<Scene> {
        Some(Scene::centred(self.layer_size))
    }

    fn params_at(&mut self, _layer_time: f64) -> Option<ParamsAt> {
        Some(ParamsAt { params: self.params?.clone(), history: Some(0) })
    }

    fn clipboard(&mut self) -> Result<String, String> {
        self.recorder.lock().map(|recorder| recorder.clipboard.clone()).map_err(|_| "the test host panicked earlier".into())
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        self.recorder.lock().map(|mut recorder| recorder.clipboard = text.to_string()).map_err(|_| "the test host panicked earlier".into())
    }
}

pub struct TestPlugin {
    plugin: Plugin,
    recorder: Mutex<Recorder>,
    data: tempfile::TempDir,
}

fn manifest_dir() -> Result<PathBuf, String> {
    std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).ok_or_else(|| "CARGO_MANIFEST_DIR is not set; run the test with cargo test".into())
}

fn data_dir(id: &str) -> Result<tempfile::TempDir, String> {
    tempfile::Builder::new().prefix(&format!("craft-plugin-test-{id}-")).tempdir().map_err(|e| format!("no temporary data folder: {e}"))
}

impl TestPlugin {
    pub fn of(plugin: fn() -> craft_plugin::Plugin) -> TestPlugin {
        TestPlugin::in_process(plugin).unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn in_process(plugin: fn() -> craft_plugin::Plugin) -> Result<TestPlugin, String> {
        let dir = manifest_dir()?;
        let manifest = Bundle::manifest_of(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let data = data_dir(&manifest.id)?;
        let plugin = Plugin::in_process(manifest, plugin(), dir, data.path().join("data"), Limits::default()).map_err(|e| e.to_string())?;
        Ok(TestPlugin { plugin, recorder: Mutex::default(), data })
    }

    pub fn load(bundle: &Bundle) -> Result<TestPlugin, String> {
        let data = data_dir(&bundle.manifest.id)?;
        let plugin = Plugin::load(bundle, bundle.manifest.permissions.clone(), data.path().join("data"), Limits::default()).map_err(|e| e.to_string())?;
        Ok(TestPlugin { plugin, recorder: Mutex::default(), data })
    }

    pub fn plugin(&self) -> &Plugin {
        &self.plugin
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data.path().join("data")
    }

    pub fn render(&self, effect: &str) -> Render<'_> {
        Render {
            plugin: self,
            effect: effect.to_string(),
            set: Vec::new(),
            pattern: Pattern::Photo,
            size: [320, 180],
            input: None,
            layers: BTreeMap::new(),
            masks: Vec::new(),
            time: 0.0,
            scale: 1.0,
            float: false,
            tolerance: 1,
            error: None,
        }
    }

    pub fn answer(&self, command: &str, reply: serde_json::Value) {
        if let Ok(mut recorder) = self.recorder.lock() {
            recorder.answers.insert(command.to_string(), reply);
        }
    }

    pub fn commands(&self) -> Vec<(String, serde_json::Value)> {
        self.recorder.lock().map(|recorder| recorder.commands.clone()).unwrap_or_default()
    }

    pub fn notes(&self) -> Vec<String> {
        self.recorder.lock().map(|recorder| recorder.notes.clone()).unwrap_or_default()
    }

    pub fn logs(&self) -> Vec<String> {
        self.plugin.logs(0).into_iter().map(|line| line.message).collect()
    }

    pub fn command(&self, id: &str) -> Result<(), String> {
        self.plugin
            .call::<serde_json::Value>(&Call::Command { command: id.into() }, None, &mut TestHost::bare(&self.recorder))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn task(&self, id: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut host = TestHost::bare(&self.recorder);
        let started = self.plugin.start(id, args, &host).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + TASK_WAIT;
        loop {
            self.plugin.pump(&mut host);
            let state = self.plugin.tasks().into_iter().find(|state| state.id == started).ok_or(format!("task {started} is gone"))?;
            match state.status {
                TaskStatus::Done => return Ok(state.result.unwrap_or_default()),
                TaskStatus::Failed | TaskStatus::Cancelled => return Err(state.error.unwrap_or_else(|| format!("task `{id}` stopped"))),
                TaskStatus::Running if Instant::now() > deadline => return Err(format!("task `{id}` ran longer than {} s", TASK_WAIT.as_secs())),
                TaskStatus::Running => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    pub fn tool(&self, id: &str, event: ToolEvent) -> Result<Overlay, String> {
        self.plugin.call(&Call::Tool { tool: id.into(), event }, None, &mut TestHost::bare(&self.recorder)).map_err(|e| e.to_string())
    }

    pub fn panel(&self, id: &str) -> Result<PanelTest<'_>, String> {
        let mut panel = PanelTest { plugin: self, id: id.to_string(), reply: PanelReply::default() };
        panel.call(UiInput::default())?;
        Ok(panel)
    }

    fn spec(&self, effect: &str) -> Result<&EffectSpec, String> {
        let effects = &self.plugin.contributions().effects;
        effects.iter().find(|spec| spec.id == effect).ok_or_else(|| {
            let known: Vec<&str> = effects.iter().map(|spec| spec.id.as_str()).collect();
            format!("this plugin has no effect `{effect}`; its effects are {known:?}")
        })
    }
}

pub struct Render<'a> {
    plugin: &'a TestPlugin,
    effect: String,
    set: Vec<(String, Value)>,
    pattern: Pattern,
    size: [u32; 2],
    input: Option<Picture>,
    layers: BTreeMap<String, Picture>,
    masks: Vec<MaskPath>,
    time: f64,
    scale: f64,
    float: bool,
    tolerance: u8,
    error: Option<String>,
}

impl Render<'_> {
    pub fn set(mut self, id: &str, value: impl IntoValue) -> Self {
        self.set.push((id.to_string(), value.into_value()));
        self
    }

    pub fn with(mut self, params: serde_json::Value) -> Self {
        let Some(fields) = params.as_object() else {
            self.error = Some(format!("parameters must be a JSON object such as {{\"radius\": 20}}, not {params}"));
            return self;
        };
        for (id, value) in fields {
            match value_of_json(value) {
                Some(value) => self.set.push((id.clone(), value)),
                None => self.error = Some(format!("parameter `{id}`: {value} is not a number, bool, text or list of numbers")),
            }
        }
        self
    }

    pub fn on(mut self, pattern: Pattern) -> Self {
        self.pattern = pattern;
        self
    }

    pub fn input(mut self, picture: Picture) -> Self {
        self.input = Some(picture);
        self
    }

    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = [width, height];
        self
    }

    pub fn source(&self) -> Picture {
        self.input.clone().unwrap_or_else(|| self.pattern.picture(self.size[0], self.size[1]))
    }

    pub fn layer(mut self, param: &str, picture: Picture) -> Self {
        self.layers.insert(param.to_string(), picture);
        self
    }

    pub fn mask(mut self, points: &[[f64; 2]], closed: bool) -> Self {
        self.masks.push(MaskPath { name: format!("Mask {}", self.masks.len() + 1), points: points.to_vec(), closed, inverted: false });
        self
    }

    pub fn time(mut self, seconds: f64) -> Self {
        self.time = seconds;
        self
    }

    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    pub fn float(mut self) -> Self {
        self.float = true;
        self
    }

    pub fn tolerance(mut self, levels: u8) -> Self {
        self.tolerance = levels;
        self
    }

    fn cx(&self, spec: &EffectSpec, source: &Picture, size: [u32; 2], offset: [f64; 2]) -> Result<RenderCx, String> {
        let layer_size = [f64::from(source.width), f64::from(source.height)].map(|side| side / self.scale);
        let mut params: BTreeMap<String, Value> =
            spec.params.iter().filter_map(|param| Some((param.id.clone(), default_value(&param.kind, layer_size)?))).collect();
        for (id, value) in &self.set {
            let Some(param) = spec.params.iter().find(|param| param.id == *id) else {
                let known: Vec<&str> = spec.params.iter().map(|param| param.id.as_str()).collect();
                return Err(format!("effect `{}` has no parameter `{id}`; its parameters are {known:?}", spec.id));
            };
            params.insert(id.clone(), param.fit(value.clone())?);
        }
        for param in &spec.params {
            let value = params.get_mut(&param.id);
            match (&param.kind, value) {
                (ParamKind::Point { .. } | ParamKind::Point3 { .. }, Some(Value::List(point))) => {
                    point.iter_mut().zip(offset).for_each(|(c, by)| *c = *c * self.scale + by);
                    point.iter_mut().skip(2).take(1).for_each(|z| *z *= self.scale);
                }
                (ParamKind::Layer, Some(value)) => *value = Value::Bool(self.layers.contains_key(&param.id)),
                _ => {}
            }
        }
        Ok(RenderCx {
            params,
            time: self.time,
            comp_time: self.time,
            frame_rate: 30.0,
            scale: self.scale,
            layer_size,
            offset,
            size,
            instance: 1,
            float: self.float,
            linear: false,
        })
    }

    pub fn layout(&self) -> Result<Layout, String> {
        self.layout_on(&self.source())
    }

    fn layout_on(&self, source: &Picture) -> Result<Layout, String> {
        let spec = self.plugin.spec(&self.effect)?;
        let cx = self.cx(spec, source, [source.width, source.height], [0.0; 2])?;
        let mut host = self.host(source, &self.layers, &self.masks, &cx);
        let layout: Layout =
            self.plugin.plugin.call(&Call::Layout { effect: self.effect.clone(), cx: cx.clone() }, None, &mut host).map_err(|e| e.to_string())?;
        Ok(Layout { padding: layout.padding.map(|side| side.min(MAX_PADDING)), ..layout })
    }

    pub fn overlay(&self) -> Result<Overlay, String> {
        let source = self.source();
        let spec = self.plugin.spec(&self.effect)?;
        let cx = RenderCx { scale: 1.0, ..self.cx(spec, &source, [source.width, source.height], [0.0; 2])? };
        let mut host = self.host(&source, &self.layers, &self.masks, &cx);
        self.plugin.plugin.call(&Call::Overlay { effect: self.effect.clone(), cx: cx.clone() }, None, &mut host).map_err(|e| e.to_string())
    }

    fn host<'h>(&'h self, source: &'h Picture, layers: &'h BTreeMap<String, Picture>, masks: &'h [MaskPath], cx: &'h RenderCx) -> TestHost<'h> {
        TestHost { recorder: &self.plugin.recorder, source: Some(source), layers: Some(layers), masks, params: Some(&cx.params), layer_size: cx.layer_size }
    }

    pub fn run(&self) -> Result<Picture, String> {
        self.timed(1).map(|(picture, _)| picture)
    }

    pub fn timed(&self, rounds: u32) -> Result<(Picture, Duration), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let spec = self.plugin.spec(&self.effect)?;
        let unpadded = self.source();
        let layout = self.layout_on(&unpadded)?;
        let pad = layout.padding;
        let source = unpadded.padded(pad);
        let layers: BTreeMap<String, Picture> = self.layers.iter().map(|(id, picture)| (id.clone(), picture.padded(pad))).collect();
        let shift = [f64::from(pad[0]), f64::from(pad[1])];
        let masks: Vec<MaskPath> = self
            .masks
            .iter()
            .map(|mask| MaskPath { points: mask.points.iter().map(|p| [p[0] * self.scale + shift[0], p[1] * self.scale + shift[1]]).collect(), ..mask.clone() })
            .collect();
        let cx = self.cx(spec, &unpadded, [source.width, source.height], shift)?;
        let host = || self.host(&source, &layers, &masks, &cx);
        let plan = layout.gpu.as_ref().filter(|plan| layout.cpu && plan.runs_on_cpu());
        let render = |reach: Option<u32>| {
            let mut pixels = source.pixels.clone();
            let started = Instant::now();
            let layout = Layout { reach, ..layout.clone() };
            match plan {
                Some(plan) => self.plugin.plugin.run_passes(&self.effect, &cx, plan, &mut pixels, host),
                None => self.plugin.plugin.render(&self.effect, &cx, &layout, None, &mut pixels, host).map(|_| ()),
            }
            .map_err(|e| e.to_string())?;
            Ok::<_, String>((Picture { width: source.width, height: source.height, pixels }, started.elapsed()))
        };
        let (mut picture, mut best) = render(layout.reach)?;
        for _ in 1..rounds {
            let (again, took) = render(layout.reach)?;
            (picture, best) = (again, best.min(took));
        }
        if plan.is_none()
            && !layout.maps_input
            && let Some(reach) = layout.reach
        {
            let (whole, _) = render(None)?;
            if let Some(at) = picture.pixels.iter().zip(&whole.pixels).position(|(banded, whole)| banded != whole) {
                let (x, y) = (at as u32 % picture.width, at as u32 / picture.width);
                return Err(format!(
                    "effect `{}` declares that each row reads rows at most {reach} away, but rendering in bands changed pixel ({x}, {y}): \
                     return Layout::neighbourhood with the real distance from layout(), or Layout::whole_frame()",
                    self.effect
                ));
            }
        }
        Ok((picture, best))
    }
}

pub struct PanelTest<'a> {
    plugin: &'a TestPlugin,
    id: String,
    reply: PanelReply,
}

impl PanelTest<'_> {
    fn call(&mut self, input: UiInput) -> Result<(), String> {
        let input = UiInput { size: [320.0, 480.0], ..input };
        let call = Call::Panel { panel: self.id.clone(), input };
        self.reply = self.plugin.plugin.call(&call, None, &mut TestHost::bare(&self.plugin.recorder)).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn widgets(&self) -> &[Widget] {
        &self.reply.widgets
    }

    pub fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.reply.widgets, &mut |widget| {
            if let Some((_, text)) = labelled(widget) {
                out.push(text.to_string());
            } else if let Widget::Label { text, .. } | Widget::Progress { text, .. } = widget {
                out.push(text.clone());
            }
        });
        out
    }

    pub fn send(&mut self, label: &str, kind: UiEventKind) -> Result<(), String> {
        let mut found = None;
        walk(&self.reply.widgets, &mut |widget| {
            if found.is_none()
                && let Some((id, text)) = labelled(widget)
                && text == label
            {
                found = Some(id);
            }
        });
        let id = found.ok_or_else(|| format!("panel `{}` shows no widget labelled `{label}`; it shows {:?}", self.id, self.texts()))?;
        self.call(UiInput { events: vec![UiEvent { id, kind }], ..Default::default() })
    }

    pub fn click(&mut self, label: &str) -> Result<(), String> {
        self.send(label, UiEventKind::Click)
    }

    pub fn app_event(&mut self, event: AppEvent) -> Result<(), String> {
        self.call(UiInput { app: vec![event], ..Default::default() })
    }

    pub fn timer(&mut self, name: &str) -> Result<(), String> {
        self.call(UiInput { timers: vec![name.to_string()], ..Default::default() })
    }

    pub fn redraw(&mut self) -> Result<(), String> {
        self.call(UiInput::default())
    }
}

fn labelled(widget: &Widget) -> Option<(u64, &str)> {
    match widget {
        Widget::Button { id, text, .. }
        | Widget::Checkbox { id, text, .. }
        | Widget::Slider { id, text, .. }
        | Widget::Number { id, text, .. }
        | Widget::Popup { id, text, .. }
        | Widget::Color { id, text, .. } => Some((*id, text)),
        Widget::TextEdit { id, hint, .. } => Some((*id, hint)),
        _ => None,
    }
}

fn walk(widgets: &[Widget], visit: &mut impl FnMut(&Widget)) {
    for widget in widgets {
        visit(widget);
        match widget {
            Widget::Row { children }
            | Widget::Column { children }
            | Widget::Group { children, .. }
            | Widget::Scroll { children, .. }
            | Widget::Collapsing { children, .. }
            | Widget::Grid { children, .. }
            | Widget::DropTarget { children, .. }
            | Widget::DragSource { children, .. } => walk(children, visit),
            _ => {}
        }
    }
}

pub struct Difference {
    pub differing: usize,
    pub largest: u8,
    pub first: Option<(u32, u32)>,
    pub image: Vec<u8>,
}

pub fn difference(expected: &[u8], actual: &[u8], width: u32, tolerance: u8) -> Difference {
    let mut out = Difference { differing: 0, largest: 0, first: None, image: Vec::with_capacity(expected.len()) };
    for (at, (want, got)) in expected.chunks_exact(4).zip(actual.chunks_exact(4)).enumerate() {
        let gap = want.iter().zip(got).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        out.largest = out.largest.max(gap);
        if gap > tolerance {
            out.differing += 1;
            out.first.get_or_insert((at as u32 % width.max(1), at as u32 / width.max(1)));
            out.image.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            let grey = ((u16::from(want[0]) + u16::from(want[1]) + u16::from(want[2])) / 12) as u8;
            out.image.extend_from_slice(&[grey, grey, grey, 255]);
        }
    }
    out
}

#[track_caller]
pub fn assert_frame(name: &str, render: Render) {
    if let Err(message) = manifest_dir().and_then(|dir| check_frame(&dir, name, &render)) {
        panic!("{message}");
    }
}

pub fn updating_frames(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

fn plain_file(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(found) if found.file_type().is_symlink() => Err(format!("{} is a link; stored frames must be plain files", path.display())),
        _ => Ok(()),
    }
}

pub fn check_frame(dir: &Path, name: &str, render: &Render) -> Result<(), String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("frame name `{name}` must be a plain name of letters, digits, `-` and `_`, such as `full`"));
    }
    let folder = dir.join("tests/frames");
    let expected_path = folder.join(format!("{name}.png"));
    let failures = dir.join("target/frames");
    let actual_path = failures.join(format!("{name}.actual.png"));
    let diff_path = failures.join(format!("{name}.diff.png"));
    for path in [&dir.join("tests"), &folder, &expected_path] {
        plain_file(path)?;
    }
    let actual = render.run().map_err(|e| format!("frame `{name}`: {e}"))?;
    let missing = !expected_path.is_file();
    if updating_frames(std::env::var_os("UPDATE_FRAMES").as_deref()) || (missing && std::env::var_os("CI").is_none()) {
        actual.save(&expected_path)?;
        let _ = std::fs::remove_file(&actual_path);
        let _ = std::fs::remove_file(&diff_path);
        eprintln!("stored frame `{name}` at {}; look at it and commit it", expected_path.display());
        return Ok(());
    }
    if missing {
        actual.save(&actual_path)?;
        return Err(format!(
            "no stored frame {}; the rendered frame is at {}. Run with UPDATE_FRAMES=1 to store it",
            expected_path.display(),
            actual_path.display()
        ));
    }
    let expected = image::open(&expected_path).map_err(|e| format!("{}: {e}", expected_path.display()))?.to_rgba8();
    if [expected.width(), expected.height()] != [actual.width, actual.height] {
        actual.save(&actual_path)?;
        return Err(format!(
            "frame `{name}` is {}x{} but {} is {}x{}; the new frame is at {}. If the new size is right, run with UPDATE_FRAMES=1",
            actual.width,
            actual.height,
            expected_path.display(),
            expected.width(),
            expected.height(),
            actual_path.display()
        ));
    }
    let found = difference(&expected, &actual.rgba8(), actual.width, render.tolerance);
    if found.differing == 0 {
        let _ = std::fs::remove_file(&actual_path);
        let _ = std::fs::remove_file(&diff_path);
        return Ok(());
    }
    actual.save(&actual_path)?;
    image::save_buffer(&diff_path, &found.image, actual.width, actual.height, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    let (x, y) = found.first.unwrap_or_default();
    Err(format!(
        "frame `{name}` differs from the stored frame: {} of {} pixels are off by more than {}/255 (largest {}/255, first at ({x}, {y}))\n  \
         expected:   {}\n  actual:     {}\n  difference: {} (red marks the pixels that differ)\n\
         If the new frame is right, run again with UPDATE_FRAMES=1.",
        found.differing,
        actual.pixels.len(),
        render.tolerance,
        found.largest,
        expected_path.display(),
        actual_path.display(),
        diff_path.display()
    ))
}
