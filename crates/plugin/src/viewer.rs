use std::sync::Arc;

pub use craft_plugin_host::viewer::{Bind, Cursor, Handle, Mark, Overlay, Space, ToolEvent, ToolEventKind, ToolSpec};
use craft_plugin_host::{Call, RenderCx, Value as Wire};
use effectcraft_engine::Session;
use effectcraft_engine::effects::dynamic::Grid;
use effectcraft_engine::project::{GroupKind, Layer, PropGroup};
use effectcraft_engine::render::{EvalCtx, effect_instance};
use serde_json::{Value, json};

use crate::effects::wire_params;
use crate::manage::{Loaded, effect_owner, running, running_plugins};
use crate::panels::{Bound, Gesture, Key, Overlaid, Panels, Request, Target, Tooling};

const MAX_RETAINED: usize = 256;

pub struct Shown {
    pub overlay: Overlay,
    pub plugin: String,
    pub(crate) cx: RenderCx,
    pub(crate) bound: Bound,
    pub(crate) effect: String,
    pub(crate) place: [u64; 2],
}

impl Shown {
    pub fn binds(&self, param: &str) -> bool {
        self.overlay.handles.iter().any(|handle| handle.bind.params().contains(&param))
    }

    pub fn continues(&self, grabbed: &Shown, handle: usize) -> bool {
        let bind = |shown: &Shown| shown.overlay.handles.get(handle).map(|handle| handle.bind.clone());
        self.bound.same(&grabbed.bound) && self.cx.instance == grabbed.cx.instance && bind(self).is_some() && bind(self) == bind(grabbed)
    }

    fn point(&self, param: &str) -> Option<[f64; 2]> {
        match self.cx.params.get(param) {
            Some(Wire::List(values)) if values.len() >= 2 => Some([values[0], values[1]]),
            _ => None,
        }
    }

    fn number(&self, param: &str) -> f64 {
        match self.cx.params.get(param) {
            Some(Wire::Number(n)) => *n,
            _ => 0.0,
        }
    }
}

