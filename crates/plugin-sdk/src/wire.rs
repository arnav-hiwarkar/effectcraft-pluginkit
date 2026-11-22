use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

pub const ABI: u32 = 1;
pub const PIXEL_BYTES: usize = 16;

pub type Pixel = [f32; 4];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Describe,
    Layout { effect: String, cx: RenderCx },
    Render { effect: String, cx: RenderCx, frame: FrameDesc },
    Panel { panel: String, input: UiInput },
    Command { command: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Outcome<T> {
    Ok(T),
    Err(String),
}

impl<T> From<Result<T, String>> for Outcome<T> {
    fn from(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Outcome::Ok(value),
            Err(message) => Outcome::Err(message),
        }
    }
}

impl<T> From<Outcome<T>> for Result<T, String> {
    fn from(outcome: Outcome<T>) -> Self {
        match outcome {
            Outcome::Ok(value) => Ok(value),
            Outcome::Err(message) => Err(message),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Contributions {
    #[serde(default)]
    pub effects: Vec<EffectSpec>,
    #[serde(default)]
    pub panels: Vec<PanelSpec>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EffectSpec {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub time_dependent: bool,
    #[serde(default)]
    pub reads_other_frames: bool,
    #[serde(default)]
    pub reads_scene: bool,
    #[serde(default)]
    pub shader: Option<String>,
    #[serde(default)]
    pub designer: Option<String>,
}

impl EffectSpec {
    pub fn new(id: &str, name: &str, category: &str) -> Self {
        EffectSpec { id: id.into(), name: name.into(), category: category.into(), ..Default::default() }
    }

    pub fn param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    pub fn group(mut self, group: &str, params: impl IntoIterator<Item = Param>) -> Self {
        self.params.extend(params.into_iter().map(|mut p| {
            p.id = format!("{group}/{}", p.id);
            p
        }));
        self
    }

    pub fn time_dependent(mut self) -> Self {
        self.time_dependent = true;
        self
    }

    pub fn reads_other_frames(mut self) -> Self {
        self.reads_other_frames = true;
        self
    }

    pub fn reads_scene(mut self) -> Self {
        self.reads_scene = true;
        self
    }

    pub fn shader(mut self, wgsl: &str) -> Self {
        self.shader = Some(wgsl.into());
        self
    }

    pub fn designer(mut self, panel: &str) -> Self {
        self.designer = Some(panel.into());
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: ParamKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamKind {
    Slider { default: f64, min: f64, max: f64 },
    Percent { default: f64 },
    Angle { default: f64 },
    Checkbox { default: bool },
    Popup { options: Vec<String>, default: u32 },
    Point { default: [f64; 2] },
    Point3 { default: [f64; 3] },
    Color { default: [f64; 4] },
    Text { default: String },
    Layer,
    Mask,
}

impl Param {
    fn new(id: &str, name: &str, kind: ParamKind) -> Self {
        Param { id: id.into(), name: name.into(), kind }
    }

    pub fn slider(id: &str, name: &str, default: f64, range: RangeInclusive<f64>) -> Self {
        Param::new(id, name, ParamKind::Slider { default, min: *range.start(), max: *range.end() })
    }

    pub fn percent(id: &str, name: &str, default: f64) -> Self {
        Param::new(id, name, ParamKind::Percent { default })
    }

    pub fn angle(id: &str, name: &str, default: f64) -> Self {
        Param::new(id, name, ParamKind::Angle { default })
    }

    pub fn checkbox(id: &str, name: &str, default: bool) -> Self {
        Param::new(id, name, ParamKind::Checkbox { default })
    }

    pub fn popup(id: &str, name: &str, options: &[&str], default: u32) -> Self {
        let options = options.iter().map(|o| o.to_string()).collect();
        Param::new(id, name, ParamKind::Popup { options, default })
    }

    pub fn point(id: &str, name: &str, default: [f64; 2]) -> Self {
        Param::new(id, name, ParamKind::Point { default })
    }

    pub fn point3(id: &str, name: &str, default: [f64; 3]) -> Self {
        Param::new(id, name, ParamKind::Point3 { default })
    }

    pub fn color(id: &str, name: &str, default: [f64; 4]) -> Self {
        Param::new(id, name, ParamKind::Color { default })
    }

    pub fn text(id: &str, name: &str, default: &str) -> Self {
        Param::new(id, name, ParamKind::Text { default: default.into() })
    }

    pub fn layer(id: &str, name: &str) -> Self {
        Param::new(id, name, ParamKind::Layer)
    }

    pub fn mask(id: &str, name: &str) -> Self {
        Param::new(id, name, ParamKind::Mask)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Number(f64),
    List(Vec<f64>),
    Text(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RenderCx {
    pub params: BTreeMap<String, Value>,
    pub time: f64,
    pub comp_time: f64,
    pub frame_rate: f64,
    pub scale: f64,
    pub layer_size: [f64; 2],
    pub offset: [f64; 2],
    pub size: [u32; 2],
    pub instance: u64,
    #[serde(default)]
    pub float: bool,
    #[serde(default)]
    pub linear: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameDesc {
    pub width: u32,
    pub height: u32,
    pub origin: [u32; 2],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Layout {
    #[serde(default)]
    pub padding: u32,
    #[serde(default)]
    pub reach: Option<u32>,
    #[serde(default)]
    pub gpu: Option<GpuPlan>,
}

impl Layout {
    pub fn whole_frame() -> Self {
        Layout::default()
    }

    pub fn pointwise() -> Self {
        Layout { reach: Some(0), ..Default::default() }
    }

    pub fn neighbourhood(reach: u32) -> Self {
        Layout { reach: Some(reach), ..Default::default() }
    }

    pub fn padded(mut self, padding: u32) -> Self {
        self.padding = padding;
        self
    }

    pub fn gpu(mut self, plan: GpuPlan) -> Self {
        self.gpu = Some(plan);
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct GpuPlan {
    #[serde(default)]
    pub images: Vec<[u32; 2]>,
    pub passes: Vec<GpuPass>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GpuPass {
    pub kernel: String,
    pub src: GpuImage,
    #[serde(default)]
    pub aux: Option<GpuImage>,
    pub out: GpuImage,
    #[serde(default)]
    pub f: Vec<f32>,
    #[serde(default)]
    pub u: Vec<u32>,
}

impl GpuPlan {
    pub fn new() -> Self {
        GpuPlan::default()
    }

    pub fn scratch(&mut self, size: [u32; 2]) -> GpuImage {
        self.images.push(size);
        GpuImage::Scratch(self.images.len().saturating_sub(1) as u32)
    }

    pub fn pass(mut self, pass: GpuPass) -> Self {
        self.passes.push(pass);
        self
    }
}

impl GpuPass {
    pub fn new(kernel: &str, src: GpuImage, out: GpuImage) -> Self {
        GpuPass { kernel: kernel.to_string(), src, aux: None, out, f: Vec::new(), u: Vec::new() }
    }

    pub fn aux(mut self, image: GpuImage) -> Self {
        self.aux = Some(image);
        self
    }

    pub fn floats(mut self, values: &[f32]) -> Self {
        self.f = values.to_vec();
        self
    }

    pub fn uints(mut self, values: &[u32]) -> Self {
        self.u = values.to_vec();
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuImage {
    Source,
    Output,
    Scratch(u32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PanelSpec {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CommandSpec {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum HostCall {
    Log { level: LogLevel, message: String },
    Notify { message: String },
    Command { id: String, params: serde_json::Value },
    Layer { param: String, time: Option<f64> },
    Source { time: f64 },
    Masks,
    Scene,
    ReadFile { root: Root, path: String },
    WriteFile { root: Root, path: String },
    ListFiles { root: Root, path: String },
    CacheGet { key: String },
    CachePut { key: String },
    CacheRemove { key: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Root {
    Bundle,
    Data,
    User,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageDesc {
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MaskPath {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub inverted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Scene {
    #[serde(default)]
    pub projection: Option<[[f64; 4]; 3]>,
    #[serde(default)]
    pub camera_layer: bool,
}

impl Scene {
    pub fn project(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        let m = self.projection?;
        let row = |r: [f64; 4]| r[0] * point[0] + r[1] * point[1] + r[2] * point[2] + r[3];
        let w = row(m[2]);
        (w > 1e-9).then(|| [row(m[0]) / w, row(m[1]) / w])
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UiInput {
    #[serde(default)]
    pub events: Vec<UiEvent>,
    pub size: [f32; 2],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UiEvent {
    pub id: u64,
    pub kind: UiEventKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UiEventKind {
    Click,
    Bool(bool),
    Number(f64),
    Text(String),
    Color([f32; 4]),
    Pointer(Pointer),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub pos: [f32; 2],
    pub phase: PointerPhase,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    Hover,
    Down,
    Drag,
    Up,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "w", rename_all = "snake_case")]
pub enum Widget {
    Label { text: String, style: TextStyle },
    Button { id: u64, text: String, enabled: bool },
    Checkbox { id: u64, text: String, value: bool },
    Slider { id: u64, text: String, value: f64, min: f64, max: f64 },
    Number { id: u64, text: String, value: f64, speed: f64 },
    TextEdit { id: u64, value: String, hint: String },
    Popup { id: u64, text: String, options: Vec<String>, selected: u32 },
    Color { id: u64, text: String, value: [f32; 4] },
    Row { children: Vec<Widget> },
    Column { children: Vec<Widget> },
    Group { title: String, children: Vec<Widget> },
    Separator,
    Space { size: f32 },
    Canvas { id: u64, size: [f32; 2], shapes: Vec<Shape> },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    Body,
    Heading,
    Weak,
    Mono,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "s", rename_all = "snake_case")]
pub enum Shape {
    Line { points: Vec<[f32; 2]>, color: Paint, width: f32 },
    Bezier { points: [[f32; 2]; 4], color: Paint, width: f32 },
    Rect { min: [f32; 2], max: [f32; 2], fill: Option<Paint>, stroke: Option<(Paint, f32)>, radius: f32 },
    Circle { center: [f32; 2], radius: f32, fill: Option<Paint>, stroke: Option<(Paint, f32)> },
    Text { pos: [f32; 2], text: String, color: Paint, size: f32 },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Paint {
    Text,
    Weak,
    Accent,
    Background,
    Surface,
    Border,
    Error,
    Rgba([u8; 4]),
}
