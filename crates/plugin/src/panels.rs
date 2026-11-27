use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use craft_plugin_host::ui::{AppEvent, AskedForm, Document, Gesture, Surfaces, Values};
pub use craft_plugin_host::ui::{Owner, Shown};
use craft_plugin_host::{Call, FrameRequest, Host, Pixels, Plugin};
use effectcraft_engine::project::{ItemId, LayerId, Project, Property};
use effectcraft_engine::{History, Session};
use serde_json::{Value, json};

use crate::gate::SessionHost;
use crate::manage::{Loaded, plugins, running};

pub(crate) fn run_command(session: &mut Session, loaded: Arc<Loaded>, command: &str) -> Result<(), String> {
    loaded.runnable(command)?;
    let call = Call::Command { command: command.to_string() };
    craft_plugin_host::ui::run_now(&mut Doc::new(session), &loaded.plugin, call, Gesture::command(&loaded.plugin, command))
}

pub(crate) fn press_now(session: &mut Session, layer: u64, prop: u64) -> Result<(), String> {
    let (plugin, call, gesture) = press(session, layer, prop)?;
    craft_plugin_host::ui::run_now(&mut Doc::new(session), &plugin, call, gesture)
}

pub fn asked_form<T>(session: &Session, look: impl FnOnce(&mut AskedForm) -> T) -> Option<T> {
    craft_plugin_host::ui::asked_form(&session.project_closed, look)
}

pub fn answer_form(session: &Session, values: Option<Values>) -> Result<(), String> {
    craft_plugin_host::ui::answer_form(&session.project_closed, values)
}

#[derive(Clone)]
struct Pressed {
    comp: ItemId,
    layer: u64,
    prop: u64,
}

struct Press {
    effect: String,
    param: String,
    instance: u64,
    name: String,
}

fn pressable(session: &Session, &Pressed { comp, layer, prop }: &Pressed) -> Result<Press, String> {
    use effectcraft_engine::commands::expr_tools::{effect_param, eval_ctx, param_state};
    let found = session.project.comp(comp).ok_or("its composition was closed or deleted")?;
    let on = found.layer(LayerId(layer)).ok_or_else(|| format!("layer {layer} was deleted"))?;
    let (root, param) = effect_param(on, prop).ok_or_else(|| format!("its effect was removed from layer {layer}"))?;
    let button = on.props.find(prop).filter(|pr| matches!(pr.ui, effectcraft_engine::project::ParamUi::Button));
    let name = button.ok_or_else(|| format!("@{prop} is not a plug-in button"))?.name.clone();
    match param_state(&eval_ctx(session, comp, found, session.time_of(comp)), on, root, &param) {
        [true, true] => Ok(Press { effect: root.match_id.clone(), param, instance: root.uid, name }),
        _ => Err(format!("the button `{name}` is hidden or turned off by another control")),
    }
}

fn press(session: &Session, layer: u64, prop: u64) -> Result<(Arc<Plugin>, Call, Gesture), String> {
    let pressed = Pressed { comp: session.active_comp_id().ok_or("no composition is open")?, layer, prop };
    let Press { effect, param, instance, name } = pressable(session, &pressed)?;
    let (_, loaded, local) = crate::manage::effect_owner(&effect)?;
    let gesture = Gesture::new(&name).keeping_own().with(Step { pressed: Some(pressed), last: None });
    Ok((loaded.plugin.clone(), Call::Button { effect: local, param, instance, layer }, gesture))
}

#[derive(Default)]
struct Step {
    pressed: Option<Pressed>,
    last: Option<Arc<Project>>,
}

fn admit(session: &Session, gesture: &Gesture) -> Result<(), String> {
    let (pressed, last) = gesture.state(|step: &mut Step| (step.pressed.clone(), step.last.clone()));
    let Some(pressed) = pressed else { return Ok(()) };
    let stopped = |why: String| format!("the press of `{}` stopped: {why}", gesture.label());
    pressable(session, &pressed).map_err(stopped)?;
    match last {
        Some(after) if !Arc::ptr_eq(&after, &session.project) => {
            Err(stopped("another edit changed the project while it ran, so its later edits are dropped to keep it one undo step".into()))
        }
        _ => Ok(()),
    }
}

fn fold(session: &mut Session, gesture: &Gesture, before: &Arc<Project>, earlier: History) {
    if Arc::ptr_eq(&session.project, before) {
        return;
    }
    gesture.state(|step: &mut Step| {
        let joined = step.last.as_ref().is_some_and(|after| Arc::ptr_eq(after, before));
        let made: Vec<_> = session.history.undo.iter().filter(|(_, state)| !earlier.undo.iter().any(|(_, old)| Arc::ptr_eq(old, state))).collect();
        let own = gesture.keeps_own() && !joined && matches!(made.as_slice(), [(_, state)] if Arc::ptr_eq(state, before));
        if !own {
            session.history = earlier;
            if !joined {
                let levels = session.prefs.general.undo_levels.max(1) as usize;
                session.history.record(gesture.label(), before.clone(), levels);
            } else if let Some((label, _)) = session.history.undo.last_mut() {
                *label = gesture.label().to_string();
            }
        }
        session.history.merge_key = None;
        step.last = Some(session.project.clone());
    });
}