impl Panels {
    pub fn overlay(&mut self, session: &mut Session, ctx: &EvalCtx, layer: &Layer, group: &PropGroup) -> Option<Arc<Shown>> {
        let GroupKind::Effect { effect } = &group.kind else { return None };
        let (plugin, loaded, local) = effect_owner(effect).ok()?;
        let spec = loaded.plugin.contributions().effects.iter().find(|spec| spec.id == local)?.clone();
        let layer_size = ctx.effect_bounds(layer).0;
        let params = effectcraft_engine::effects::flatten_params(group, &mut |prop| ctx.value(layer, prop));
        let size = match layer.switches.adjustment {
            true => [ctx.comp.width, ctx.comp.height],
            false => layer_size.map(|side| side.round().max(0.0) as u32),
        };
        let grid = Grid { width: size[0], height: size[1], offset: [0.0; 2], scale: 1.0 };
        let instance = effect_instance(layer, group);
        let cx = RenderCx {
            params: wire_params(&spec, &params, layer_size, grid),
            time: layer.layer_time(ctx.time).seconds(),
            comp_time: ctx.time.seconds(),
            frame_rate: ctx.comp.frame_rate.as_f64(),
            scale: 1.0,
            layer_size,
            offset: [0.0; 2],
            size,
            instance,
            float: false,
            linear: false,
        };
        let call = Call::Overlay { effect: local, cx };
        let request = serde_json::to_vec(&call).ok()?;
        let key = (plugin.clone(), instance);
        if self.overlays.len() >= MAX_RETAINED && !self.overlays.contains_key(&key) {
            self.overlays.clear();
        }
        let fresh = || {
            let open = Arc::default();
            Overlaid {
                bound: Bound::of(loaded.clone(), session, Arc::downgrade(&open)),
                open,
                request: Vec::new(),
                shown: None,
                place: [ctx.comp_id.0, layer.id.0],
            }
        };
        let slot = self.overlays.entry(key.clone()).or_insert_with(fresh);
        if !slot.bound.from(&loaded, session) {
            *slot = fresh();
        }
        let lane = Key::Effect(plugin, instance);
        if slot.request != request {
            slot.request = request;
            let bound = slot.bound.clone();
            self.post(Request { key: lane.clone(), bound, call, edit: None, gesture: None });
        }
        self.wait(session, |panels| !panels.lanes.contains_key(&lane));
        self.overlays.get(&key)?.shown.clone()
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
                Ok(vec![set(param, json!(shown.number(param) + turn))?])
            }
            Bind::Radius { param, centre, unit } => {
                if !(unit.is_finite() && *unit > 0.0) {
                    return Err(format!("the radius handle of `{param}` needs a unit above 0, not {unit}"));
                }
                Ok(vec![set(param, json!((to[0] - centre[0]).hypot(to[1] - centre[1]) / unit))?])
            }
            Bind::Rect { min, max } => {
                [min, max].into_iter().filter_map(|param| shown.point(param).map(|[x, y]| set(param, json!([x + delta[0], y + delta[1]])))).collect()
            }
            Bind::Custom { param, key } => {
                let value = match shown.cx.params.get(param) {
                    Some(Wire::Text(text)) => text.clone(),
                    _ => String::new(),
                };
                let edit = Target { comp: shown.place[0], layer: shown.place[1], prop: uid(param)?, time: shown.cx.comp_time, merge: merge.to_string() };
                let call = Call::Drag { effect: shown.effect.clone(), cx: shown.cx.clone(), key: key.clone(), to, value };
                let lane = Key::Effect(shown.plugin.clone(), shown.cx.instance);
                self.post(Request { key: lane, bound: shown.bound.clone(), call, edit: Some(edit), gesture: None });
                Ok(Vec::new())
            }
        }
    }

    pub fn tool(&mut self, session: &Session, plugin: &str, tool: &str, event: ToolEvent) -> Result<(), String> {
        let loaded = running(plugin)?;
        let name = loaded
            .plugin
            .contributions()
            .tools
            .iter()
            .find(|spec| spec.id == tool)
            .map(|spec| spec.name.clone())
            .ok_or_else(|| format!("plugin `{plugin}` has no tool `{tool}`"))?;
        let key = Key::Tool(plugin.to_string(), tool.to_string());
        if self.tool.as_ref().is_some_and(|state| state.key != key || !Arc::ptr_eq(&state.loaded, &loaded)) {
            self.tool = None;
        }
        let state = self.tool.get_or_insert_with(|| Tooling {
            key: key.clone(),
            loaded: loaded.clone(),
            open: Arc::default(),
            overlay: Overlay::default(),
            gesture: None,
        });
        if event.kind == ToolEventKind::Down {
            state.gesture = Some(Gesture::new(&name));
        }
        let gesture = match event.kind {
            ToolEventKind::Up => state.gesture.take(),
            _ => state.gesture.clone(),
        };
        let bound = Bound::of(loaded, session, Arc::downgrade(&state.open));
        let call = Call::Tool { tool: tool.to_string(), event };
        self.post(Request { key, bound, call, edit: None, gesture });
        Ok(())
    }

    pub fn tool_overlay(&mut self, session: &mut Session) -> Option<&Overlay> {
        let key = self.tool.as_ref()?.key.clone();
        self.wait(session, |panels| !panels.lanes.contains_key(&key));
        self.tool.as_ref().map(|state| &state.overlay)
    }

    pub fn put_tool_away(&mut self) {
        self.tool = None;
    }
}

pub fn tools() -> Vec<(String, ToolSpec)> {
    let of =
        |loaded: &Arc<Loaded>| loaded.plugin.contributions().tools.iter().map(|tool| (loaded.bundle.manifest.id.clone(), tool.clone())).collect::<Vec<_>>();
    running_plugins().iter().flat_map(of).collect()
}
