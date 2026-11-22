use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_plugin_host::{
    Blend, Call, Camera, Cull, Draw, EffectSpec, GpuData, GpuImage, Host, Layout, Light, LightKind, MaskPath, Param, ParamKind, ParamsAt, Pixels, RenderCx,
    Scene, Value as Wire,
};
use effectcraft_engine::effects::dynamic::{
    self, DynamicEffect, DynamicParam, GpuBlend, GpuBytes, GpuCull, GpuDraw, GpuPass, GpuPicture, GpuProgram, GpuSlot, Grid,
};
use effectcraft_engine::effects::{Buf, EffectCtx, Image, LayerPixels, MaskShape, Params, SceneLightKind, util};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::ParamUi;

use crate::manage::Loaded;

const MAX_PADDING: u32 = 4096;
const MAX_STAMPED_FILES: usize = 256;
const STAMP_LIFETIME: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) fn effect_id(plugin: &str, effect: &str) -> String {
    format!("{plugin}.{effect}")
}

pub(crate) fn register(loaded: &Arc<Loaded>) {
    let plugin_id = &loaded.bundle.manifest.id;
    for spec in &loaded.plugin.contributions().effects {
        let (render_plugin, render_spec) = (loaded.clone(), spec.clone());
        let gpu = spec.shader.as_ref().map(|_| {
            let (plugin, spec) = (loaded.clone(), spec.clone());
            Box::new(move |ctx: &EffectCtx, grid: Grid| gpu_program(&plugin, &spec, ctx, grid)) as dynamic::GpuDyn
        });
        dynamic::register(DynamicEffect {
            owner: plugin_id.clone(),
            id: effect_id(plugin_id, &spec.id),
            name: spec.name.clone(),
            category: spec.category.clone(),
            params: spec.params.iter().map(declare).collect(),
            render: Box::new(move |ctx, buf| render(&render_plugin, &render_spec, ctx, buf)),
            shader: spec.shader.as_deref().map(Arc::from),
            gpu,
            time_dependent: spec.time_dependent,
            reads_other_frames: spec.reads_other_frames,
            reads_comp_scene: spec.reads_scene,
            reads_param_history: spec.reads_param_history,
            revision: loaded.revision,
            files: Some(folders_stamp(vec![loaded.bundle.dir.clone(), loaded.data_dir.clone()])),
        });
    }
}

fn folders_stamp(folders: Vec<std::path::PathBuf>) -> dynamic::StampDyn {
    let recent: std::sync::Mutex<Option<(std::time::Instant, u64)>> = std::sync::Mutex::new(None);
    Box::new(move || {
        let mut recent = recent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, stamp)) = *recent
            && at.elapsed() < STAMP_LIFETIME
        {
            return stamp;
        }
        let stamp = stamp_folders(&folders);
        *recent = Some((std::time::Instant::now(), stamp));
        stamp
    })
}

fn stamp_folders(folders: &[std::path::PathBuf]) -> u64 {
    let mut stamps = Vec::new();
    let mut pending = folders.to_vec();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten().take(MAX_STAMPED_FILES.saturating_sub(stamps.len())) {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            }
            stamps.push((entry.path(), meta.len(), meta.modified().ok()));
        }
    }
    stamps.sort();
    let mut hasher = std::hash::DefaultHasher::new();
    stamps.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn unregister(plugin: &str) {
    dynamic::unregister(plugin);
}

fn declare(param: &Param) -> DynamicParam {
    let (default, ui) = match &param.kind {
        ParamKind::Slider { default, min, max } => {
            let decimals = if max - min <= 10.0 { 2 } else { 1 };
            let ui = ParamUi::Slider { min: *min, max: *max, slider_min: *min, slider_max: *max, decimals };
            (Value::Scalar(*default), ui)
        }
        ParamKind::Percent { default } => (Value::Scalar(*default), ParamUi::Percent),
        ParamKind::Angle { default } => (Value::Scalar(*default), ParamUi::Angle),
        ParamKind::Checkbox { default } => (Value::Bool(*default), ParamUi::Checkbox),
        ParamKind::Popup { options, default } => (Value::Enum(*default), ParamUi::Popup { options: options.clone() }),
        ParamKind::Point { default } => (Value::Vec2(*default), ParamUi::Point),
        ParamKind::Point3 { default } => (Value::Vec3(*default), ParamUi::Point3),
        ParamKind::Color { default } => (Value::Color(*default), ParamUi::Color),
        ParamKind::Text { default } => (Value::Str(default.clone()), ParamUi::Text),
        ParamKind::Layer => (Value::Layer(None), ParamUi::Layer),
        ParamKind::Mask => (Value::Enum(0), ParamUi::Mask),
    };
    DynamicParam { id: param.id.clone(), name: param.name.clone(), default, ui }
}

