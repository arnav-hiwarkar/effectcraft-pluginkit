use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use craft_plugin_host::kernels::{self, Lowered, StepImage};
use craft_plugin_host::{
    AuxPlanes, Blend, Call, Camera, Channel, Cull, Draw, EffectRegistry, EffectSpec, Focus, Gradient as WireGradient, Host, LayerPlacement, Layout, Light,
    LightKind, MaskPath, Memo, Param, ParamKind, ParamValues, ParamsAt, Pixels, Place, PlanData, PlanImage, Rect, RenderCx, Scene, Value as Wire,
};
use effectcraft_engine::effects::dynamic::{
    self, DynamicEffect, DynamicParam, GpuBlend, GpuBytes, GpuCull, GpuDraw, GpuPass, GpuPicture, GpuProgram, GpuSlot, Grid, Planning,
};
use effectcraft_engine::effects::{Buf, EffectCtx, Image, LayerPixels, MaskShape, Params, SceneLightKind, util};
use effectcraft_engine::keyframe::{Gradient, Value};
use effectcraft_engine::project::ParamUi;
use effectcraft_engine::raster::AuxChannels;
use effectcraft_engine::raster::channels3d::{self, AuxLookup};

use crate::manage::Loaded;

const PROBE_SIDE: u32 = 256;
const MAX_STAMPED_FILES: usize = 256;
const STAMP_LIFETIME: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) fn effect_id(plugin: &str, effect: &str) -> String {
    format!("{plugin}.{effect}")
}

pub(crate) struct Engine;

impl EffectRegistry for Engine {
    type Loaded = Loaded;

    fn register(&self, loaded: &Arc<Loaded>) {
        register(loaded);
    }

    fn unregister(&self, plugin: &str) {
        dynamic::unregister(plugin);
    }
}

