use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_plugin_host::kernels::{self, Lowered, StepImage};
use craft_plugin_host::{
    Blend, Call, Camera, Cull, Draw, EffectSpec, Focus, Host, LayerPlacement, Layout, Light, LightKind, MaskPath, Param, ParamKind, ParamsAt, Pixels, PlanData,
    PlanImage, Rect, RenderCx, Scene, Value as Wire,
};
use effectcraft_engine::effects::dynamic::{
    self, DynamicEffect, DynamicParam, GpuBlend, GpuBytes, GpuCull, GpuDraw, GpuPass, GpuPicture, GpuProgram, GpuSlot, Grid,
};
use effectcraft_engine::effects::{Buf, EffectCtx, Image, LayerPixels, MaskShape, Params, SceneLightKind, util};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::ParamUi;

use crate::manage::Loaded;

const PROBE_SIDE: u32 = 256;
const MAX_STAMPED_FILES: usize = 256;
const STAMP_LIFETIME: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) fn effect_id(plugin: &str, effect: &str) -> String {
    format!("{plugin}.{effect}")
}

pub(crate) fn register(loaded: &Arc<Loaded>) {
    let plugin_id = &loaded.bundle.manifest.id;
    for spec in &loaded.plugin.contributions().effects {
        let (render_plugin, render_spec) = (loaded.clone(), spec.clone());
        let (plugin, gpu_spec) = (loaded.clone(), spec.clone());
        let (margin_plugin, margin_spec) = (loaded.clone(), spec.clone());
        let gpu: dynamic::GpuDyn = Box::new(move |ctx: &EffectCtx, grid: Grid| gpu_program(&plugin, &gpu_spec, ctx, grid));
        let id = effect_id(plugin_id, &spec.id);
        dynamic::register(DynamicEffect {
            owner: plugin_id.clone(),
            id: id.clone(),
            name: spec.name.clone(),
            category: spec.category.clone(),
            params: spec.params.iter().map(declare).collect(),
            render: Box::new(move |ctx, buf| render(&render_plugin, &render_spec, ctx, buf)),
            shader: Some(Arc::from(spec.shader.as_deref().unwrap_or_default())),
            gpu: Some(gpu),
            time_dependent: spec.time_dependent,
            reads_other_frames: spec.reads_other_frames,
            reads_comp_scene: spec.reads_scene,
            reads_param_history: spec.reads_param_history,
            presets: spec.presets.iter().map(|preset| (preset.name.clone(), preset_values(spec, &preset.values))).collect(),
            groups: spec.groups.clone().into_iter().collect(),
            revision: loaded.revision,
            files: Some(revision_stamp(loaded.clone())),
            margin: Some(Box::new(move |ctx, scale| margin(&margin_plugin, &margin_spec, ctx, scale))),
        });
        if spec.shader.is_none() && default_plan_runs_on_gpu(loaded, spec) {
            mark_gpu_planned(&id);
        }
    }
}

fn plan_runs_on_gpu(spec: &EffectSpec, plan: &craft_plugin_host::Plan) -> bool {
    spec.shader.is_some() || plan.passes.iter().all(|pass| pass.host.is_some())
}

