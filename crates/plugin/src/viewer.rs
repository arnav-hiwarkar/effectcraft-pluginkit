use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use craft_plugin_host::viewer::{Bind, Overlay, ToolEvent, ToolEventKind, ToolSpec};
use craft_plugin_host::{Call, Host, RenderCx, Value as Wire};
use effectcraft_engine::Session;
use effectcraft_engine::effects::dynamic::Grid;
use effectcraft_engine::project::{GroupKind, Layer, Project, PropGroup};
use effectcraft_engine::render::{EvalCtx, effect_instance};
use serde_json::{Value, json};

use crate::effects::wire_params;
use crate::gate::SessionHost;
use crate::manage::{Loaded, effect_owner, running, running_plugins};

const MAX_RETAINED: usize = 256;

pub struct Shown {
    pub overlay: Overlay,
    pub plugin: String,
    cx: RenderCx,
    loaded: Arc<Loaded>,
    effect: String,
}

struct Quiet;

impl Host for Quiet {}

type Retained = HashMap<(String, u64), (Vec<u8>, Arc<Shown>)>;

static RETAINED: LazyLock<Mutex<Retained>> = LazyLock::new(Mutex::default);
static GESTURE: Mutex<Option<Arc<Project>>> = Mutex::new(None);

fn hold<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn overlay(ctx: &EvalCtx, layer: &Layer, group: &PropGroup) -> Option<Arc<Shown>> {
    let GroupKind::Effect { effect } = &group.kind else { return None };
    let (plugin, loaded, local) = effect_owner(effect).ok()?;
    let spec = loaded.plugin.contributions().effects.iter().find(|spec| spec.id == local)?.clone();
    let layer_size = ctx.effect_bounds(layer).0;
    let params = effectcraft_engine::effects::flatten_params(group, &mut |prop| ctx.value(layer, prop));
    let size = layer_size.map(|side| side.round().max(0.0) as u32);
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
    let call = Call::Overlay { effect: local.clone(), cx };
    let request = serde_json::to_vec(&call).ok()?;
    let key = (plugin.clone(), effect_instance(layer, group));
    if let Some((seen, shown)) = hold(&RETAINED).get(&key)
        && *seen == request
        && Arc::ptr_eq(&shown.loaded, &loaded)
    {
        return Some(shown.clone());
    }
    let overlay = loaded.plugin.call(&call, None, &mut Quiet).unwrap_or_else(|error| {
        log::debug!("plugin `{plugin}` effect `{local}` draws no overlay: {error}");
        Overlay::default()
    });
    let Call::Overlay { cx, .. } = call else { return None };
    let shown = Arc::new(Shown { overlay, plugin, cx, loaded, effect: local });
    let mut retained = hold(&RETAINED);
    if retained.len() >= MAX_RETAINED {
        retained.clear();
    }
    retained.insert(key, (request, shown.clone()));
    Some(shown)
}

impl Shown {
    pub fn binds(&self, param: &str) -> bool {
        self.overlay.handles.iter().any(|handle| handle.bind.params().contains(&param))
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

    pub fn drag(&self, handle: usize, delta: [f64; 2]) -> Result<Vec<(String, Value)>, String> {
        let handle = self.overlay.handles.get(handle).ok_or_else(|| format!("the overlay has no handle {handle}"))?;
        let to = [handle.at[0] + delta[0], handle.at[1] + delta[1]];
        Ok(match &handle.bind {
            Bind::Point { param } => vec![(param.clone(), json!(to))],
            Bind::Angle { param, centre } => {
                let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
                let turn = (angle(to) - angle(handle.at) + 180.0).rem_euclid(360.0) - 180.0;
                vec![(param.clone(), json!(self.number(param) + turn))]
            }
            Bind::Radius { param, centre, unit } => {
                if !(unit.is_finite() && *unit > 0.0) {
                    return Err(format!("the radius handle of `{param}` needs a unit above 0, not {unit}"));
                }
                vec![(param.clone(), json!((to[0] - centre[0]).hypot(to[1] - centre[1]) / unit))]
            }
            Bind::Rect { min, max } => {
                [min, max].into_iter().filter_map(|param| self.point(param).map(|[x, y]| (param.clone(), json!([x + delta[0], y + delta[1]])))).collect()
            }
            Bind::Custom { param, key } => {
                let value = match self.cx.params.get(param) {
                    Some(Wire::Text(text)) => text.clone(),
                    _ => String::new(),
                };
                let call = Call::Drag { effect: self.effect.clone(), cx: self.cx.clone(), key: key.clone(), to, value };
                let changed: Option<String> = self.loaded.plugin.call(&call, None, &mut Quiet).map_err(|e| e.to_string())?;
                changed.map(|text| (param.clone(), json!(text))).into_iter().collect()
            }
        })
    }

    pub fn drag_commands(&self, handle: usize, delta: [f64; 2], layer: &Layer, group: &PropGroup, merge: &str) -> Result<Vec<(String, Value)>, String> {
        self.drag(handle, delta)?
            .into_iter()
            .map(|(param, value)| {
                let prop = group.prop(&param).ok_or_else(|| format!("the effect has no parameter `{param}`"))?;
                Ok(("prop.set".to_string(), json!({"layer": layer.id.0, "prop": prop.uid, "value": value, "merge": merge})))
            })
            .collect()
    }
}

pub fn tools() -> Vec<(String, ToolSpec)> {
    let of =
        |loaded: &Arc<Loaded>| loaded.plugin.contributions().tools.iter().map(|tool| (loaded.bundle.manifest.id.clone(), tool.clone())).collect::<Vec<_>>();
    running_plugins().iter().flat_map(of).collect()
}

pub fn tool_event(session: &mut Session, plugin: &str, tool: &str, event: &ToolEvent) -> Result<Overlay, String> {
    let loaded = running(plugin)?;
    let name = loaded
        .plugin
        .contributions()
        .tools
        .iter()
        .find(|spec| spec.id == tool)
        .map(|spec| spec.name.clone())
        .ok_or_else(|| format!("plugin `{plugin}` has no tool `{tool}`"))?;
    if event.kind == ToolEventKind::Down {
        *hold(&GESTURE) = Some(session.project.clone());
    }
    let call = Call::Tool { tool: tool.to_string(), event: event.clone() };
    let reply = loaded.plugin.call(&call, None, &mut SessionHost::new(session, &loaded)).map_err(|e| e.to_string());
    if event.kind == ToolEventKind::Up
        && let Some(start) = hold(&GESTURE).take()
    {
        fold_history(session, &start, &name);
    }
    reply
}

fn fold_history(session: &mut Session, start: &Arc<Project>, label: &str) {
    let undo = &mut session.history.undo;
    let Some(first) = undo.iter().rposition(|(_, state)| Arc::ptr_eq(state, start)) else { return };
    if undo.len() > first + 1 {
        undo.truncate(first + 1);
        undo[first].0 = label.to_string();
    }
    session.history.merge_key = None;
}
