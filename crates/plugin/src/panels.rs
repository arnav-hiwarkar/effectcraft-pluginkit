use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use craft_plugin_host::ui::{AppEvent, ControlsReply, PanelReply, Topic, UiEvent, UiInput, Widget};
use craft_plugin_host::{Access, AudioRequest, Call, FrameRequest, Host, PickFile, Pixels, Retained, Samples};
use effectcraft_engine::Session;
use effectcraft_engine::project::{ItemId, Project};
use serde_json::Value;

use crate::gate::SessionHost;
use crate::manage::{Loaded, plugins};

const WAIT: Duration = Duration::from_millis(4);
const POLL: Duration = Duration::from_micros(500);

type Job = Box<dyn FnOnce(&mut Session) + Send>;
type Waker = Arc<dyn Fn() + Send + Sync>;

struct Forwarded {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
    busy: Mutex<Vec<String>>,
}

static FORWARDED: Forwarded = Forwarded { jobs: Mutex::new(Vec::new()), ready: Condvar::new(), busy: Mutex::new(Vec::new()) };

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn serve(session: &mut Session) -> usize {
    let jobs = std::mem::take(&mut *lock(&FORWARDED.jobs));
    let served = jobs.len();
    jobs.into_iter().for_each(|job| job(session));
    served
}

fn wait_for_work(timeout: Duration) {
    let jobs = lock(&FORWARDED.jobs);
    if jobs.is_empty() {
        let _ = FORWARDED.ready.wait_timeout(jobs, timeout);
    }
}

fn idle(plugin: &str) {
    let mut busy = lock(&FORWARDED.busy);
    if let Some(at) = busy.iter().position(|id| id == plugin) {
        busy.swap_remove(at);
    }
}

fn busy(plugin: &str) -> bool {
    lock(&FORWARDED.busy).iter().any(|id| id == plugin)
}

pub(crate) fn settle(session: &mut Session, plugin: &str) {
    while busy(plugin) {
        serve(session);
        wait_for_work(POLL);
    }
}

pub trait Clipboard: Send + Sync {
    fn text(&self) -> Result<String, String>;
    fn set_text(&self, text: &str) -> Result<(), String>;
}

static CLIPBOARD: OnceLock<Box<dyn Clipboard>> = OnceLock::new();

pub fn set_clipboard(clipboard: Box<dyn Clipboard>) {
    let _ = CLIPBOARD.set(clipboard);
}

pub(crate) fn clipboard() -> Result<&'static dyn Clipboard, String> {
    CLIPBOARD.get().map(|clipboard| clipboard.as_ref()).ok_or_else(|| "this app has no clipboard".into())
}

struct Remote {
    plugin: Arc<Loaded>,
    wake: Waker,
}

impl Remote {
    fn on_app<T: Send + 'static>(&self, run: impl FnOnce(&mut SessionHost) -> T + Send + 'static) -> Result<T, String> {
        let (reply, answer) = channel();
        let plugin = self.plugin.clone();
        lock(&FORWARDED.jobs).push(Box::new(move |session: &mut Session| {
            let _ = reply.send(run(&mut SessionHost::new(session, &plugin)));
        }));
        FORWARDED.ready.notify_all();
        (self.wake)();
        answer.recv().map_err(|_| "the app closed before it answered".to_string())
    }
}

impl Host for Remote {
    fn access(&self, command: &str, params: &Value) -> Access {
        let (command, params) = (command.to_string(), params.clone());
        self.on_app(move |host| host.access(&command, &params)).unwrap_or_else(Access::Refused)
    }

    fn command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let id = id.to_string();
        self.on_app(move |host| host.command(&id, params))?
    }

    fn notify(&mut self, message: &str) {
        let message = message.to_string();
        let _ = self.on_app(move |host| host.notify(&message));
    }

    fn frame(&mut self, request: &FrameRequest) -> Result<Pixels, String> {
        let request = request.clone();
        self.on_app(move |host| host.frame(&request))?
    }

    fn audio(&mut self, request: &AudioRequest) -> Result<Samples, String> {
        let request = request.clone();
        self.on_app(move |host| host.audio(&request))?
    }

    fn pick_file(&mut self, request: &PickFile) -> Result<Option<PathBuf>, String> {
        let request = request.clone();
        self.on_app(move |host| host.pick_file(&request))?
    }

    fn project_closed(&self) -> Option<Arc<AtomicBool>> {
        self.on_app(|host| host.project_closed()).ok().flatten()
    }

    fn clipboard(&mut self) -> Result<String, String> {
        self.on_app(|host| host.clipboard())?
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        self.on_app(move |host| host.set_clipboard(&text))?
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Surface<'a> {
    Panel { plugin: &'a str, panel: &'a str },
    Control { effect: &'a str, param: &'a str, instance: u64, value: &'a str },
}

struct Slot {
    retained: Retained,
    plugin: String,
    generation: usize,
    shown: bool,
    value: Option<String>,
    edited: Option<String>,
}

struct Done {
    key: String,
    input: UiInput,
    reply: Result<(PanelReply, Option<String>), String>,
}

pub struct Shown<'a> {
    pub plugin: &'a str,
    pub widgets: &'a [Widget],
    pub error: Option<&'a str>,
    pub edited: Option<String>,
}