fn register(loaded: &Arc<Loaded>) {
    let plugin_id = &loaded.bundle.manifest.id;
    for spec in &loaded.plugin.contributions().effects {
        let (render_plugin, render_spec) = (loaded.clone(), spec.clone());
        let (plugin, gpu_spec) = (loaded.clone(), spec.clone());
        let (margin_plugin, margin_spec) = (loaded.clone(), spec.clone());
        let (migrate_plugin, migrate_spec) = (loaded.clone(), spec.clone());
        let gpu: dynamic::GpuDyn = Box::new(move |ctx: &EffectCtx, grid: Grid, planning| gpu_program(&plugin, &gpu_spec, ctx, grid, planning));
        let id = effect_id(plugin_id, &spec.id);
        dynamic::register(DynamicEffect {
            owner: plugin_id.clone(),
            version: loaded.bundle.manifest.version.clone(),
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
            revision: loaded.revision,
            files: Some(revision_stamp(loaded.clone())),
            margin: Some(Box::new(move |ctx, scale| margin(&margin_plugin, &margin_spec, ctx, scale))),
            state_version: spec.version,
            migrate: Some(Box::new(move |from, values| migrate(&migrate_plugin, &migrate_spec, from, values))),
            aux: Channel::unique(&spec.reads_aux).iter().flat_map(aux_lookups).collect(),
        });
        if spec.shader.is_none()
            && let Some(registered) = dynamic::get(&id)
        {
            let (probe_plugin, probe_spec) = (loaded.clone(), spec.clone());
            let _ = registered.gpu_probe.set(Box::new(move || default_plan_runs_on_gpu(&probe_plugin, &probe_spec)));
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
        ParamKind::Gradient { default } => (Value::Gradient(app_gradient(default)), ParamUi::Gradient),
        ParamKind::Curve { default, channels } => {
            (Value::Str(craft_plugin_host::default_curves_text(default, *channels)), ParamUi::Curve { channels: *channels })
        }
        ParamKind::Button => (Value::Bool(false), ParamUi::Button),
    };
    DynamicParam { id: param.id.clone(), name: param.name.clone(), default, ui }
}

fn app_gradient(gradient: &WireGradient) -> Gradient {
    Gradient {
        colors: gradient.colors.iter().map(|(at, color)| (*at, color.map(|c| c as f32))).collect(),
        opacities: gradient.opacities.iter().map(|(at, opacity)| (*at, *opacity as f32)).collect(),
    }
}

fn preset_values(spec: &EffectSpec, values: &BTreeMap<String, Wire>) -> Vec<(String, Value)> {
    spec.params.iter().filter_map(|param| Some((param.id.clone(), Placed::from_wire(param, values.get(&param.id)?.clone())?))).collect()
}

struct Placed(Grid);

impl ParamValues for Placed {
    type Value = Value;

    fn from_wire(param: &Param, value: Wire) -> Option<Value> {
        Some(match (&param.kind, param.fit(value).ok()?) {
            (ParamKind::Popup { options, .. }, Wire::Number(n)) => Value::Enum((n as u32).min(options.len().saturating_sub(1) as u32)),
            (ParamKind::Mask | ParamKind::Layer, _) => return None,
            (_, Wire::Number(n)) => Value::Scalar(n),
            (_, Wire::Bool(b)) => Value::Bool(b),
            (ParamKind::Point { .. }, Wire::List(v)) => Value::Vec2([v[0], v[1]]),
            (ParamKind::Point3 { .. }, Wire::List(v)) => Value::Vec3([v[0], v[1], v[2]]),
            (_, Wire::List(v)) => Value::Color([v[0], v[1], v[2], v[3]]),
            (_, Wire::Text(text)) => Value::Str(text),
            (_, Wire::Gradient(gradient)) => Value::Gradient(app_gradient(&gradient)),
        })
    }

    fn to_wire(&self, param: &Param, value: &Value) -> Wire {
        let grid = self.0;
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
            ParamKind::Text { .. } | ParamKind::Custom { .. } | ParamKind::Curve { .. } => match value {
                Value::Str(text) => Wire::Text(text.clone()),
                _ => Wire::Text(String::new()),
            },
            ParamKind::Layer | ParamKind::Button => Wire::Bool(value.as_layer().is_some()),
            ParamKind::Gradient { .. } => match value {
                Value::Gradient(gradient) => Wire::Gradient(WireGradient {
                    colors: gradient.colors.iter().map(|(at, color)| (*at, color.map(f64::from))).collect(),
                    opacities: gradient.opacities.iter().map(|(at, opacity)| (*at, f64::from(*opacity))).collect(),
                }),
                _ => Wire::Gradient(WireGradient::default()),
            },
        }
    }
}

pub fn plugin_param(effect: &str, param: &str) -> Option<Param> {
    let (_, loaded, local) = crate::manage::effect_owner(effect).ok()?;
    let spec = loaded.plugin.contributions().effects.iter().find(|spec| spec.id == local)?;
    spec.params.iter().find(|declared| declared.id == param).cloned()
}

pub fn param_state(param: &Param, value: &dyn Fn(&str) -> Option<Value>) -> [bool; 2] {
    let wire = |id: &str| value(id).as_ref().and_then(saved_to_wire);
    [param.visible(wire), param.enabled(wire)]
}

fn saved_to_wire(value: &Value) -> Option<Wire> {
    Some(match value {
        Value::Scalar(n) => Wire::Number(*n),
        Value::Enum(n) => Wire::Number(f64::from(*n)),
        Value::Bool(b) => Wire::Bool(*b),
        Value::Vec2(v) => Wire::List(v.to_vec()),
        Value::Vec3(v) => Wire::List(v.to_vec()),
        Value::Color(c) => Wire::List(c.to_vec()),
        Value::Str(text) => Wire::Text(text.clone()),
        Value::Gradient(gradient) => Wire::Gradient(WireGradient {
            colors: gradient.colors.iter().map(|(at, color)| (*at, color.map(f64::from))).collect(),
            opacities: gradient.opacities.iter().map(|(at, opacity)| (*at, f64::from(*opacity))).collect(),
        }),
        _ => return None,
    })
}

