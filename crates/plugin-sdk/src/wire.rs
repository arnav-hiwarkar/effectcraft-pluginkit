use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::kernels::{BlendMode, BlurShape, Filter, HostKernel, Measure, Taps};
use crate::viewer::{ToolEvent, ToolSpec};

pub const ABI: u32 = 1;
pub const PIXEL_BYTES: usize = 16;

pub type Pixel = [f32; 4];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Describe,
    Layout { effect: String, cx: RenderCx },
    Render { effect: String, cx: RenderCx, frame: FrameDesc },
    Input { effect: String, cx: RenderCx, out: Rect },
    Pass { effect: String, cx: RenderCx, kernel: String, f: Vec<f32>, u: Vec<u32>, src: Window, aux: Option<Window>, out: Window, spread: bool },
    Panel { panel: String, input: UiInput },
    Command { command: String },
    Task { task: String, args: serde_json::Value },
    Controls { effect: String, param: String, instance: u64, value: String, input: UiInput },
    Overlay { effect: String, cx: RenderCx },
    Drag { effect: String, cx: RenderCx, key: String, to: [f64; 2], value: String },
    Tool { tool: String, event: ToolEvent },
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
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub separate_input: bool,
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

    pub fn params(mut self, params: impl IntoIterator<Item = Param>) -> Self {
        self.params.extend(params);
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
    Slider {
        default: f64,
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        soft: Option<[f64; 2]>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        log: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        percent: bool,
    },
    Percent {
        default: f64,
    },
    Angle {
        default: f64,
    },
    Checkbox {
        default: bool,
    },
    Popup {
        options: Vec<String>,
        default: u32,
    },
    Point {
        default: [f64; 2],
    },
    Point3 {
        default: [f64; 3],
    },
    Color {
        default: [f64; 4],
    },
    Text {
        default: String,
    },
    Custom {
        default: String,
    },
    Layer,
    Mask,
}

impl Param {
    fn new(id: &str, name: &str, kind: ParamKind) -> Self {
        Param { id: id.into(), name: name.into(), kind }
    }

    pub fn slider(id: &str, name: &str, default: f64, range: RangeInclusive<f64>) -> Self {
        Param::new(id, name, ParamKind::Slider { default, min: *range.start(), max: *range.end(), soft: None, log: false, percent: false })
    }

