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
    Task { task: String, args: serde_json::Value },
    Controls { effect: String, param: String, value: String, input: UiInput },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ControlsReply {
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub value: Option<String>,
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
    #[serde(default)]
    pub tasks: Vec<TaskSpec>,
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
    #[serde(default)]
    pub reads_param_history: bool,
    #[serde(default)]
    pub presets: Vec<Preset>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub name: String,
    pub values: BTreeMap<String, Value>,
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

    pub fn reads_param_history(mut self) -> Self {
        self.reads_param_history = true;
        self
    }

    pub fn preset<'a>(mut self, name: &str, values: impl IntoIterator<Item = (&'a str, Value)>) -> Self {
        let values = values.into_iter().map(|(id, value)| (id.to_string(), value)).collect();
        self.presets.push(Preset { name: name.into(), values });
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
    Custom { default: String },
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

    pub fn custom(id: &str, name: &str, default: &str) -> Self {
        Param::new(id, name, ParamKind::Custom { default: default.into() })
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
    #[serde(default)]
    pub buffers: Vec<GpuData>,
    #[serde(default)]
    pub textures: Vec<GpuTexture>,
    #[serde(default)]
    pub layers: Vec<String>,
    pub passes: Vec<GpuPass>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GpuData {
    Cache(String),
    Floats(Vec<f32>),
    Indices(Vec<u32>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GpuTexture {
    pub key: String,
    pub size: [u32; 2],
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
    #[serde(default)]
    pub draw: Option<Draw>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Draw {
    pub vertices: u32,
    pub layout: Vec<u32>,
    #[serde(default)]
    pub indices: Option<u32>,
    #[serde(default)]
    pub instances: Option<u32>,
    #[serde(default)]
    pub instance_layout: Vec<u32>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub depth: bool,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default)]
    pub cull: Cull,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    Replace,
    Over,
    Add,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Cull {
    #[default]
    None,
    Back,
    Front,
}

impl GpuPlan {
    pub fn new() -> Self {
        GpuPlan::default()
    }

    pub fn scratch(&mut self, size: [u32; 2]) -> GpuImage {
        self.images.push(size);
        GpuImage::Scratch(self.images.len().saturating_sub(1) as u32)
    }

    pub fn buffer(&mut self, data: GpuData) -> u32 {
        self.buffers.push(data);
        self.buffers.len().saturating_sub(1) as u32
    }

    pub fn texture(&mut self, key: &str, size: [u32; 2]) -> GpuImage {
        self.textures.push(GpuTexture { key: key.to_string(), size });
        GpuImage::Texture(self.textures.len().saturating_sub(1) as u32)
    }

    pub fn layer(&mut self, param: &str) -> GpuImage {
        self.layers.push(param.to_string());
        GpuImage::Layer(self.layers.len().saturating_sub(1) as u32)
    }

    pub fn pass(mut self, pass: GpuPass) -> Self {
        self.passes.push(pass);
        self
    }
}

impl GpuPass {
    pub fn new(kernel: &str, src: GpuImage, out: GpuImage) -> Self {
        GpuPass { kernel: kernel.to_string(), src, aux: None, out, f: Vec::new(), u: Vec::new(), draw: None }
    }

    pub fn draw(kernel: &str, out: GpuImage) -> Self {
        GpuPass { draw: Some(Draw::default()), ..GpuPass::new(kernel, GpuImage::Source, out) }
    }

    pub fn src(mut self, image: GpuImage) -> Self {
        self.src = image;
        self
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

    fn drawing(mut self, set: impl FnOnce(&mut Draw)) -> Self {
        set(self.draw.get_or_insert_with(Draw::default));
        self
    }

    pub fn vertices(self, buffer: u32, layout: &[u32]) -> Self {
        self.drawing(|draw| (draw.vertices, draw.layout) = (buffer, layout.to_vec()))
    }

    pub fn indices(self, buffer: u32) -> Self {
        self.drawing(|draw| draw.indices = Some(buffer))
    }

    pub fn instances(self, buffer: u32, layout: &[u32]) -> Self {
        self.drawing(|draw| (draw.instances, draw.instance_layout) = (Some(buffer), layout.to_vec()))
    }

    pub fn count(self, count: u32) -> Self {
        self.drawing(|draw| draw.count = Some(count))
    }

    pub fn depth(self) -> Self {
        self.drawing(|draw| draw.depth = true)
    }

    pub fn blend(self, blend: Blend) -> Self {
        self.drawing(|draw| draw.blend = blend)
    }

    pub fn cull(self, cull: Cull) -> Self {
        self.drawing(|draw| draw.cull = cull)
    }

    pub fn clear(self) -> Self {
        self.drawing(|draw| draw.clear = true)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuImage {
    Source,
    Output,
    Scratch(u32),
    Texture(u32),
    Layer(u32),
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TaskSpec {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TaskState {
    pub id: u64,
    pub task: String,
    pub status: TaskStatus,
    pub progress: f64,
    pub message: String,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FrameRequest {
    #[serde(default)]
    pub comp: Option<String>,
    #[serde(default)]
    pub layer: Option<String>,
    pub time: f64,
    pub scale: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AudioRequest {
    #[serde(default)]
    pub layer: Option<String>,
    pub start: f64,
    pub duration: f64,
    pub rate: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioDesc {
    pub channels: u32,
    pub rate: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PickFile {
    #[serde(default)]
    pub save: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub extensions: Vec<String>,
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
    Start { task: String, args: serde_json::Value },
    Task { id: u64 },
    Cancel { id: u64 },
    Progress { fraction: f64, message: String },
    Frame { request: FrameRequest },
    Audio { request: AudioRequest },
    PickFile { request: PickFile },
    UploadImage { key: String, width: u32, height: u32 },
    ParamsAt { time: f64 },
    CacheHas { key: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ParamsAt {
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub history: Option<u64>,
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
    #[serde(default)]
    pub camera: Option<Camera>,
    #[serde(default)]
    pub lights: Vec<Light>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera {
    pub view: [[f64; 4]; 4],
    pub position: [f64; 3],
    pub zoom: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub position: [f64; 3],
    pub direction: [f64; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
    pub cone_angle: f32,
    pub feather: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Ambient,
    #[default]
    Point,
    Spot,
    Parallel,
}

impl Scene {
    pub const NEAR: f64 = 1.0;

    pub fn centred(layer_size: [f64; 2]) -> Scene {
        let zoom = (layer_size[0] * 50.0 / 36.0).max(1.0);
        let centre = [layer_size[0] / 2.0, layer_size[1] / 2.0];
        let projection = [[1.0, 0.0, centre[0] / zoom, 0.0], [0.0, 1.0, centre[1] / zoom, 0.0], [0.0, 0.0, 1.0 / zoom, 1.0]];
        let view = [[1.0, 0.0, 0.0, -centre[0]], [0.0, 1.0, 0.0, -centre[1]], [0.0, 0.0, 1.0, zoom], [0.0, 0.0, 0.0, 1.0]];
        let camera = Camera { view, position: [centre[0], centre[1], -zoom], zoom };
        Scene { projection: Some(projection), camera_layer: false, camera: Some(camera), lights: Vec::new() }
    }

    pub fn project(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        let m = self.projection?;
        let row = |r: [f64; 4]| r[0] * point[0] + r[1] * point[1] + r[2] * point[2] + r[3];
        let w = row(m[2]);
        (w > 1e-9).then(|| [row(m[0]) / w, row(m[1]) / w])
    }

    pub fn view_projection(&self, cx: &RenderCx) -> Option<[f32; 16]> {
        let [x, y, w] = self.projection?;
        let [width, height] = [f64::from(cx.size[0].max(1)), f64::from(cx.size[1].max(1))];
        let steepness = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        let depth = match self.camera {
            Some(camera) if steepness <= w[3].abs() * 1e-12 => std::array::from_fn(|i| w[3] * (if i == 3 { 0.5 } else { 0.0 } - camera.view[2][i] * 1e-6)),
            _ => [0.0, 0.0, 0.0, Self::NEAR * steepness],
        };
        let rows: [[f64; 4]; 4] = [
            std::array::from_fn(|i| 2.0 * (x[i] * cx.scale + cx.offset[0] * w[i]) / width - w[i]),
            std::array::from_fn(|i| w[i] - 2.0 * (y[i] * cx.scale + cx.offset[1] * w[i]) / height),
            depth,
            w,
        ];
        Some(std::array::from_fn(|i| rows[i % 4][i / 4] as f32))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UiInput {
    #[serde(default)]
    pub events: Vec<UiEvent>,
    pub size: [f32; 2],
    #[serde(default)]
    pub keys: String,
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
    #[serde(default)]
    pub modifiers: Modifiers,
    #[serde(default)]
    pub scroll: [f32; 2],
}

impl Pointer {
    pub fn new(pos: [f32; 2], phase: PointerPhase) -> Self {
        Pointer { pos, phase, modifiers: Modifiers::default(), scroll: [0.0; 2] }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub command: bool,
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
    Image { id: u64, key: String, size: [f32; 2] },
    Scroll { id: u64, height: f32, children: Vec<Widget> },
    Collapsing { id: u64, title: String, open: bool, children: Vec<Widget> },
    Tabs { id: u64, options: Vec<String>, selected: u32 },
    Progress { fraction: f32, text: String },
    Tooltip { text: String },
    Grid { id: u64, columns: u32, children: Vec<Widget> },
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
    Polygon { points: Vec<[f32; 2]>, fill: Paint },
    Image { key: String, min: [f32; 2], max: [f32; 2] },
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