fn wire_to_saved(spec: &EffectSpec, id: &str, value: Wire) -> Option<Value> {
    if let Some(param) = spec.params.iter().find(|param| param.id == id) {
        return Placed::from_wire(param, value);
    }
    Some(match value {
        Wire::Number(n) => Value::Scalar(n),
        Wire::Bool(b) => Value::Bool(b),
        Wire::List(v) if v.len() == 2 => Value::Vec2([v[0], v[1]]),
        Wire::List(v) if v.len() == 3 => Value::Vec3([v[0], v[1], v[2]]),
        Wire::List(v) if v.len() == 4 => Value::Color([v[0], v[1], v[2], v[3]]),
        Wire::Text(text) => Value::Str(text),
        Wire::Gradient(gradient) => Value::Gradient(app_gradient(&gradient)),
        Wire::List(_) => return None,
    })
}

fn migrate(loaded: &Loaded, spec: &EffectSpec, from: u32, values: &mut BTreeMap<String, Value>) -> Result<(), String> {
    let sent: BTreeMap<String, Wire> = values.iter().filter_map(|(id, value)| Some((id.clone(), saved_to_wire(value)?))).collect();
    let call = Call::Migrate { effect: spec.id.clone(), from, values: sent.clone() };
    let migrated: BTreeMap<String, Wire> = loaded.plugin.call(&call, None, &mut Quiet).map_err(|error| error.to_string())?;
    for id in sent.keys().filter(|id| !migrated.contains_key(*id)) {
        values.remove(id);
    }
    for (id, value) in migrated.into_iter().filter(|(id, value)| sent.get(id) != Some(value)) {
        let saved = wire_to_saved(spec, &id, value).ok_or_else(|| format!("the migration gave `{id}` a value it cannot hold"))?;
        values.insert(id, saved);
    }
    Ok(())
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
    let placed = Placed(grid);
    let convert = |param: &Param| match params.get(&param.id) {
        Some(value) => placed.to_wire(param, value),
        None => placed.to_wire(param, &declared_default(param, layer_size)),
    };
    spec.params.iter().filter_map(|param| Some((param.id.clone(), param.fit(convert(param)).ok()?))).collect()
}

fn warn_dropped_stops(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) {
    for param in spec.params.iter().filter(|param| matches!(param.kind, ParamKind::Gradient { .. })) {
        if let Some(value @ Value::Gradient(_)) = ctx.params.get(&param.id) {
            let wire = Placed(grid).to_wire(param, value);
            loaded.plugin.warn_dropped_stops(&spec.id, &param.id, &wire);
        }
    }
}