    pub fn percent(id: &str, name: &str, default: f64) -> Self {
        Param::new(id, name, ParamKind::Percent { default })
    }

    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        let (min, max) = (*range.start(), *range.end());
        self.kind = match self.kind {
            ParamKind::Percent { default } => ParamKind::Slider { default, min, max, soft: None, log: false, percent: true },
            ParamKind::Slider { default, soft, log, percent, .. } => ParamKind::Slider { default, min, max, soft, log, percent },
            other => other,
        };
        self
    }

    pub fn soft(mut self, range: RangeInclusive<f64>) -> Self {
        if let ParamKind::Slider { soft, .. } = &mut self.kind {
            *soft = Some([*range.start(), *range.end()]);
        }
        self
    }

    pub fn log(mut self) -> Self {
        if let ParamKind::Slider { log, .. } = &mut self.kind {
            *log = true;
        }
        self
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
    #[serde(default)]
    pub input: Option<Window>,
    #[serde(default)]
    pub spread: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn of_size(size: [u32; 2]) -> Rect {
        let side = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
        Rect { left: 0, top: 0, right: side(size[0]), bottom: side(size[1]) }
    }

    pub fn width(&self) -> u32 {
        self.right.saturating_sub(self.left).max(0) as u32
    }

    pub fn height(&self) -> u32 {
        self.bottom.saturating_sub(self.top).max(0) as u32
    }

    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    pub fn grown(self, by: u32) -> Rect {
        let by = i32::try_from(by).unwrap_or(i32::MAX);
        Rect {
            left: self.left.saturating_sub(by),
            top: self.top.saturating_sub(by),
            right: self.right.saturating_add(by),
            bottom: self.bottom.saturating_add(by),
        }
    }

    pub fn clipped(self, size: [u32; 2]) -> Rect {
        let frame = Rect::of_size(size);
        let left = self.left.clamp(0, frame.right);
        let top = self.top.clamp(0, frame.bottom);
        Rect { left, top, right: self.right.clamp(left, frame.right), bottom: self.bottom.clamp(top, frame.bottom) }
    }

    pub fn union(self, other: Rect) -> Rect {
        Rect { left: self.left.min(other.left), top: self.top.min(other.top), right: self.right.max(other.right), bottom: self.bottom.max(other.bottom) }
    }

    pub fn contains(&self, other: Rect) -> bool {
        other.left >= self.left && other.top >= self.top && other.right <= self.right && other.bottom <= self.bottom
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Window {
    pub size: [u32; 2],
    pub first_row: u32,
    pub rows: u32,
}

impl Window {
    pub fn pixels(&self) -> usize {
        self.size[0] as usize * self.rows as usize
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Layout {
    #[serde(default, deserialize_with = "four_sides")]
    pub padding: [u32; 4],
    #[serde(default)]
    pub reach: Option<u32>,
    #[serde(default)]
    pub gpu: Option<GpuPlan>,
    #[serde(default)]
    pub cpu: bool,
    #[serde(default)]
    pub maps_input: bool,
}

fn four_sides<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<[u32; 4], D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Sides {
        All(u32),
        Each([u32; 4]),
    }
    Ok(match Sides::deserialize(deserializer)? {
        Sides::All(n) => [n; 4],
        Sides::Each(sides) => sides,
    })
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
        self.padding = [padding; 4];
        self
    }

    pub fn padded_sides(mut self, left: u32, top: u32, right: u32, bottom: u32) -> Self {
        self.padding = [left, top, right, bottom];
        self
    }

    pub fn maps_input(mut self) -> Self {
        self.maps_input = true;
        self
    }

    pub fn gpu(mut self, plan: GpuPlan) -> Self {
        self.gpu = Some(plan);
        self
    }

    pub fn passes(mut self, plan: GpuPlan) -> Self {
        self.gpu = Some(plan);
        self.cpu = true;
        self
    }
}

pub const MAX_PASSES: usize = 128;
pub const MAX_SCRATCH_IMAGES: usize = 32;
pub const MAX_SCRATCH_FRAMES: u64 = 8;
pub const MAX_PASS_FLOATS: usize = 96;
pub const MAX_PASS_UINTS: usize = 16;
pub const MAX_BUFFERS: usize = 64;
pub const MAX_TEXTURES: usize = 16;
pub const MAX_TEXELS: u64 = 1 << 24;

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
    #[serde(default)]
    pub reach: Option<u32>,
    #[serde(default)]
    pub aux_reach: Option<u32>,
    #[serde(default)]
    pub host: Option<HostKernel>,
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

    pub fn size_of(&self, image: GpuImage, frame: [u32; 2]) -> Option<[u32; 2]> {
        match image {
            GpuImage::Source | GpuImage::Output => Some(frame),
            GpuImage::Scratch(index) => self.images.get(index as usize).copied(),
            GpuImage::Texture(index) => self.textures.get(index as usize).map(|texture| texture.size),
            GpuImage::Layer(index) => ((index as usize) < self.layers.len()).then_some(frame),
        }
    }

    pub fn runs_on_cpu(&self) -> bool {
        self.buffers.is_empty() && self.textures.is_empty() && self.layers.is_empty() && self.passes.iter().all(|pass| pass.draw.is_none())
    }

    pub fn check(&self, frame: [u32; 2]) -> Result<(), String> {
        if !self.runs_on_cpu() {
            return Err("a plan with draw passes, buffers, textures or layers runs only on the GPU".into());
        }
        self.validate(frame)
    }

    pub fn validate(&self, frame: [u32; 2]) -> Result<(), String> {
        let passes = self.passes.len();
        if !(1..=MAX_PASSES).contains(&passes) {
            return Err(format!("a plan has 1 to {MAX_PASSES} passes, this one has {passes}"));
        }
        if self.images.len() > MAX_SCRATCH_IMAGES {
            return Err(format!("a plan has at most {MAX_SCRATCH_IMAGES} scratch images, this one has {}", self.images.len()));
        }
        let [fw, fh] = frame;
        for (i, &[w, h]) in self.images.iter().enumerate() {
            if w == 0 || h == 0 || w > fw.saturating_mul(2) || h > fh.saturating_mul(2) {
                return Err(format!("scratch image {i} is {w}x{h}; it must be non-empty and at most twice the {fw}x{fh} padded frame"));
            }
        }
        let area = |[w, h]: [u32; 2]| u64::from(w) * u64::from(h);
        let mut temporaries: Vec<[u32; 2]> = Vec::new();
        for pass in self.passes.iter().filter(|pass| pass.host.as_ref().is_some_and(HostKernel::needs_temporary)) {
            if let Some(size) = self.size_of(pass.out, frame).filter(|size| !temporaries.contains(size)) {
                temporaries.push(size);
            }
        }
        let scratch_area = self.images.iter().chain(&temporaries).copied().map(area).fold(0, u64::saturating_add);
        if scratch_area > MAX_SCRATCH_FRAMES.saturating_mul(area(frame)) {
            return Err(format!("the scratch images and blur temporaries together exceed {MAX_SCRATCH_FRAMES} times the {fw}x{fh} padded frame"));
        }
        if self.buffers.len() > MAX_BUFFERS {
            return Err(format!("a plan has at most {MAX_BUFFERS} buffers, this one has {}", self.buffers.len()));
        }
        let pictures = self.textures.len() + self.layers.len();
        if pictures > MAX_TEXTURES {
            return Err(format!("a plan has at most {MAX_TEXTURES} textures and layers together, this one has {pictures}"));
        }
        if let Some((i, texture)) = self.textures.iter().enumerate().find(|(_, texture)| area(texture.size) == 0) {
            return Err(format!("texture {i} ({:?}) is {}x{}; it must be non-empty", texture.key, texture.size[0], texture.size[1]));
        }
        let texels = self.textures.iter().map(|texture| area(texture.size)).fold((self.layers.len() as u64).saturating_mul(area(frame)), u64::saturating_add);
        if texels > MAX_TEXELS {
            return Err(format!("the textures and layers hold {texels} pixels; the limit is {MAX_TEXELS}"));
        }
        let mut written = vec![GpuImage::Source];
        let mut depth_size = None;
        for (n, pass) in self.passes.iter().enumerate() {
            if pass.f.len() > MAX_PASS_FLOATS || pass.u.len() > MAX_PASS_UINTS {
                let (floats, uints) = (pass.f.len(), pass.u.len());
                return Err(format!("pass {n} has {floats} floats and {uints} uints; the limits are {MAX_PASS_FLOATS} and {MAX_PASS_UINTS}"));
            }
            if matches!(pass.out, GpuImage::Source | GpuImage::Texture(_) | GpuImage::Layer(_)) {
                return Err(format!("pass {n} writes {:?}, which passes only read", pass.out));
            }
            for image in std::iter::once(pass.src).chain(pass.aux) {
                if self.size_of(image, frame).is_none() {
                    return Err(format!("pass {n} reads {image:?}, which the plan does not declare"));
                }
                if matches!(image, GpuImage::Output | GpuImage::Scratch(_)) && !written.contains(&image) {
                    return Err(format!("pass {n} reads {image:?} before a pass writes it"));
                }
                if image == pass.out && !pass.host.as_ref().is_some_and(HostKernel::pointwise) {
                    return Err(format!("pass {n} reads the image it writes"));
                }
            }
            let Some(out_size) = self.size_of(pass.out, frame) else {
                return Err(format!("pass {n} writes {:?}, which the plan does not declare", pass.out));
            };
            if let Some(kernel) = &pass.host {
                let size = |image| self.size_of(image, frame).unwrap_or_default();
                let refused = match pass.draw {
                    Some(_) => Err("a host kernel pass cannot also draw".to_string()),
                    None => kernel.check(size(pass.src), pass.aux.map(size), out_size),
                };
                refused.map_err(|why| format!("pass {n} ({}): {why}", kernel.name()))?;
            }
            if let Some(draw) = &pass.draw {
                let named = [Some(draw.vertices), draw.indices, draw.instances];
                if let Some(missing) = named.into_iter().flatten().find(|&buffer| buffer as usize >= self.buffers.len()) {
                    return Err(format!("pass {n} names buffer {missing}, which the plan does not declare"));
                }
                if draw.depth && *depth_size.get_or_insert(out_size) != out_size {
                    return Err(format!("pass {n} uses the depth buffer on an image of another size than an earlier pass"));
                }
            }
            written.push(pass.out);
        }
        if !written.contains(&GpuImage::Output) {
            return Err("no pass writes the output image".into());
        }
        Ok(())
    }
}

impl GpuPass {
    pub fn new(kernel: &str, src: GpuImage, out: GpuImage) -> Self {
        GpuPass { kernel: kernel.to_string(), src, aux: None, out, f: Vec::new(), u: Vec::new(), draw: None, reach: None, aux_reach: None, host: None }
    }

    pub fn host(kernel: HostKernel, src: GpuImage, out: GpuImage) -> Self {
        GpuPass { host: Some(kernel.clone()), ..GpuPass::new(kernel.name(), src, out) }
    }

    pub fn blur(src: GpuImage, out: GpuImage, radius: [f32; 2]) -> Self {
        GpuPass::host(HostKernel::Blur { shape: BlurShape::Gaussian, radius }, src, out)
    }

    pub fn box_blur(src: GpuImage, out: GpuImage, radius: [f32; 2]) -> Self {
        GpuPass::host(HostKernel::Blur { shape: BlurShape::Box, radius }, src, out)
    }

    pub fn resample(src: GpuImage, out: GpuImage, filter: Filter) -> Self {
        GpuPass::host(HostKernel::Resample { filter, scale: None }, src, out)
    }

    pub fn blend_images(base: GpuImage, top: GpuImage, out: GpuImage, mode: BlendMode, amount: f32) -> Self {
        GpuPass::host(HostKernel::Blend { mode, amount }, base, out).aux(top)
    }

    pub fn curves(src: GpuImage, out: GpuImage, table: &[Pixel]) -> Self {
        GpuPass::host(HostKernel::Curves { table: table.to_vec() }, src, out)
    }

    pub fn convolve(src: GpuImage, out: GpuImage, taps: Taps) -> Self {
        GpuPass::host(HostKernel::Convolve { taps }, src, out)
    }

    pub fn warp(src: GpuImage, displacement: GpuImage, out: GpuImage, amount: [f32; 2]) -> Self {
        GpuPass::host(HostKernel::Warp { amount }, src, out).aux(displacement)
    }

    pub fn matrix(src: GpuImage, out: GpuImage, rows: [[f32; 4]; 4]) -> Self {
        GpuPass::host(HostKernel::Matrix { rows }, src, out)
    }

    pub fn to_linear(src: GpuImage, out: GpuImage) -> Self {
        GpuPass::host(HostKernel::ToLinear, src, out)
    }

    pub fn to_srgb(src: GpuImage, out: GpuImage) -> Self {
        GpuPass::host(HostKernel::ToSrgb, src, out)
    }

    pub fn brightness_curve(src: GpuImage, out: GpuImage, measure: Measure, curve: Vec<f32>, range: f32) -> Self {
        GpuPass::host(HostKernel::BrightnessCurve { measure, curve, range }, src, out)
    }

    pub fn dither(src: GpuImage, out: GpuImage, seed: u32, origin: [i64; 2]) -> Self {
        GpuPass::host(HostKernel::Dither { seed, origin }, src, out)
    }

    pub fn light_alpha(src: GpuImage, out: GpuImage) -> Self {
        GpuPass::host(HostKernel::LightAlpha, src, out)
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

    pub fn reach(mut self, rows: u32) -> Self {
        self.reach = Some(rows);
        self
    }

    pub fn aux_reach(mut self, rows: u32) -> Self {
        self.aux_reach = Some(rows);
        self
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
    ReadFile { root: Root, path: String, error_kind: Option<bool> },
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
    RemoveImage { key: String },
    ClipboardRead,
    ClipboardWrite { text: String },
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
pub enum FileErrorKind {
    NotFound,
    PermissionDenied,
    Other,
}

const NOT_FOUND: &str = "not found: ";
const DENIED: &str = "permission denied: ";

fn file_failure(path: &str, error: &std::io::Error) -> String {
    let prefix = match error.kind() {
        std::io::ErrorKind::NotFound => NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => DENIED,
        _ => "",
    };
    format!("{prefix}{path}: {error}")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileError {
    pub kind: FileErrorKind,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReadFailed {
    pub failed: FileError,
}

impl From<FileError> for String {
    fn from(error: FileError) -> String {
        error.message
    }
}

impl FileError {
    pub fn of(path: &str, error: &std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound => FileErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => FileErrorKind::PermissionDenied,
            _ => FileErrorKind::Other,
        };
        FileError { kind, message: file_failure(path, error) }
    }

    pub fn other(message: impl Into<String>) -> Self {
        FileError { kind: FileErrorKind::Other, message: message.into() }
    }

    pub fn from_reply(message: String) -> Self {
        let kind = if message.starts_with(NOT_FOUND) {
            FileErrorKind::NotFound
        } else if message.starts_with(DENIED) {
            FileErrorKind::PermissionDenied
        } else {
            FileErrorKind::Other
        };
        FileError { kind, message }
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FileError {}

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

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UiInput {
    #[serde(default)]
    pub events: Vec<UiEvent>,
    pub size: [f32; 2],
    #[serde(default)]
    pub keys: String,
    #[serde(default)]
    pub app: Vec<AppEvent>,
    #[serde(default)]
    pub timers: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Selection,
    Time,
    Comp,
    Project,
    Edit,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "topic", rename_all = "snake_case")]
pub enum AppEvent {
    Selection { layers: Vec<u64>, properties: Vec<[u64; 2]>, keyframes: u32 },
    Time { comp: u64, time: f64 },
    Comp { comp: Option<u64> },
    Project { path: Option<String> },
    Edit { layers: Vec<u64> },
}

impl AppEvent {
    pub fn topic(&self) -> Topic {
        match self {
            AppEvent::Selection { .. } => Topic::Selection,
            AppEvent::Time { .. } => Topic::Time,
            AppEvent::Comp { .. } => Topic::Comp,
            AppEvent::Project { .. } => Topic::Project,
            AppEvent::Edit { .. } => Topic::Edit,
        }
    }
}

pub const MIN_TIMER_MS: u32 = 16;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Timer {
    pub name: String,
    pub ms: u32,
    #[serde(default)]
    pub repeat: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(from = "PanelWire")]
pub struct PanelReply {
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub timers: Vec<Timer>,
    #[serde(skip)]
    pub legacy: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PanelWire {
    Legacy(Vec<Widget>),
    Reply {
        widgets: Vec<Widget>,
        #[serde(default)]
        topics: Vec<Topic>,
        #[serde(default)]
        timers: Vec<Timer>,
    },
}

impl From<PanelWire> for PanelReply {
    fn from(wire: PanelWire) -> Self {
        match wire {
            PanelWire::Legacy(widgets) => PanelReply { widgets, legacy: true, ..Default::default() },
            PanelWire::Reply { widgets, topics, timers } => PanelReply { widgets, topics, timers, legacy: false },
        }
    }
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
    Key(Key),
    Drop(Dropped),
}

impl UiEventKind {
    pub fn known_to_every_sdk(&self) -> bool {
        !matches!(self, UiEventKind::Key(_) | UiEventKind::Drop(_))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Key {
    pub name: String,
    #[serde(default)]
    pub modifiers: Modifiers,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Dropped {
    Files { paths: Vec<String> },
    Items { ids: Vec<u64> },
    Data { plugin: String, data: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Drag {
    File { root: Root, path: String },
    Data { data: String },
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
    Label {
        text: String,
        style: TextStyle,
    },
    Button {
        id: u64,
        text: String,
        enabled: bool,
    },
    Checkbox {
        id: u64,
        text: String,
        value: bool,
    },
    Slider {
        id: u64,
        text: String,
        value: f64,
        min: f64,
        max: f64,
    },
    Number {
        id: u64,
        text: String,
        value: f64,
        speed: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<[f64; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimals: Option<u8>,
    },
    TextEdit {
        id: u64,
        value: String,
        hint: String,
    },
    Popup {
        id: u64,
        text: String,
        options: Vec<String>,
        selected: u32,
    },
    Color {
        id: u64,
        text: String,
        value: [f32; 4],
    },
    Row {
        children: Vec<Widget>,
    },
    Column {
        children: Vec<Widget>,
    },
    Group {
        title: String,
        children: Vec<Widget>,
    },
    Separator,
    Space {
        size: f32,
    },
    Canvas {
        id: u64,
        size: [f32; 2],
        shapes: Vec<Shape>,
    },
    Image {
        id: u64,
        key: String,
        size: [f32; 2],
    },
    Scroll {
        id: u64,
        height: f32,
        children: Vec<Widget>,
    },
    Collapsing {
        id: u64,
        title: String,
        open: bool,
        children: Vec<Widget>,
    },
    Tabs {
        id: u64,
        options: Vec<String>,
        selected: u32,
    },
    Progress {
        fraction: f32,
        text: String,
    },
    Tooltip {
        text: String,
    },
    Grid {
        id: u64,
        columns: u32,
        children: Vec<Widget>,
    },
    DropTarget {
        id: u64,
        children: Vec<Widget>,
    },
    DragSource {
        id: u64,
        drag: Drag,
        children: Vec<Widget>,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: [u32; 2] = [53, 37];

    fn finish() -> GpuPass {
        GpuPass::new("tint", GpuImage::Source, GpuImage::Output)
    }

    fn plan(images: Vec<[u32; 2]>, passes: Vec<GpuPass>) -> GpuPlan {
        GpuPlan { images, passes, ..Default::default() }
    }

    #[test]
    fn a_read_asks_for_its_error_kind_as_a_field_and_a_read_from_an_older_sdk_does_not() {
        let asked = HostCall::ReadFile { root: Root::Data, path: "a".into(), error_kind: Some(true) };
        assert_eq!(serde_json::to_string(&asked).unwrap(), r#"{"call":"read_file","root":"data","path":"a","error_kind":true}"#);
        let older: HostCall = serde_json::from_str(r#"{"call":"read_file","root":"data","path":"a"}"#).unwrap();
        assert!(matches!(older, HostCall::ReadFile { error_kind: None, .. }));
        let failed = ReadFailed { failed: FileError { kind: FileErrorKind::NotFound, message: "a: gone".into() } };
        assert_eq!(serde_json::to_string(&failed).unwrap(), r#"{"failed":{"kind":"not_found","message":"a: gone"}}"#);
    }

    #[test]
    fn one_check_refuses_every_plan_beyond_the_limits() {
        let tint = |src, out| GpuPass::new("tint", src, out);
        let scratch = GpuImage::Scratch(0);
        let mut draw_without_buffer = plan(vec![], vec![GpuPass::draw("mesh", GpuImage::Output)]);
        draw_without_buffer.passes[0] = draw_without_buffer.passes[0].clone().vertices(0, &[3]);
        let mut depth_on_two_sizes = plan(vec![[8, 8]], vec![]);
        let mesh = depth_on_two_sizes.buffer(GpuData::Floats(vec![0.0; 9]));
        depth_on_two_sizes.passes =
            vec![GpuPass::draw("mesh", scratch).vertices(mesh, &[3]).depth(), GpuPass::draw("mesh", GpuImage::Output).vertices(mesh, &[3]).depth()];
        let mut too_many_pictures = plan(vec![], vec![finish()]);
        for i in 0..17 {
            too_many_pictures.layer(&format!("layer{i}"));
        }
        let mut too_many_texels = plan(vec![], vec![finish()]);
        too_many_texels.texture("big", [4096, 4097]);
        let mut wrapping_texels = plan(vec![], vec![finish()]);
        for size in [[u32::MAX, u32::MAX], [u32::MAX, 2], [1, 1]] {
            wrapping_texels.texture("huge", size);
        }
        let (source, output) = (GpuImage::Source, GpuImage::Output);
        let host = |kernel| plan(vec![], vec![GpuPass::host(kernel, source, output)]);
        let small_top = plan(vec![[8, 8]], vec![tint(source, scratch), GpuPass::blend_images(source, scratch, output, BlendMode::Add, 1.0)]);
        let drawing_blur = plan(vec![], vec![GpuPass { draw: Some(Draw::default()), ..GpuPass::blur(source, output, [1.0; 2]) }]);
        let separable = |x: usize, y: usize| Taps::Separable { x: vec![0.5; x], y: vec![0.5; y] };
        let cases = [
            ("no passes", plan(vec![], vec![]), "1 to 128 passes"),
            ("129 passes", plan(vec![], vec![finish(); 129]), "1 to 128 passes"),
            ("33 images", plan(vec![[8, 8]; 33], vec![finish()]), "at most 32 scratch images"),
            ("empty image", plan(vec![[0, 8]], vec![finish()]), "scratch image 0 is 0x8"),
            ("huge image", plan(vec![[2 * 53 + 1, 8]], vec![finish()]), "scratch image 0 is 107x8"),
            ("unwritten read", plan(vec![[8, 8]], vec![tint(scratch, GpuImage::Output)]), "before a pass writes it"),
            ("undeclared image", plan(vec![], vec![tint(GpuImage::Source, scratch), finish()]), "does not declare"),
            ("no output", plan(vec![[8, 8]], vec![tint(GpuImage::Source, scratch)]), "no pass writes the output"),
            ("reads own output", plan(vec![[8, 8]], vec![tint(GpuImage::Source, scratch), tint(scratch, scratch), finish()]), "reads the image it writes"),
            ("writes source", plan(vec![], vec![tint(GpuImage::Source, GpuImage::Source), finish()]), "which passes only read"),
            ("writes a layer", plan(vec![], vec![tint(GpuImage::Source, GpuImage::Layer(0)), finish()]), "which passes only read"),
            ("undeclared layer", plan(vec![], vec![tint(GpuImage::Layer(0), GpuImage::Output)]), "does not declare"),
            ("too much scratch", plan(vec![[2 * 53, 2 * 37]; 3], vec![finish()]), "exceed 8 times"),
            (
                "scratch at the limit and a blur temporary",
                plan(vec![[53, 37]; 8], vec![GpuPass::blur(source, scratch, [1.0; 2]), finish()]),
                "the scratch images and blur temporaries together exceed 8 times",
            ),
            ("97 floats", plan(vec![], vec![finish().floats(&[0.0; 97])]), "97 floats"),
            ("17 uints", plan(vec![], vec![finish().uints(&[0; 17])]), "17 uints"),
            ("17 pictures", too_many_pictures, "at most 16 textures and layers"),
            ("too many texels", too_many_texels, "the limit is 16777216"),
            ("texels that wrap", wrapping_texels, "hold 18446744073709551615 pixels; the limit is 16777216"),
            ("draw without its buffer", draw_without_buffer, "names buffer 0"),
            ("depth on two sizes", depth_on_two_sizes, "depth buffer on an image of another size"),
            (
                "wide blur",
                plan(vec![], vec![GpuPass::blur(source, output, [600.0, 1.0])]),
                "pass 0 (blur): `radius` is [600.0, 1.0]; each axis must be 0 to 512",
            ),
            ("blur of nan", plan(vec![], vec![GpuPass::box_blur(source, output, [f32::NAN, 1.0])]), "`radius` is [NaN, 1.0]"),
            ("blend without top", host(HostKernel::Blend { mode: BlendMode::Over, amount: 1.0 }), "pass 0 (blend): `aux` is missing"),
            ("blend of sizes", small_top, "pass 1 (blend): `aux` is 8x8 but `out` is 53x37"),
            ("even taps", plan(vec![], vec![GpuPass::convolve(source, output, separable(3, 2))]), "`y` has 2 taps; give an odd number from 1 to 1025"),
            ("too many taps", plan(vec![], vec![GpuPass::convolve(source, output, separable(1027, 1))]), "`x` has 1027 taps"),
            ("ragged full taps", host(HostKernel::Convolve { taps: Taps::Full { width: 3, weights: vec![1.0; 6] } }), "`weights` has 6 values for `width` 3"),
            ("one point curve", plan(vec![], vec![GpuPass::curves(source, output, &[[0.0; 4]])]), "`table` has 1 entries; it needs 2 to 4096"),
            ("infinite matrix", plan(vec![], vec![GpuPass::matrix(source, output, [[f32::INFINITY; 4]; 4])]), "`rows` holds a value that is not a finite"),
            ("far warp", plan(vec![], vec![GpuPass::warp(source, source, output, [5000.0, 0.0])]), "`amount` is [5000.0, 0.0]"),
            ("zero scale", host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some([0.0, 1.0]) }), "`scale` is [0.0, 1.0]"),
            ("drawing blur", drawing_blur, "pass 0 (blur): a host kernel pass cannot also draw"),
            (
                "flat brightness range",
                plan(vec![], vec![GpuPass::brightness_curve(source, output, Measure::Luma, vec![0.0, 1.0], 0.0)]),
                "pass 0 (brightness_curve): `range` is 0; it must be a finite number above 0",
            ),
            (
                "one point brightness curve",
                plan(vec![], vec![GpuPass::brightness_curve(source, output, Measure::MaxChannel, vec![1.0], 1.0)]),
                "`curve` has 1 entries",
            ),
        ];
        for (name, plan, expected) in cases {
            let why = plan.validate(FRAME).expect_err(name);
            assert!(why.contains(expected), "{name}: {why}");
        }
    }

    #[test]
    fn plans_at_the_limits_pass_and_only_cpu_plans_pass_the_cpu_check() {
        let mut images = vec![[2 * 53, 2 * 37], [53, 37], [53, 37], [53, 37]];
        images.extend([[1, 1]; 12]);
        let at_the_limits = plan(images, std::iter::repeat_n(finish().floats(&[0.0; 96]).uints(&[0; 16]), 64).collect());
        assert_eq!(at_the_limits.validate(FRAME), Ok(()));
        assert_eq!(at_the_limits.check(FRAME), Ok(()));
        let mut with_layer = plan(vec![], vec![]);
        let matte = with_layer.layer("matte");
        let with_layer = with_layer.pass(finish().aux(matte));
        assert_eq!(with_layer.validate(FRAME), Ok(()));
        assert_eq!(with_layer.check(FRAME), Err("a plan with draw passes, buffers, textures or layers runs only on the GPU".into()));
    }

    #[test]
    fn host_kernel_plans_pass_both_checks_and_plans_without_kernels_still_parse() {
        let mut plan = GpuPlan::new();
        let (half, blurred, glow) = (plan.scratch([27, 19]), plan.scratch([27, 19]), plan.scratch(FRAME));
        let plan = plan
            .pass(GpuPass::resample(GpuImage::Source, half, Filter::Bicubic))
            .pass(GpuPass::blur(half, blurred, [512.0, 0.0]))
            .pass(GpuPass::resample(blurred, glow, Filter::Bilinear))
            .pass(GpuPass::blend_images(GpuImage::Source, glow, GpuImage::Output, BlendMode::Screen, 0.5));
        assert_eq!(plan.check(FRAME), Ok(()));
        let sent: GpuPlan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(sent, plan);
        let before_kernels = r#"{"passes":[{"kernel":"tint","src":"source","out":"output","f":[1.0],"reach":2}]}"#;
        let old: GpuPlan = serde_json::from_str(before_kernels).unwrap();
        assert_eq!(old.passes, vec![finish().floats(&[1.0]).reach(2)]);
    }

    #[test]
    fn padding_from_a_plugin_built_before_four_sides_applies_to_every_side() {
        let old: Layout = serde_json::from_str(r#"{"padding":6,"reach":0}"#).unwrap();
        assert_eq!(old.padding, [6; 4]);
        let sides = Layout::pointwise().padded_sides(5, 0, 2, 1);
        let again: Layout = serde_json::from_str(&serde_json::to_string(&sides).unwrap()).unwrap();
        assert_eq!(again.padding, [5, 0, 2, 1]);
    }

    #[test]
    fn a_slider_or_percent_from_an_older_sdk_has_no_soft_range_and_stays_linear() {
        let slider: Param = serde_json::from_str(r#"{"id":"a","name":"A","kind":"slider","default":1.0,"min":0.0,"max":2.0}"#).unwrap();
        assert_eq!(slider, Param::slider("a", "A", 1.0, 0.0..=2.0));
        let percent: Param = serde_json::from_str(r#"{"id":"b","name":"B","kind":"percent","default":50.0}"#).unwrap();
        assert_eq!(percent, Param::percent("b", "B", 50.0));
        assert_eq!(
            serde_json::to_string(&Param::slider("a", "A", 1.0, 0.0..=2.0)).unwrap(),
            r#"{"id":"a","name":"A","kind":"slider","default":1.0,"min":0.0,"max":2.0}"#
        );
    }

    #[test]
    fn a_slider_takes_a_soft_range_and_a_log_scale_and_a_percent_takes_a_range() {
        let gain = Param::slider("g", "Gain", 1.0, 0.0..=1000.0).soft(0.01..=10.0).log();
        assert_eq!(gain.kind, ParamKind::Slider { default: 1.0, min: 0.0, max: 1000.0, soft: Some([0.01, 10.0]), log: true, percent: false });
        let mix = Param::percent("m", "Mix", 50.0).range(-100.0..=300.0);
        assert_eq!(mix.kind, ParamKind::Slider { default: 50.0, min: -100.0, max: 300.0, soft: None, log: false, percent: true });
        let sent: Param = serde_json::from_str(&serde_json::to_string(&gain).unwrap()).unwrap();
        assert_eq!(sent, gain);
    }
}