fn grid_of(buf: &Buf) -> Grid {
    Grid { width: buf.img.width, height: buf.img.height, offset: buf.offset, scale: buf.scale }
}

fn to_px(grid: Grid, point: [f64; 2]) -> [f64; 2] {
    [point[0] * grid.scale + grid.offset[0], point[1] * grid.scale + grid.offset[1]]
}

fn declared_default(param: &Param, layer_size: [f64; 2]) -> Value {
    match declare(param).default {
        Value::Vec2([x, y]) => Value::Vec2([x * layer_size[0], y * layer_size[1]]),
        Value::Vec3([x, y, z]) => Value::Vec3([x * layer_size[0], y * layer_size[1], z]),
        other => other,
    }
}

fn wire_params(spec: &EffectSpec, params: &Params, layer_size: [f64; 2], grid: Grid) -> BTreeMap<String, Wire> {
    let convert = |param: &Param| -> Wire {
        let fallback;
        let value = match params.get(&param.id) {
            Some(value) => value,
            None => {
                fallback = declared_default(param, layer_size);
                &fallback
            }
        };
        match &param.kind {
            ParamKind::Slider { .. } | ParamKind::Percent { .. } | ParamKind::Angle { .. } => Wire::Number(value.as_f64()),
            ParamKind::Checkbox { .. } => Wire::Bool(value.as_bool()),
            ParamKind::Popup { .. } | ParamKind::Mask => Wire::Number(f64::from(value.as_enum())),
            ParamKind::Point { .. } => Wire::List(to_px(grid, value.as_vec2()).to_vec()),
            ParamKind::Point3 { .. } => {
                let [x, y, z] = value.as_vec3();
                let [x, y] = to_px(grid, [x, y]);
                Wire::List(vec![x, y, z * grid.scale])
            }
            ParamKind::Color { .. } => Wire::List(value.as_color().iter().map(|c| f64::from(*c)).collect()),
            ParamKind::Text { .. } => match value {
                Value::Str(text) => Wire::Text(text.clone()),
                _ => Wire::Text(String::new()),
            },
            ParamKind::Layer => Wire::Bool(value.as_layer().is_some()),
        }
    };
    spec.params.iter().map(|param| (param.id.clone(), convert(param))).collect()
}

