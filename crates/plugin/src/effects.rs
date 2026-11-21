use std::collections::BTreeMap;
use std::sync::Arc;

use craft_plugin_host::{Call, EffectSpec, GpuImage, Host, Layout, MaskPath, Param, ParamKind, Pixels, RenderCx, Scene, Value as Wire};
use effectcraft_engine::effects::dynamic::{self, DynamicEffect, DynamicParam, GpuPass, GpuProgram, GpuSlot, Grid};
use effectcraft_engine::effects::{Buf, EffectCtx, Image, LayerPixels, MaskShape, util};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::ParamUi;

use crate::manage::Loaded;

const MAX_PADDING: u32 = 4096;

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
            revision: loaded.revision,
        });
    }
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

fn wire_params(spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> BTreeMap<String, Wire> {
    let convert = |param: &Param| -> Wire {
        let fallback;
        let value = match ctx.params.get(&param.id) {
            Some(value) => value,
            None => {
                fallback = declared_default(param, ctx.layer_size);
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
        params: wire_params(spec, ctx, grid),
        time: ctx.time,
        comp_time: ctx.env.comp_time,
        frame_rate: ctx.fps(),
        scale: grid.scale,
        layer_size: ctx.layer_size,
        offset: grid.offset,
        size: [grid.width, grid.height],
        instance: u64::from(ctx.seed),
        float: ctx.env.float_depth,
        linear: ctx.env.working_linear,
    }
}

struct RenderHost<'a, 'b> {
    ctx: &'a EffectCtx<'b>,
    grid: Grid,
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

    fn scene(&mut self) -> Option<Scene> {
        let scene = self.ctx.env.host?.comp_scene()?;
        Some(Scene { projection: scene.camera, camera_layer: scene.camera_layer })
    }
}

fn layout(loaded: &Loaded, spec: &EffectSpec, ctx: &EffectCtx, grid: Grid) -> Layout {
    let call = Call::Layout { effect: spec.id.clone(), cx: render_cx(spec, ctx, grid) };
    let mut layout: Layout = loaded.plugin.call(&call, None, &mut RenderHost { ctx, grid }).unwrap_or_default();
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
    let rendered = loaded.plugin.render(&spec.id, &cx, layout.reach, &mut buf.img.data, || RenderHost { ctx, grid });
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
    };
    let passes = plan
        .passes
        .into_iter()
        .map(|pass| GpuPass { kernel: pass.kernel, src: slot(pass.src), aux: pass.aux.map(slot), out: slot(pass.out), f: pass.f, u: pass.u })
        .collect();
    Some(GpuProgram { padding: layout.padding, images: plan.images, passes })
}
