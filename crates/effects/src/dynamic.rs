use std::sync::{Arc, RwLock};

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;

use crate::{Buf, EffectCtx, EffectSpec, ParamSpec};

pub type RenderDyn = Box<dyn Fn(&EffectCtx, Buf) -> Buf + Send + Sync>;
pub type GpuDyn = Box<dyn Fn(&EffectCtx, Grid) -> Option<GpuProgram> + Send + Sync>;
pub type StampDyn = Box<dyn Fn() -> u64 + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub width: u32,
    pub height: u32,
    pub offset: [f64; 2],
    pub scale: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuProgram {
    pub padding: u32,
    pub images: Vec<[u32; 2]>,
    pub buffers: Vec<GpuBytes>,
    pub textures: Vec<GpuPicture>,
    pub layers: Vec<String>,
    pub passes: Vec<GpuPass>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuBytes {
    pub name: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuPicture {
    pub name: String,
    pub size: [u32; 2],
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuPass {
    pub kernel: String,
    pub src: GpuSlot,
    pub aux: Option<GpuSlot>,
    pub out: GpuSlot,
    pub f: Vec<f32>,
    pub u: Vec<u32>,
    pub draw: Option<GpuDraw>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GpuDraw {
    pub vertices: usize,
    pub layout: Vec<u32>,
    pub indices: Option<usize>,
    pub instances: Option<usize>,
    pub instance_layout: Vec<u32>,
    pub count: Option<u32>,
    pub depth: bool,
    pub blend: GpuBlend,
    pub cull: GpuCull,
    pub clear: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GpuBlend {
    #[default]
    Replace,
    Over,
    Add,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GpuCull {
    #[default]
    None,
    Back,
    Front,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSlot {
    Source,
    Output,
    Scratch(usize),
    Texture(usize),
    Layer(usize),
}

pub struct DynamicParam {
    pub id: String,
    pub name: String,
    pub default: Value,
    pub ui: ParamUi,
}

pub struct DynamicEffect {
    pub owner: String,
    pub id: String,
    pub name: String,
    pub category: String,
    pub params: Vec<DynamicParam>,
    pub render: RenderDyn,
    pub shader: Option<Arc<str>>,
    pub gpu: Option<GpuDyn>,
    pub time_dependent: bool,
    pub reads_other_frames: bool,
    pub reads_comp_scene: bool,
    pub reads_param_history: bool,
    pub presets: Vec<(String, Vec<(String, Value)>)>,
    pub revision: u64,
    pub files: Option<StampDyn>,
}

pub struct Registered {
    pub spec: &'static EffectSpec,
    pub effect: DynamicEffect,
}

static REGISTRY: RwLock<Vec<Arc<Registered>>> = RwLock::new(Vec::new());
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

fn passthrough(_: &EffectCtx, buf: Buf) -> Buf {
    buf
}

fn keep(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

static SPECS: RwLock<Vec<&'static EffectSpec>> = RwLock::new(Vec::new());

fn declares(spec: &EffectSpec, effect: &DynamicEffect) -> bool {
    let same_param = |(known, param): (&ParamSpec, &DynamicParam)| {
        known.id == param.id && known.name == param.name && known.default == param.default && known.ui == param.ui
    };
    spec.id == effect.id
        && spec.name == effect.name
        && spec.category == effect.category
        && spec.gpu == effect.gpu.is_some()
        && spec.params.len() == effect.params.len()
        && spec.params.iter().zip(&effect.params).all(same_param)
}

fn spec_of(effect: &DynamicEffect) -> &'static EffectSpec {
    let mut specs = SPECS.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(known) = specs.iter().find(|spec| declares(spec, effect)) {
        return known;
    }
    let params = effect.params.iter().map(|p| ParamSpec { id: keep(&p.id), name: keep(&p.name), default: p.default.clone(), ui: p.ui.clone() }).collect();
    let spec: &'static EffectSpec = Box::leak(Box::new(EffectSpec {
        id: keep(&effect.id),
        name: keep(&effect.name),
        category: keep(&effect.category),
        params,
        render: passthrough,
        gpu: effect.gpu.is_some(),
        float: true,
    }));
    specs.push(spec);
    spec
}

pub fn register(effect: DynamicEffect) {
    let spec = spec_of(&effect);
    effectcraft_project::declare_reads_other_frames(spec.id, effect.reads_other_frames);
    let mut registry = REGISTRY.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|r| r.spec.id != spec.id);
    registry.push(Arc::new(Registered { spec, effect }));
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

pub fn unregister(owner: &str) {
    let mut registry = REGISTRY.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    for gone in registry.iter().filter(|r| r.effect.owner == owner) {
        effectcraft_project::declare_reads_other_frames(gone.spec.id, false);
    }
    registry.retain(|r| r.effect.owner != owner);
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

pub fn get(id: &str) -> Option<Arc<Registered>> {
    REGISTRY.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().find(|r| r.spec.id == id).cloned()
}

pub fn specs() -> Vec<&'static EffectSpec> {
    REGISTRY.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().map(|r| r.spec).collect()
}