fn render_cx(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> RenderCx {
    warn_dropped_stops(loaded, spec, ctx, grid);
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

type SourceAux = OnceLock<Option<Arc<AuxChannels>>>;

struct RenderHost<'a, 'b> {
    ctx: &'a EffectCtx<'b>,
    grid: Grid,
    spec: &'a EffectSpec,
    aux: &'a SourceAux,
}

fn source_aux(cell: &SourceAux, ctx: &EffectCtx) -> Option<Arc<AuxChannels>> {
    cell.get_or_init(|| ctx.env.host.and_then(|host| host.aux())).clone()
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

    fn cancel(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        self.ctx.env.host?.cancel()
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

    fn aux(&mut self, channels: &[Channel], rows: Option<[u32; 2]>, size: Option<[u32; 2]>) -> Result<Option<AuxPlanes>, String> {
        let Some(aux) = source_aux(self.aux, self.ctx) else { return Ok(None) };
        let Grid { width: frame_width, height: frame_height, offset, scale } = self.grid;
        let [width, height] = size.unwrap_or([frame_width, frame_height]);
        let [first, count] = rows.unwrap_or([0, height]);
        let rows = first.min(height)..first.saturating_add(count).min(height);
        let found: Vec<(Channel, Vec<&[f32]>)> = channels
            .iter()
            .filter_map(|channel| Some((channel.clone(), aux_lookups(channel).iter().map(|lookup| aux_plane(&aux, lookup)).collect::<Option<_>>()?)))
            .collect();
        let bytes = Channel::read_bytes(width, rows.len() as u32, &found.iter().map(|(channel, _)| channel.clone()).collect::<Vec<_>>())?;
        let mut planes: Vec<(Channel, Vec<f32>)> = Vec::with_capacity(found.len());
        for (channel, sources) in &found {
            let mut plane = Vec::new();
            plane.try_reserve_exact(width as usize * rows.len() * sources.len()).map_err(|_| format!("no memory for {bytes} bytes of channels"))?;
            planes.push((channel.clone(), plane));
        }
        let step = [f64::from(frame_width) / f64::from(width.max(1)), f64::from(frame_height) / f64::from(height.max(1))];
        for y in rows.clone() {
            for x in 0..width {
                let centre = [(f64::from(x) + 0.5) * step[0], (f64::from(y) + 0.5) * step[1]];
                let [ax, ay] = effectcraft_engine::effects::aux_point(scale, offset, centre);
                let at = aux.index_at(ax, ay);
                for ((channel, sources), (_, plane)) in found.iter().zip(&mut planes) {
                    let outside = if *channel == Channel::Depth { channels3d::BACKGROUND_DEPTH } else { 0.0 };
                    plane.extend(sources.iter().map(|source| at.and_then(|i| source.get(i).copied()).unwrap_or(outside)));
                }
            }
        }
        Ok(Some(AuxPlanes { width, height: rows.len() as u32, planes }))
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

    fn data(&mut self, plugin: &str, _place: Place, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.ctx.env.host.and_then(|host| host.layer_data(plugin, key)))
    }

    fn placement(&mut self, param: &str, comp_time: Option<f64>) -> Option<LayerPlacement> {
        let layer = self.ctx.params.get(param)?.as_layer()?;
        let placement = self.ctx.env.host?.placement(layer, comp_time)?;
        Some(LayerPlacement { matrix: placement.matrix, opacity: placement.opacity, size: placement.size })
    }
}

fn aux_lookups(channel: &Channel) -> Vec<AuxLookup> {
    let find = AuxLookup::find;
    match channel {
        Channel::Depth => vec![find(&channels3d::DEPTH_NAMES)],
        Channel::Normal => channels3d::NORMAL_NAMES.map(find).into(),
        Channel::Uv => channels3d::UV_NAMES[..2].iter().map(|names| find(names)).collect(),
        Channel::ObjectId => vec![find(&channels3d::OBJECT_ID_NAMES)],
        Channel::MaterialId => vec![find(&channels3d::MATERIAL_ID_NAMES)],
        Channel::Coverage => vec![find(&channels3d::COVERAGE_NAMES)],
        Channel::Background => channels3d::BACKGROUND_NAMES.map(find).into(),
        Channel::Named(name) => vec![AuxLookup::Name(name.trim().to_string())],
    }
}

fn aux_plane<'a>(aux: &'a AuxChannels, lookup: &AuxLookup) -> Option<&'a [f32]> {
    match lookup {
        AuxLookup::Name(name) => aux.get(name),
        AuxLookup::Find(names) => aux.find(&names.iter().map(String::as_str).collect::<Vec<_>>()),
        AuxLookup::Crypto { .. } => None,
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

fn layout(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid, aux: &SourceAux) -> Layout {
    staged_layout(loaded, spec, ctx, grid, aux, None)
}

fn staged_layout(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid, aux: &SourceAux, memo: Option<&Memo>) -> Layout {
    let call = Call::Layout { effect: spec.id.clone(), cx: render_cx(loaded, spec, ctx, grid) };
    let mut host = RenderHost { ctx, grid, spec, aux };
    let layout = match memo {
        Some(memo) => loaded.plugin.call_staged(&call, &mut host, memo),
        None => loaded.plugin.call(&call, None, &mut host),
    };
    let mut layout: Layout = layout.unwrap_or_default();
    layout.padding = craft_plugin_host::applied_padding(layout.padding, [grid.width, grid.height]);
    layout
}

fn margin(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, scale: f64) -> Option<f64> {
    if spec.reads_other_frames || !spec.reads_aux.is_empty() {
        return None;
    }
    let [width, height] = ctx.layer_size.map(|side| (side * scale).ceil().clamp(1.0, f64::from(u32::MAX)) as u32);
    let layout = layout(loaded, spec, ctx, Grid { width, height, offset: [0.0; 2], scale }, &SourceAux::new());
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
    let aux = SourceAux::new();
    let layout = layout(loaded, spec, ctx, grid_of(&buf), &aux);
    if !spec.reads_aux.is_empty() {
        source_aux(&aux, ctx);
    }
    if spec.shader.is_none() && layout.gpu.as_ref().is_some_and(|plan| plan_runs_on_gpu(spec, plan)) {
        mark_gpu_planned(&effect_id(&loaded.bundle.manifest.id, &spec.id));
    }
    buf.pad_sides(layout.padding);
    let grid = grid_of(&buf);
    let cx = render_cx(loaded, spec, ctx, grid);
    let host = || RenderHost { ctx, grid, spec, aux: &aux };
    let passes = layout.gpu.as_ref().filter(|plan| layout.cpu && plan.runs_on_cpu());
    let shown = shown_area(ctx, grid);
    let size = [grid.width, grid.height];
    let rendered = match passes {
        Some(plan) => loaded.plugin.run_passes(&spec.id, &cx, plan, &mut buf.img.data, host),
        None => loaded.plugin.render(&spec.id, &cx, &layout, shown, &mut buf.img.data, host).map(|area| {
            let kept = shown.map_or(Rect::of_size(size), |shown| shown.clipped(size)).union(area);
            buf.img.width = area.width();
            buf.img.height = area.height();
            buf.offset = [buf.offset[0] - f64::from(area.left), buf.offset[1] - f64::from(area.top)];
            buf.pad_sides([area.left - kept.left, area.top - kept.top, kept.right - area.right, kept.bottom - area.bottom].map(|side| side as u32));
            if kept.is_empty() {
                buf.img = Image::new(1, 1);
            }
        }),
    };
    if matches!(rendered, Err(craft_plugin_host::PluginError::Cancelled)) {
        return buf;
    }
    let finished_in_bands = passes.is_none() && rendered.is_ok();
    let stopped = dynamic::get(&effect_id(&loaded.bundle.manifest.id, &spec.id)).and_then(|r| r.gpu_stopped.get().map(|why| format!("on the GPU: {why}")));
    let error = rendered.err().map(|e| e.to_string()).or_else(|| stopped.clone());
    if loaded.report(&spec.id, error.clone())
        && error == stopped
        && let Some(stop) = stopped
    {
        loaded.plugin.warn(format!("effect `{}` {stop}", spec.id));
    }
    if finished_in_bands { buf } else { ctx.quantized(buf) }
}

fn gpu_program(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid, planning: Planning) -> Option<GpuProgram> {
    let memo = Memo::default();
    let program = planned(loaded, spec, ctx, grid, &memo);
    if planning == Planning::Frame {
        loaded.plugin.keep(memo);
    }
    program
}

fn planned(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid, memo: &Memo) -> Option<GpuProgram> {
    let layout = staged_layout(loaded, spec, ctx, grid, &SourceAux::new(), Some(memo));
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
        let bytes = loaded.plugin.cached_in(memo, key);
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