fn mark_gpu_planned(id: &str) {
    if let Some(registered) = dynamic::get(id) {
        registered.gpu_planned.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

struct Quiet;

impl Host for Quiet {}

fn default_plan_runs_on_gpu(loaded: &Loaded, spec: &EffectSpec) -> bool {
    let grid = Grid { width: PROBE_SIDE, height: PROBE_SIDE, offset: [0.0; 2], scale: 1.0 };
    let layer_size = [f64::from(PROBE_SIDE); 2];
    let cx = RenderCx { params: wire_params(spec, &Params::default(), layer_size, grid), scale: 1.0, layer_size, size: [PROBE_SIDE; 2], ..RenderCx::default() };
    let layout = loaded.plugin.call::<Layout>(&Call::Layout { effect: spec.id.clone(), cx }, None, &mut Quiet);
    layout.is_ok_and(|layout| layout.gpu.is_some_and(|plan| plan_runs_on_gpu(spec, &plan)))
}

fn revision_stamp(loaded: Arc<Loaded>) -> dynamic::StampDyn {
    let folders = vec![loaded.bundle.dir.clone(), loaded.data_dir.clone()];
    let recent: std::sync::Mutex<Option<(std::time::Instant, u64)>> = std::sync::Mutex::new(None);
    Box::new(move || {
        let files = {
            let mut recent = recent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match *recent {
                Some((at, stamp)) if at.elapsed() < STAMP_LIFETIME => stamp,
                _ => {
                    let stamp = stamp_folders(&folders);
                    *recent = Some((std::time::Instant::now(), stamp));
                    stamp
                }
            }
        };
        let mut hasher = std::hash::DefaultHasher::new();
        (files, loaded.plugin.cache_generation()).hash(&mut hasher);
        hasher.finish()
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
        ParamKind::Slider { default, min, max, soft, log, percent } => {
            let [slider_min, slider_max] = soft.map(|[low, high]| [low.max(*min), high.min(*max)]).filter(|[low, high]| low < high).unwrap_or([*min, *max]);
            let decimals = if slider_max - slider_min <= 10.0 { 2 } else { 1 };
            let log = *log && slider_min > 0.0;
            let ui = if log || *percent {
                ParamUi::Scaled { min: *min, max: *max, slider_min, slider_max, decimals, log, percent: *percent }
            } else {
                ParamUi::Slider { min: *min, max: *max, slider_min, slider_max, decimals }
            };
            (Value::Scalar(*default), ui)
        }
        ParamKind::Percent { default, .. } => (Value::Scalar(*default), ParamUi::Percent),
        ParamKind::Angle { default } => (Value::Scalar(*default), ParamUi::Angle),
        ParamKind::Checkbox { default } => (Value::Bool(*default), ParamUi::Checkbox),
        ParamKind::Popup { options, default } => (Value::Enum(*default), ParamUi::Popup { options: options.clone() }),
        ParamKind::Point { default } => (Value::Vec2(*default), ParamUi::Point),
        ParamKind::Point3 { default } => (Value::Vec3(*default), ParamUi::Point3),
        ParamKind::Color { default } => (Value::Color(*default), ParamUi::Color),
        ParamKind::Text { default } => (Value::Str(default.clone()), ParamUi::Text),
        ParamKind::Custom { default } => (Value::Str(default.clone()), ParamUi::Custom),
        ParamKind::Layer => (Value::Layer(None), ParamUi::Layer),
        ParamKind::Mask => (Value::Enum(0), ParamUi::Mask),
    };
    DynamicParam { id: param.id.clone(), name: param.name.clone(), default, ui }
}

fn preset_values(spec: &EffectSpec, values: &BTreeMap<String, Wire>) -> Vec<(String, Value)> {
    let convert = |param: &Param, value: &Wire| -> Option<Value> {
        Some(match (&param.kind, param.fit(value.clone()).ok()?) {
            (ParamKind::Popup { options, .. }, Wire::Number(n)) => Value::Enum((n as u32).min(options.len().saturating_sub(1) as u32)),
            (ParamKind::Mask | ParamKind::Layer, _) => return None,
            (_, Wire::Number(n)) => Value::Scalar(n),
            (_, Wire::Bool(b)) => Value::Bool(b),
            (ParamKind::Point { .. }, Wire::List(v)) => Value::Vec2([v[0], v[1]]),
            (ParamKind::Point3 { .. }, Wire::List(v)) => Value::Vec3([v[0], v[1], v[2]]),
            (_, Wire::List(v)) => Value::Color([v[0], v[1], v[2], v[3]]),
            (_, Wire::Text(text)) => Value::Str(text),
        })
    };
    spec.params.iter().filter_map(|param| Some((param.id.clone(), convert(param, values.get(&param.id)?)?))).collect()
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

pub(crate) fn wire_params(spec: &EffectSpec, params: &Params, layer_size: [f64; 2], grid: Grid) -> BTreeMap<String, Wire> {
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
            ParamKind::Text { .. } | ParamKind::Custom { .. } => match value {
                Value::Str(text) => Wire::Text(text.clone()),
                _ => Wire::Text(String::new()),
            },
            ParamKind::Layer => Wire::Bool(value.as_layer().is_some()),
        }
    };
    spec.params.iter().filter_map(|param| Some((param.id.clone(), param.fit(convert(param)).ok()?))).collect()
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
    fn effect(&self) -> Option<&EffectSpec> {
        Some(self.spec)
    }

    fn finish(&mut self, written: &mut [craft_plugin_host::Pixel]) {
        self.ctx.quantize_chunk(written);
    }

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
        Some(wire_scene(self.ctx.env.host?.comp_scene()?))
    }

    fn scene_at(&mut self, comp_time: f64) -> Option<Scene> {
        Some(wire_scene(self.ctx.env.host?.comp_scene_at(comp_time)?))
    }

    fn placement(&mut self, param: &str, comp_time: Option<f64>) -> Option<LayerPlacement> {
        let layer = self.ctx.params.get(param)?.as_layer()?;
        let placement = self.ctx.env.host?.placement(layer, comp_time)?;
        Some(LayerPlacement { matrix: placement.matrix, opacity: placement.opacity, size: placement.size })
    }
}

fn wire_scene(scene: effectcraft_engine::effects::CompScene) -> Scene {
    let camera = scene.view.map(|camera| Camera {
        view: camera.layer_to_camera_rows,
        position: camera.eye,
        zoom: camera.zoom,
        focus: camera.focus.map(|[distance, aperture, blur]| Focus { distance, aperture, blur }),
    });
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
    Scene { projection: scene.camera, camera_layer: scene.camera_layer, camera, lights: scene.lights.iter().map(light).collect() }
}

fn layout(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> Layout {
    let call = Call::Layout { effect: spec.id.clone(), cx: render_cx(spec, ctx, grid) };
    let mut layout: Layout = loaded.plugin.call(&call, None, &mut RenderHost { ctx, grid, spec }).unwrap_or_default();
    layout.padding = craft_plugin_host::applied_padding(layout.padding, [grid.width, grid.height]);
    layout
}

fn margin(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, scale: f64) -> Option<f64> {
    if spec.reads_other_frames {
        return None;
    }
    let [width, height] = ctx.layer_size.map(|side| (side * scale).ceil().clamp(1.0, f64::from(u32::MAX)) as u32);
    let layout = layout(loaded, spec, ctx, Grid { width, height, offset: [0.0; 2], scale });
    if layout.reach != Some(0) || layout.maps_input || layout.gpu.is_some_and(|plan| !plan.layers.is_empty()) {
        return None;
    }
    Some(f64::from(layout.padding.into_iter().max().unwrap_or(0)) / scale)
}

fn shown_area(ctx: &EffectCtx, grid: Grid) -> Option<Rect> {
    let shown = ctx.env.host?.shown()?;
    let [left, top, right, bottom] = shown.part;
    let step = f64::from(shown.grid);
    let edge = |at: f64, round: fn(f64) -> f64| (round(at / step) * step).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let [left, top] = to_px(grid, [left, top]).map(|at| edge(at, f64::floor));
    let [right, bottom] = to_px(grid, [right, bottom]).map(|at| edge(at, f64::ceil));
    Some(Rect { left, top, right, bottom })
}

fn render(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, mut buf: Buf) -> Buf {
    let layout = layout(loaded, spec, ctx, grid_of(&buf));
    if spec.shader.is_none() && layout.gpu.as_ref().is_some_and(|plan| plan_runs_on_gpu(spec, plan)) {
        mark_gpu_planned(&effect_id(&loaded.bundle.manifest.id, &spec.id));
    }
    buf.pad_sides(layout.padding);
    let grid = grid_of(&buf);
    let cx = render_cx(spec, ctx, grid);
    let host = || RenderHost { ctx, grid, spec };
    let passes = layout.gpu.as_ref().filter(|plan| layout.cpu && plan.runs_on_cpu());
    let rendered = match passes {
        Some(plan) => loaded.plugin.run_passes(&spec.id, &cx, plan, &mut buf.img.data, host),
        None => loaded.plugin.render(&spec.id, &cx, &layout, shown_area(ctx, grid), &mut buf.img.data, host).map(|area| {
            buf.img.width = area.width();
            buf.img.height = area.height();
            buf.offset = [buf.offset[0] - f64::from(area.left), buf.offset[1] - f64::from(area.top)];
            if area.is_empty() {
                buf.img = Image::new(1, 1);
            }
        }),
    };
    let finished_in_bands = passes.is_none() && rendered.is_ok();
    loaded.report(&spec.id, rendered.err().map(|e| e.to_string()));
    if finished_in_bands { buf } else { ctx.quantized(buf) }
}

fn gpu_program(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> Option<GpuProgram> {
    let layout = layout(loaded, spec, ctx, grid);
    let plan = layout.gpu.filter(|plan| plan_runs_on_gpu(spec, plan))?;
    let [left, top, right, bottom] = layout.padding;
    let frame = [grid.width.saturating_add(left).saturating_add(right), grid.height.saturating_add(top).saturating_add(bottom)];
    if let Err(why) = plan.validate(frame) {
        loaded.report(&spec.id, Some(format!("the GPU plan was refused: {why}")));
        return None;
    }
    let slot = |image: PlanImage| match image {
        PlanImage::Source => GpuSlot::Source,
        PlanImage::Output => GpuSlot::Output,
        PlanImage::Scratch(index) => GpuSlot::Scratch(index as usize),
        PlanImage::Texture(index) => GpuSlot::Texture(index as usize),
        PlanImage::Layer(index) => GpuSlot::Layer(index as usize),
    };
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
    let (lowered, temps) = kernels::lower(&plan, frame);
    let mut images = plan.images.clone();
    let first_temp = images.len();
    images.extend(temps);
    let place = |image: StepImage| match image {
        StepImage::Plan(image) => slot(image),
        StepImage::Temp(index) => GpuSlot::Scratch(first_temp + index),
    };
    let passes = lowered
        .into_iter()
        .map(|step| match step {
            Lowered::Plugin(pass) => GpuPass {
                kernel: pass.kernel.clone(),
                src: slot(pass.src),
                aux: pass.aux.map(slot),
                out: slot(pass.out),
                f: pass.f.clone(),
                u: pass.u.clone(),
                data: Vec::new(),
                draw: pass.draw.clone().map(draw),
                host: None,
            },
            Lowered::Host(step) => GpuPass {
                kernel: step.entry.to_string(),
                src: place(step.src),
                aux: step.aux.map(place),
                out: place(step.out),
                f: step.f,
                u: step.u,
                data: step.data,
                draw: None,
                host: Some(step.wgsl),
            },
        })
        .collect();
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
                PlanData::Cache(key) => GpuBytes { bytes: cached(&key)?, name: key },
                PlanData::Floats(values) => GpuBytes { name: format!("#{index}"), bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() },
                PlanData::Indices(values) => GpuBytes { name: format!("#{index}"), bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() },
            })
        })
        .collect::<Option<_>>()?;
    let textures = plan
        .textures
        .into_iter()
        .map(|texture| Some(GpuPicture { bytes: cached(&texture.key)?, name: texture.key, size: texture.size }))
        .collect::<Option<_>>()?;
    Some(GpuProgram { padding: layout.padding, images, buffers, textures, layers: plan.layers, passes })
}