pub struct Panels {
    slots: HashMap<String, Slot>,
    done: (Sender<Done>, Receiver<Done>),
    wake: Option<Waker>,
    seen: Option<Seen>,
    now: Duration,
    deadline: Option<Instant>,
}

impl Default for Panels {
    fn default() -> Self {
        Panels { slots: HashMap::new(), done: channel(), wake: None, seen: None, now: Duration::ZERO, deadline: None }
    }
}

fn find(surface: Surface) -> Result<(Arc<Loaded>, Call), String> {
    let plugins = plugins().ok_or("plugins are not available")?;
    match surface {
        Surface::Panel { plugin, panel } => {
            let loaded = plugins.get(plugin).ok_or_else(|| format!("plugin `{plugin}` is not running"))?;
            Ok((loaded, Call::Panel { panel: panel.to_string(), input: UiInput::default() }))
        }
        Surface::Control { effect, param, instance, value } => {
            let (_, loaded, local) = crate::manage::effect_owner(effect)?;
            Ok((loaded, Call::Controls { effect: local, param: param.to_string(), instance, value: value.to_string(), input: UiInput::default() }))
        }
    }
}

fn with_input(call: Call, input: UiInput) -> Call {
    match call {
        Call::Panel { panel, .. } => Call::Panel { panel, input },
        Call::Controls { effect, param, instance, value, .. } => Call::Controls { effect, param, instance, value, input },
        other => other,
    }
}

fn run(loaded: &Arc<Loaded>, call: &Call, host: &mut Remote) -> Result<(PanelReply, Option<String>), String> {
    let failed = |e: craft_plugin_host::PluginError| e.to_string();
    match call {
        Call::Controls { .. } => {
            let reply: ControlsReply = loaded.plugin.call(call, None, host).map_err(failed)?;
            Ok((PanelReply { widgets: reply.widgets, ..Default::default() }, reply.value))
        }
        _ => loaded.plugin.call(call, None, host).map(|reply| (reply, None)).map_err(failed),
    }
}

impl Panels {
    pub fn has_waker(&self) -> bool {
        self.wake.is_some()
    }

    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    pub fn frame(&mut self, session: &mut Session, now: Duration) -> Option<Duration> {
        self.now = now;
        serve(session);
        self.collect();
        let running = |slot: &Slot| plugins().and_then(|p| p.get(&slot.plugin)).is_some_and(|loaded| generation(&loaded) == slot.generation);
        self.slots.retain(|_, slot| slot.retained.in_flight() || running(slot));
        for slot in self.slots.values_mut() {
            if !std::mem::take(&mut slot.shown) {
                slot.retained.hide();
            }
        }
        let wants_edits = self.slots.values().any(|slot| slot.retained.subscribed(Topic::Edit));
        let seen = Seen::of(session, wants_edits);
        if let Some(before) = &self.seen {
            for event in before.changes(&seen) {
                self.slots.values_mut().for_each(|slot| slot.retained.notify(&event));
            }
        }
        self.seen = Some(seen);
        self.deadline = Some(Instant::now() + WAIT);
        self.slots.values().filter_map(|slot| slot.retained.next_due()).min().map(|due| due.saturating_sub(now))
    }

    pub fn submit(&mut self, key: &str, events: Vec<UiEvent>, keys: &str) {
        if let Some(slot) = self.slots.get_mut(key) {
            slot.retained.push(events, keys);
        }
    }

    pub fn calls(&self, key: &str) -> u64 {
        self.slots.get(key).map_or(0, |slot| slot.retained.calls())
    }