pub(crate) struct Doc<'a> {
    session: &'a mut Session,
    seen: Option<&'a mut Option<Seen>>,
}

impl<'a> Doc<'a> {
    pub(crate) fn new(session: &'a mut Session) -> Doc<'a> {
        Doc { session, seen: None }
    }
}

pub(crate) fn held(prop: &Property) -> Value {
    serde_json::to_value(prop).unwrap_or_default()
}

impl Document for Doc<'_> {
    fn project(&self) -> Arc<AtomicBool> {
        self.session.project_closed.clone()
    }

    fn app_events(&mut self) -> Vec<AppEvent> {
        let Some(seen) = self.seen.as_deref_mut() else { return Vec::new() };
        let now = Seen::of(self.session);
        let events = seen.as_ref().map(|before| before.changes(&now)).unwrap_or_default();
        *seen = Some(now);
        events
    }

    fn admit(&mut self, gesture: &Gesture) -> Result<(), String> {
        admit(self.session, gesture)
    }

    fn serve(&mut self, plugin: &Arc<Plugin>, gesture: Option<&Gesture>, call: &mut dyn FnMut(&mut dyn Host)) -> Result<(), String> {
        let loaded = plugins()
            .and_then(|plugins| plugins.get(&plugin.manifest().id))
            .filter(|loaded| Arc::ptr_eq(&loaded.plugin, plugin))
            .ok_or_else(|| format!("plugin `{}` is not running", plugin.manifest().id))?;
        let (before, earlier) = (self.session.project.clone(), gesture.map(|_| self.session.history.clone()));
        call(&mut SessionHost::new(self.session, &loaded));
        if let Some((gesture, earlier)) = gesture.zip(earlier) {
            fold(self.session, gesture, &before, earlier);
        }
        Ok(())
    }

    fn holds(&self, edit: &Value) -> Value {
        let id = |key: &str| edit.get(key).and_then(Value::as_u64);
        let (Some(comp), Some(layer), Some(prop)) = (id("comp"), id("layer"), id("prop")) else { return Value::Null };
        let found = self.session.project.comp(ItemId(comp)).and_then(|comp| comp.layer(LayerId(layer))).and_then(|layer| layer.props.find(prop));
        found.map_or(Value::Null, held)
    }

    fn write(&mut self, edit: &Value, value: String) -> Result<(), String> {
        let mut params = edit.clone();
        params["value"] = json!(value);
        self.session.execute("prop.set", params).map(drop).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub comp: u64,
    pub layer: u64,
    pub prop: u64,
    pub time: f64,
    pub merge: String,
}

impl Target {
    pub(crate) fn edit(&self) -> Value {
        json!({"comp": self.comp, "layer": self.layer, "prop": self.prop, "merge": self.merge, "time": self.time, "timeBase": "comp"})
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Surface<'a> {
    Panel { plugin: &'a str, panel: &'a str, instance: Option<&'a str> },
    Control { effect: &'a str, param: &'a str, instance: u64, value: &'a str, edit: Option<&'a Target> },
}

impl Surface<'_> {
    pub fn owner(&self) -> Owner {
        match *self {
            Surface::Panel { plugin, panel, instance } => Owner::panel(plugin, panel, instance),
            Surface::Control { instance, .. } => Owner::Control(instance),
        }
    }
}

fn find(surface: Surface) -> Result<(Arc<Plugin>, String), String> {
    match surface {
        Surface::Panel { plugin, .. } => Ok((running(plugin)?.plugin.clone(), String::new())),
        Surface::Control { effect, .. } => crate::manage::effect_owner(effect).map(|(_, loaded, local)| (loaded.plugin.clone(), local)),
    }
}

fn kit<'a>(surface: Surface<'a>, local: &'a str) -> craft_plugin_host::ui::Surface<'a> {
    use craft_plugin_host::ui::Surface as Kit;
    match surface {
        Surface::Panel { panel, instance, .. } => Kit::Panel { panel, instance },
        Surface::Control { param, instance, value, edit, .. } => Kit::Control { effect: local, param, instance, value, edit: edit.map(Target::edit) },
    }
}

#[derive(Default)]
pub struct Panels {
    pub(crate) surfaces: Surfaces,
    seen: Option<Seen>,
}

impl std::ops::Deref for Panels {
    type Target = Surfaces;

    fn deref(&self) -> &Surfaces {
        &self.surfaces
    }
}

impl std::ops::DerefMut for Panels {
    fn deref_mut(&mut self) -> &mut Surfaces {
        &mut self.surfaces
    }
}

impl Panels {
    pub fn frame(&mut self, session: &mut Session, now: Duration) -> Option<Duration> {
        self.surfaces.frame(&mut Doc { session, seen: Some(&mut self.seen) }, now)
    }

    pub fn show(&mut self, session: &mut Session, surface: Surface, size: [f32; 2]) -> Result<Shown<'_>, String> {
        let (plugin, local) = find(surface)?;
        self.surfaces.show(&mut Doc::new(session), &plugin, kit(surface, &local), size)
    }