fn render_cx(spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> RenderCx {
    RenderCx {
        params: wire_params(spec, ctx.params, ctx.layer_size, grid),
        time: ctx.time,
        comp_time: ctx.env.comp_time,
        frame_rate: ctx.fps(),
        scale: grid.scale,
        layer_size: ctx.layer_size,
        offset: grid.offset,
        size: [grid.width, grid.height],
        instance: ctx.env.instance,
        float: ctx.env.float_depth,
        linear: ctx.env.working_linear,
    }
}

struct RenderHost<'a, 'b> {
    ctx: &'a EffectCtx<'b>,
    grid: Grid,
    spec: &'a EffectSpec,
}

impl RenderHost<'_, '_> {
    fn on_grid(&self, other: &LayerPixels) -> Option<Pixels> {
        let grid = Buf { img: Image { width: self.grid.width, height: self.grid.height, data: Vec::new() }, offset: self.grid.offset, scale: self.grid.scale };
        let image = util::fit_layer(self.ctx, &grid, other, false);
        Some(Pixels { width: image.width, height: image.height, data: image.data })
    }
}

impl Host for RenderHost<'_, '_> {
    fn layer(&mut self, param: &str, comp_time: Option<f64>) -> Option<Pixels> {
        let other = match comp_time {
            None => self.ctx.layer_param(param, true)?,
            Some(time) => {
                let layer = self.ctx.params.get(param)?.as_layer()?;
                self.ctx.env.host?.layer_at(layer, time, true)?
            }
        };
        self.on_grid(&other)
    }

    fn source(&mut self, layer_time: f64) -> Option<Pixels> {
        let buf = self.ctx.env.host?.self_at(layer_time, self.ctx.env.effect_index)?;
        self.on_grid(&LayerPixels { buf, size: self.ctx.layer_size })
    }

    fn masks(&mut self) -> Vec<MaskPath> {
        let grid = self.grid;
        let convert = |mask: &MaskShape| MaskPath {
            name: mask.name.clone(),
            points: mask.points.iter().map(|p| to_px(grid, *p)).collect(),
            closed: mask.closed,
            inverted: mask.inverted,
        };
        self.ctx.env.masks.iter().map(convert).collect()
    }

    fn params_at(&mut self, layer_time: f64) -> Option<ParamsAt> {
        let host = self.ctx.env.host?;
        let params = host.params_at(layer_time)?;
        Some(ParamsAt { params: wire_params(self.spec, &params, self.ctx.layer_size, self.grid), history: host.param_history() })
    }

    fn scene(&mut self) -> Option<Scene> {
        let scene = self.ctx.env.host?.comp_scene()?;
        let camera = scene.view.map(|camera| Camera { view: camera.layer_to_camera_rows, position: camera.eye, zoom: camera.zoom });
        let light = |light: &effectcraft_engine::effects::SceneLight| Light {
            kind: match light.kind {
                SceneLightKind::Parallel => LightKind::Parallel,
                SceneLightKind::Ambient => LightKind::Ambient,
                SceneLightKind::Spot => LightKind::Spot,
                SceneLightKind::Point => LightKind::Point,
            },
            position: light.pos,
            direction: light.travel_direction,
            colour: light.color,
            intensity: light.intensity,
            cone_angle: light.cone_degrees,
            feather: light.feather,
        };
        Some(Scene { projection: scene.camera, camera_layer: scene.camera_layer, camera, lights: scene.lights.iter().map(light).collect() })
    }
}

fn layout(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> Layout {
    let call = Call::Layout { effect: spec.id.clone(), cx: render_cx(spec, ctx, grid) };
    let mut layout: Layout = loaded.plugin.call(&call, None, &mut RenderHost { ctx, grid, spec }).unwrap_or_default();
    layout.padding = layout.padding.min(MAX_PADDING);
    layout
}

fn render(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, mut buf: Buf) -> Buf {
    let layout = layout(loaded, spec, ctx, grid_of(&buf));
    if layout.padding > 0 {
        buf.pad(layout.padding);
    }
    let grid = grid_of(&buf);
    let cx = render_cx(spec, ctx, grid);
    let rendered = loaded.plugin.render(&spec.id, &cx, layout.reach, &mut buf.img.data, || RenderHost { ctx, grid, spec });
    loaded.report(&spec.id, rendered.err().map(|e| e.to_string()));
    buf
}

fn gpu_program(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> Option<GpuProgram> {
    let layout = layout(loaded, spec, ctx, grid);
    let plan = layout.gpu?;
    let slot = |image: GpuImage| match image {
        GpuImage::Source => GpuSlot::Source,
        GpuImage::Output => GpuSlot::Output,
        GpuImage::Scratch(index) => GpuSlot::Scratch(index as usize),
        GpuImage::Texture(index) => GpuSlot::Texture(index as usize),
    };
    let cached = |key: &str| {
        let bytes = loaded.plugin.cached(key);
        if bytes.is_none() {
            loaded.report(&spec.id, Some(format!("the GPU plan names {key:?}, which is not in the plugin's cache")));
        }
        bytes
    };
    let buffers = plan
        .buffers
        .into_iter()
        .enumerate()
        .map(|(index, data)| {
            Some(match data {
                GpuData::Cache(key) => GpuBytes { bytes: cached(&key)?, name: key },
                GpuData::Floats(values) => GpuBytes { name: format!("#{index}"), bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() },
                GpuData::Indices(values) => GpuBytes { name: format!("#{index}"), bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() },
            })
        })
        .collect::<Option<_>>()?;
    let textures = plan
        .textures
        .into_iter()
        .map(|texture| Some(GpuPicture { bytes: cached(&texture.key)?, name: texture.key, size: texture.size }))
        .collect::<Option<_>>()?;
    let draw = |draw: Draw| GpuDraw {
        vertices: draw.vertices as usize,
        layout: draw.layout,
        indices: draw.indices.map(|index| index as usize),
        instances: draw.instances.map(|index| index as usize),
        instance_layout: draw.instance_layout,
        count: draw.count,
        depth: draw.depth,
        blend: match draw.blend {
            Blend::Replace => GpuBlend::Replace,
            Blend::Over => GpuBlend::Over,
            Blend::Add => GpuBlend::Add,
        },
        cull: match draw.cull {
            Cull::None => GpuCull::None,
            Cull::Back => GpuCull::Back,
            Cull::Front => GpuCull::Front,
        },
        clear: draw.clear,
    };
    let passes = plan
        .passes
        .into_iter()
        .map(|pass| GpuPass {
            kernel: pass.kernel,
            src: slot(pass.src),
            aux: pass.aux.map(slot),
            out: slot(pass.out),
            f: pass.f,
            u: pass.u,
            draw: pass.draw.map(draw),
        })
        .collect();
    Some(GpuProgram { padding: layout.padding, images: plan.images, buffers, textures, passes })
}