    pub fn show(&mut self, session: &mut Session, key: &str, surface: Surface, size: [f32; 2]) -> Result<Shown<'_>, String> {
        let (loaded, call) = find(surface)?;
        let generation = generation(&loaded);
        let slot = self.slots.entry(key.to_string()).or_insert_with(|| Slot {
            retained: Retained::new(),
            plugin: loaded.bundle.manifest.id.clone(),
            generation,
            shown: false,
            value: None,
            edited: None,
        });
        if slot.generation != generation && !slot.retained.in_flight() {
            *slot = Slot { retained: Retained::new(), plugin: loaded.bundle.manifest.id.clone(), generation, shown: false, value: None, edited: None };
        }
        slot.shown = true;
        if let Surface::Control { value, .. } = surface
            && slot.value.as_deref() != Some(value)
        {
            slot.value = Some(value.to_string());
            slot.retained.invalidate();
        }
        if slot.generation == generation
            && let Some(input) = slot.retained.start(size, self.now)
        {
            self.spawn(key, loaded, with_input(call, input));
        }
        self.wait(session, key);
        let slot = self.slots.get_mut(key).ok_or("the panel closed")?;
        Ok(Shown { plugin: &slot.plugin, widgets: slot.retained.widgets(), error: slot.retained.error(), edited: slot.edited.take() })
    }

    fn spawn(&mut self, key: &str, loaded: Arc<Loaded>, call: Call) {
        let plugin = loaded.bundle.manifest.id.clone();
        lock(&FORWARDED.busy).push(plugin.clone());
        let input = match &call {
            Call::Panel { input, .. } | Call::Controls { input, .. } => input.clone(),
            _ => UiInput::default(),
        };
        let wake: Waker = self.wake.clone().unwrap_or_else(|| Arc::new(|| {}));
        let (done, owned_key, sent_input, owner) = (self.done.0.clone(), key.to_string(), input.clone(), plugin.clone());
        let spawned = std::thread::Builder::new().name("craft-plugin-panel".into()).spawn(move || {
            let mut host = Remote { plugin: loaded.clone(), wake: wake.clone() };
            let reply = run(&loaded, &call, &mut host);
            idle(&owner);
            let _ = done.send(Done { key: owned_key, input: sent_input, reply });
            FORWARDED.ready.notify_all();
            wake();
        });
        if let Err(error) = spawned {
            idle(&plugin);
            if let Some(slot) = self.slots.get_mut(key) {
                slot.retained.finish(input, Err(error.to_string()), self.now);
            }
        }
    }

    fn collect(&mut self) -> bool {
        let mut any = false;
        while let Ok(done) = self.done.1.try_recv() {
            any = true;
            let Some(slot) = self.slots.get_mut(&done.key) else { continue };
            let reply = done.reply.map(|(reply, edited)| {
                if edited.is_some() {
                    slot.edited = edited;
                }
                reply
            });
            slot.retained.finish(done.input, reply, self.now);
        }
        any
    }

    fn wait(&mut self, session: &mut Session, key: &str) {
        let Some(deadline) = self.deadline else { return };
        while self.slots.get(key).is_some_and(|slot| slot.retained.in_flight()) {
            serve(session);
            if self.collect() {
                continue;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            wait_for_work(left.min(POLL));
        }
    }
}

fn generation(loaded: &Arc<Loaded>) -> usize {
    Arc::as_ptr(loaded) as usize
}

struct Seen {
    project_id: usize,
    path: Option<String>,
    comp: Option<ItemId>,
    time: Option<f64>,
    selection: AppEvent,
    project: Option<Arc<Project>>,
}

impl Seen {
    fn of(session: &Session, keep_project: bool) -> Seen {
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
            project: keep_project.then(|| session.project.clone()),
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
        if let (false, Some(before), Some(after)) = (reopened, &self.project, &next.project)
            && !Arc::ptr_eq(before, after)
        {
            events.push(AppEvent::Edit { layers: changed_layers(before, after, next.comp) });
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

pub fn drag_path(plugin: &str, root: craft_plugin_host::ui::Root, path: &str) -> Option<String> {
    use craft_plugin_host::ui::Root;
    let loaded = plugins()?.get(plugin)?;
    let relative = std::path::Path::new(path);
    let plain = !path.is_empty() && relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
    let full = match root {
        Root::Bundle if plain => loaded.bundle.dir.join(relative),
        Root::Data if plain => loaded.data_dir.join(relative),
        Root::User if relative.is_absolute() && loaded.bundle.manifest.permissions.contains(&craft_plugin_host::Permission::FilesRead) => relative.to_path_buf(),
        _ => return None,
    };
    full.is_file().then(|| full.to_string_lossy().into_owned())
}
