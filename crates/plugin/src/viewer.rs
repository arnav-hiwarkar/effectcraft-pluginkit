use std::sync::Arc;

pub use craft_plugin_host::viewer::{Bind, Cursor, Handle, Mark, Overlay, Pen, Space, ToolEvent, ToolEventKind, ToolSpec};
use craft_plugin_host::{RenderCx, Value as Wire};
use effectcraft_engine::Session;
use effectcraft_engine::effects::dynamic::Grid;
use effectcraft_engine::project::{GroupKind, Layer, PropGroup};
use effectcraft_engine::render::{EvalCtx, effect_instance};
use serde_json::{Value, json};

use crate::effects::wire_params;
use crate::manage::{Loaded, effect_owner, running, running_plugins};
use crate::panels::{Doc, Panels, Target, held};

pub use craft_plugin_host::viewer::Shown;

fn point(shown: &Shown, param: &str) -> Option<[f64; 2]> {
    match shown.cx.params.get(param) {
        Some(Wire::List(values)) if values.len() >= 2 => Some([values[0], values[1]]),
        _ => None,
    }
}

fn number(shown: &Shown, param: &str) -> f64 {
    match shown.cx.params.get(param) {
        Some(Wire::Number(n)) => *n,
        _ => 0.0,
    }
}

impl Panels {
    pub fn overlay(&mut self, session: &mut Session, ctx: &EvalCtx, layer: &Layer, group: &PropGroup) -> Option<Arc<Shown>> {
        let GroupKind::Effect { effect } = &group.kind else { return None };
        let (_, loaded, local) = effect_owner(effect).ok()?;
        let spec = loaded.plugin.contributions().effects.iter().find(|spec| spec.id == local)?.clone();
        let layer_size = ctx.effect_bounds(layer).0;
        let params = effectcraft_engine::effects::flatten_params(group, &mut |prop| ctx.value(layer, prop));
        let size = match layer.switches.adjustment {
            true => [ctx.comp.width, ctx.comp.height],
            false => layer_size.map(|side| side.round().max(0.0) as u32),
        };
        let grid = Grid { width: size[0], height: size[1], offset: [0.0; 2], scale: 1.0 };
        let cx = RenderCx {
            params: wire_params(&spec, &params, layer_size, grid),
            time: layer.layer_time(ctx.time).seconds(),
            comp_time: ctx.time.seconds(),
            frame_rate: ctx.comp.frame_rate.as_f64(),
            scale: 1.0,
            layer_size,
            offset: [0.0; 2],
            size,
            instance: effect_instance(layer, group),
            float: false,
            linear: false,
        };
        self.surfaces.overlay(&mut Doc::new(session), &loaded.plugin, &local, cx, [ctx.comp_id.0, layer.id.0])
    }

    pub fn drag(&mut self, shown: &Shown, handle: usize, delta: [f64; 2], group: &PropGroup, merge: &str) -> Result<Vec<(String, Value)>, String> {
        let handle = shown.overlay.handles.get(handle).ok_or_else(|| format!("the overlay has no handle {handle}"))?;
        let to = [handle.at[0] + delta[0], handle.at[1] + delta[1]];
        let uid = |param: &str| group.prop(param).map(|prop| prop.uid).ok_or_else(|| format!("the effect has no parameter `{param}`"));
        let set = |param: &str, value: Value| -> Result<(String, Value), String> {
            Ok(("prop.set".to_string(), json!({"layer": shown.place[1], "prop": uid(param)?, "value": value, "merge": merge})))
        };
        match &handle.bind {
            Bind::Point { param } => Ok(vec![set(param, json!(to))?]),
            Bind::Angle { param, centre } => {
                let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
                let turn = (angle(to) - angle(handle.at) + 180.0).rem_euclid(360.0) - 180.0;
                Ok(vec![set(param, json!(number(shown, param) + turn))?])
            }
            Bind::Radius { param, centre, unit } => {
                if !(unit.is_finite() && *unit > 0.0) {
                    return Err(format!("the radius handle of `{param}` needs a unit above 0, not {unit}"));
                }
                Ok(vec![set(param, json!((to[0] - centre[0]).hypot(to[1] - centre[1]) / unit))?])
            }
            Bind::Rect { min, max } => {
                [min, max].into_iter().filter_map(|param| point(shown, param).map(|[x, y]| set(param, json!([x + delta[0], y + delta[1]])))).collect()
            }
            Bind::Custom { param, key } => {
                let prop = group.prop(param).ok_or_else(|| format!("the effect has no parameter `{param}`"))?;
                let edit = Target { comp: shown.place[0], layer: shown.place[1], prop: prop.uid, time: shown.cx.comp_time, merge: merge.to_string() };
                self.surfaces.drag(shown, param, key, to, edit.edit(), held(prop));
                Ok(Vec::new())
            }
        }
    }

    pub fn tool(&mut self, session: &Session, plugin: &str, tool: &str, event: ToolEvent) -> Result<(), String> {
        self.surfaces.tool(&session.project_closed, &running(plugin)?.plugin, tool, event)
    }

    pub fn tool_overlay(&mut self, session: &mut Session) -> Option<&Overlay> {
        self.surfaces.tool_overlay(&mut Doc::new(session))
    }
}

pub fn tools() -> Vec<(String, ToolSpec)> {
    let of =
        |loaded: &Arc<Loaded>| loaded.plugin.contributions().tools.iter().map(|tool| (loaded.bundle.manifest.id.clone(), tool.clone())).collect::<Vec<_>>();
    running_plugins().iter().flat_map(of).collect()
}