    pub fn close(&mut self, session: &Session, surface: Surface) {
        let plugin = find(surface).ok().map(|(plugin, _)| plugin);
        self.surfaces.close(&session.project_closed, &surface.owner(), plugin.as_ref());
    }

    pub fn command(&mut self, session: &Session, plugin: &str, command: &str) -> Result<(), String> {
        let loaded = running(plugin)?;
        loaded.runnable(command)?;
        self.surfaces.command(&session.project_closed, &loaded.plugin, command);
        Ok(())
    }

    pub fn press(&mut self, session: &Session, layer: u64, prop: u64) -> Result<(), String> {
        let (plugin, call, gesture) = press(session, layer, prop)?;
        self.surfaces.press(&session.project_closed, &plugin, call, gesture);
        Ok(())
    }
}

struct Seen {
    project_id: usize,
    path: Option<String>,
    comp: Option<ItemId>,
    time: Option<f64>,
    selection: AppEvent,
    project: Arc<Project>,
}

impl Seen {
    fn of(session: &Session) -> Seen {
        let comp = session.active_comp_id();
        let state = &session.state;
        let selection = AppEvent::Selection {
            layers: state.selected_layers.iter().map(|layer| layer.0).collect(),
            properties: state.selected_props.iter().map(|(layer, uid)| [layer.0, *uid]).collect(),
            keyframes: state.selected_keys.len() as u32,
        };
        Seen {
            project_id: Arc::as_ptr(&session.project_closed) as usize,
            path: session.path.clone(),
            comp,
            time: comp.map(|comp| session.time_of(comp).seconds()),
            selection,
            project: session.project.clone(),
        }
    }

    fn changes(&self, next: &Seen) -> Vec<AppEvent> {
        let mut events = Vec::new();
        let reopened = self.project_id != next.project_id;
        if reopened || self.path != next.path {
            events.push(AppEvent::Project { path: next.path.clone() });
        }
        if self.comp != next.comp {
            events.push(AppEvent::Comp { comp: next.comp.map(|comp| comp.0) });
        }
        if let (Some(comp), Some(time)) = (next.comp, next.time)
            && (self.comp != next.comp || self.time != next.time)
        {
            events.push(AppEvent::Time { comp: comp.0, time });
        }
        if self.selection != next.selection {
            events.push(next.selection.clone());
        }
        if !reopened && !Arc::ptr_eq(&self.project, &next.project) {
            events.push(AppEvent::Edit { layers: changed_layers(&self.project, &next.project, next.comp) });
        }
        events
    }
}

fn changed_layers(before: &Project, after: &Project, comp: Option<ItemId>) -> Vec<u64> {
    let Some(comp) = comp else { return Vec::new() };
    let layers = |project: &Project| -> Vec<(u64, *const effectcraft_engine::project::Layer)> {
        project.comp(comp).map(|comp| comp.layers.iter().map(|layer| (layer.id.0, layer as *const _)).collect()).unwrap_or_default()
    };
    let (old, new) = (layers(before), layers(after));
    let mut changed: Vec<u64> = new.iter().filter(|layer| !old.contains(layer)).map(|(id, _)| *id).collect();
    changed.extend(old.iter().filter(|(id, _)| !new.iter().any(|(other, _)| other == id)).map(|(id, _)| *id));
    changed.sort_unstable();
    changed.dedup();
    changed
}

pub fn accept_drop(plugin: &str, paths: &[String]) {
    let Some(loaded) = plugins().and_then(|plugins| plugins.get(plugin)) else { return };
    for path in paths {
        if let Err(error) = loaded.plugin.accept_drop(std::path::Path::new(path)) {
            log::warn!("{path} was dropped on `{plugin}` but stays closed to it: {error}");
        }
    }
}

pub fn drag_path(plugin: &str, root: craft_plugin_host::ui::Root, path: &str) -> Option<String> {
    use craft_plugin_host::ui::Root;
    let loaded = plugins()?.get(plugin)?;
    let relative = std::path::Path::new(path);
    let plain = !path.is_empty() && relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
    let full = match root {
        Root::Bundle if plain => loaded.bundle.dir.join(relative),
        Root::Data if plain => loaded.data_dir.join(relative),
        Root::User if relative.is_absolute() && loaded.bundle.manifest.permissions.contains(&craft_plugin_host::Permission::FilesRead) => {
            relative.to_path_buf()
        }
        _ => return None,
    };
    full.is_file().then(|| full.to_string_lossy().into_owned())
}

pub fn panel_frame(session: &Session, plugin: &str, request: &FrameRequest) -> Result<craft_plugin_host::Job<Pixels>, String> {
    let loaded = plugins().and_then(|plugins| plugins.get(plugin)).ok_or_else(|| format!("the plug-in `{plugin}` is not running"))?;
    if !loaded.plugin.granted(craft_plugin_host::Permission::DocumentRead) {
        return Err("a frame needs the plug-in's \"document.read\" permission; add it to `permissions` in plugin.json".into());
    }
    crate::gate::frame_job(session, request)
}
