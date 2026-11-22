use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use craft_plugin_host::ui::{AppEvent, ControlsReply, PanelReply, UiEvent, UiInput, Widget};
use craft_plugin_host::viewer::{Overlay, ToolEventKind};
use craft_plugin_host::{Access, AudioRequest, Call, FrameRequest, Host, PickFile, Pixels, PluginError, Retained, Samples};
use effectcraft_engine::Session;
use effectcraft_engine::project::{ItemId, Project};
use serde_json::{Value, json};

use crate::gate::SessionHost;
use crate::manage::{Loaded, plugins, running_plugins};
use crate::viewer::Shown as Drawn;

const WAIT: Duration = Duration::from_millis(4);
const POLL: Duration = Duration::from_micros(500);

struct Job {
    project: Arc<AtomicBool>,
    run: Box<dyn FnOnce(&mut Session) + Send>,
}

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
    let answers = |job: &Job| Arc::ptr_eq(&job.project, &session.project_closed) || job.project.load(Ordering::SeqCst);
    let jobs: Vec<Job> = {
        let mut queued = lock(&FORWARDED.jobs);
        let (mine, others) = std::mem::take(&mut *queued).into_iter().partition(answers);
        *queued = others;
        mine
    };
    let served = jobs.len();
    jobs.into_iter().for_each(|job| (job.run)(session));
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

#[derive(Clone)]
pub(crate) struct Bound {
    loaded: Arc<Loaded>,
    project: Arc<AtomicBool>,
}

impl Bound {
    pub(crate) fn of(loaded: Arc<Loaded>, session: &Session) -> Bound {
        Bound { loaded, project: session.project_closed.clone() }
    }

    fn plugin(&self) -> &str {
        &self.loaded.bundle.manifest.id
    }

    fn superseded(&self) -> bool {
        !plugins().and_then(|plugins| plugins.get(self.plugin())).is_some_and(|now| Arc::ptr_eq(&now, &self.loaded))
    }

    fn stale(&self, session: &Session) -> Option<&'static str> {
        if !Arc::ptr_eq(&self.project, &session.project_closed) {
            Some("its project closed")
        } else if self.superseded() {
            Some("its plugin was reloaded or turned off")
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub(crate) struct Gesture {
    label: String,
    last: Arc<Mutex<Option<Arc<Project>>>>,
}

impl Gesture {
    pub(crate) fn new(label: &str) -> Gesture {
        Gesture { label: label.to_string(), last: Arc::default() }
    }

    fn fold(&self, session: &mut Session, before: &Arc<Project>) {
        if Arc::ptr_eq(&session.project, before) {
            return;
        }
        let mut last = lock(&self.last);
        let joined = last.as_ref().is_some_and(|after| Arc::ptr_eq(after, before));
        let undo = &mut session.history.undo;
        if let Some(first) = undo.iter().rposition(|(_, state)| Arc::ptr_eq(state, before)) {
            undo.truncate(if joined { first } else { first + 1 });
            if let Some(step) = undo.last_mut().filter(|_| !joined) {
                step.0.clone_from(&self.label);
            }
        }
        session.history.merge_key = None;
        *last = Some(session.project.clone());
    }
}

struct Remote {
    bound: Bound,
    wake: Waker,
    gesture: Option<Gesture>,
}

impl Remote {
    fn on_app<T: Send + 'static>(&self, run: impl FnOnce(&mut SessionHost) -> T + Send + 'static) -> Result<T, String> {
        let (reply, answer) = channel();
        let (bound, gesture) = (self.bound.clone(), self.gesture.clone());
        let project = bound.project.clone();
        let run = Box::new(move |session: &mut Session| {
            let answered = match bound.stale(session) {
                Some(why) => {
                    log::info!("plugin `{}`: a request to the app was dropped because {why}", bound.plugin());
                    Err(format!("the app dropped this request because {why}"))
                }
                None => {
                    let before = session.project.clone();
                    let answered = run(&mut SessionHost::new(session, &bound.loaded));
                    if let Some(gesture) = &gesture {
                        gesture.fold(session, &before);
                    }
                    Ok(answered)
                }
            };
            let _ = reply.send(answered);
        });
        lock(&FORWARDED.jobs).push(Job { project, run });
        FORWARDED.ready.notify_all();
        (self.wake)();
        answer.recv().map_err(|_| "the app closed before it answered".to_string())?
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
        let render = self.on_app(move |host| host.frame_job(&request))??;
        Ok(render())
    }

    fn audio(&mut self, request: &AudioRequest) -> Result<Samples, String> {
        let request = request.clone();
        let mix = self.on_app(move |host| host.audio_job(&request))??;
        Ok(mix())
    }

    fn pick_file(&mut self, request: &PickFile) -> Result<Option<PathBuf>, String> {
        let request = request.clone();
        self.on_app(move |host| host.pick_file(&request))?
    }

    fn project_closed(&self) -> Option<Arc<AtomicBool>> {
        Some(self.bound.project.clone())
    }

    fn clipboard(&mut self) -> Result<String, String> {
        self.on_app(|host| host.clipboard())?
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        self.on_app(move |host| host.set_clipboard(&text))?
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Slot(String),
    Effect(String, u64),
    Tool(String, String),
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
    fn set(&self, session: &mut Session, value: String) -> Result<(), String> {
        let params =
            json!({"comp": self.comp, "layer": self.layer, "prop": self.prop, "value": value, "merge": self.merge, "time": self.time, "timeBase": "comp"});
        session.execute("prop.set", params).map(drop).map_err(|e| e.to_string())
    }
}

pub(crate) struct Request {
    pub(crate) key: Key,
    pub(crate) bound: Bound,
    pub(crate) call: Call,
    pub(crate) edit: Option<Target>,
    pub(crate) gesture: Option<Gesture>,
}

enum Reply {
    Panel(PanelReply, Option<String>),
    Overlay(Overlay),
    Value(Option<String>),
}

struct Done {
    request: Request,
    reply: Result<Reply, String>,
}

fn run(request: &Request, wake: &Waker) -> Result<Reply, String> {
    let mut host = Remote { bound: request.bound.clone(), wake: wake.clone(), gesture: request.gesture.clone() };
    let (plugin, call) = (&request.bound.loaded.plugin, &request.call);
    let failed = |error: PluginError| error.to_string();
    match call {
        Call::Panel { .. } => plugin.call(call, None, &mut host).map(|reply| Reply::Panel(reply, None)).map_err(failed),
        Call::Controls { .. } => plugin
            .call(call, None, &mut host)
            .map(|reply: ControlsReply| Reply::Panel(PanelReply { widgets: reply.widgets, ..Default::default() }, reply.value))
            .map_err(failed),
        Call::Drag { .. } => plugin.call(call, None, &mut host).map(Reply::Value).map_err(failed),
        _ => plugin.call(call, None, &mut host).map(Reply::Overlay).map_err(failed),
    }
}

struct Worker {
    loaded: Arc<Loaded>,
    requests: Sender<Request>,
}

fn worker(loaded: &Arc<Loaded>, done: Sender<Done>, wake: Waker) -> std::io::Result<Worker> {
    let (requests, queue) = channel::<Request>();
    let plugin = loaded.bundle.manifest.id.clone();
    std::thread::Builder::new().name(format!("craft-plugin-{plugin}")).spawn(move || {
        for request in queue {
            let reply = match request.bound.superseded() {
                true => Err("its plugin was reloaded or turned off".to_string()),
                false => run(&request, &wake),
            };
            idle(&plugin);
            let _ = done.send(Done { request, reply });
            FORWARDED.ready.notify_all();
            wake();
        }
    })?;
    Ok(Worker { loaded: loaded.clone(), requests })
}

#[derive(Clone, Copy, Debug)]
pub enum Surface<'a> {
    Panel { plugin: &'a str, panel: &'a str },
    Control { effect: &'a str, param: &'a str, instance: u64, value: &'a str, edit: Option<&'a Target> },
}

struct Slot {
    retained: Retained,
    loaded: Arc<Loaded>,
    shown: bool,
    value: Option<String>,
}

impl Slot {
    fn new(loaded: &Arc<Loaded>) -> Slot {
        Slot { retained: Retained::new(), loaded: loaded.clone(), shown: false, value: None }
    }
}

pub(crate) struct Overlaid {
    pub(crate) loaded: Arc<Loaded>,
    pub(crate) request: Vec<u8>,
    pub(crate) shown: Option<Arc<Drawn>>,
    pub(crate) place: [u64; 2],
}

pub(crate) struct Tooling {
    pub(crate) key: Key,
    pub(crate) loaded: Arc<Loaded>,
    pub(crate) overlay: Overlay,
    pub(crate) gesture: Option<Gesture>,
}

pub struct Shown<'a> {
    pub plugin: &'a str,
    pub widgets: &'a [Widget],
    pub error: Option<&'a str>,
}

pub struct Panels {
    slots: HashMap<String, Slot>,
    pub(crate) overlays: HashMap<(String, u64), Overlaid>,
    pub(crate) tool: Option<Tooling>,
    pub(crate) lanes: HashMap<Key, VecDeque<Request>>,
    workers: HashMap<String, Worker>,
    done: (Sender<Done>, Receiver<Done>),
    wake: Option<Waker>,
    seen: Option<Seen>,
    now: Duration,
    deadline: Option<Instant>,
    scheduled: Option<Duration>,
    notice: Option<String>,
}

impl Default for Panels {
    fn default() -> Self {
        Panels {
            slots: HashMap::new(),
            overlays: HashMap::new(),
            tool: None,
            lanes: HashMap::new(),
            workers: HashMap::new(),
            done: channel(),
            wake: None,
            seen: None,
            now: Duration::ZERO,
            deadline: None,
            scheduled: None,
            notice: None,
        }
    }
}

fn find(surface: Surface) -> Result<(Arc<Loaded>, Call), String> {
    let plugins = plugins().ok_or("plugins are not available")?;
    match surface {
        Surface::Panel { plugin, panel } => {
            let loaded = plugins.get(plugin).ok_or_else(|| format!("plugin `{plugin}` is not running"))?;
            Ok((loaded, Call::Panel { panel: panel.to_string(), input: UiInput::default() }))
        }
        Surface::Control { effect, param, instance, value, .. } => {
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

impl Panels {
    pub fn has_waker(&self) -> bool {
        self.wake.is_some()
    }

    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    fn waker(&self) -> Waker {
        self.wake.clone().unwrap_or_else(|| Arc::new(|| {}))
    }

    pub fn frame(&mut self, session: &mut Session, now: Duration) -> Option<Duration> {
        self.now = now;
        serve(session);
        self.collect(session);
        let live = running_plugins();
        let current = |loaded: &Arc<Loaded>| live.iter().any(|running| Arc::ptr_eq(running, loaded));
        self.slots.retain(|_, slot| current(&slot.loaded));
        self.overlays.retain(|_, overlaid| current(&overlaid.loaded));
        self.workers.retain(|_, worker| current(&worker.loaded));
        if self.tool.as_ref().is_some_and(|tool| !current(&tool.loaded)) {
            self.tool = None;
        }
        for slot in self.slots.values_mut() {
            if !std::mem::take(&mut slot.shown) {
                slot.retained.hide();
            }
        }
        let seen = Seen::of(session);
        if let Some(before) = &self.seen {
            for event in before.changes(&seen) {
                self.slots.values_mut().for_each(|slot| slot.retained.notify(&event));
            }
        }
        self.seen = Some(seen);
        self.deadline = None;
        self.scheduled = self.slots.values().filter_map(|slot| slot.retained.next_due()).min();
        self.scheduled.map(|due| due.saturating_sub(now))
    }

    pub fn submit(&mut self, key: &str, events: Vec<UiEvent>, keys: &str) {
        if let Some(slot) = self.slots.get_mut(key) {
            slot.retained.push(events, keys);
        }
    }

    pub fn busy(&self, key: &str) -> bool {
        self.slots.get(key).is_some_and(|slot| slot.retained.in_flight())
    }

    pub fn calls(&self, key: &str) -> u64 {
        self.slots.get(key).map_or(0, |slot| slot.retained.calls())
    }

    pub fn idle(&self) -> bool {
        self.lanes.is_empty() && !self.slots.values().any(|slot| slot.retained.in_flight())
    }

    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub fn show(&mut self, session: &mut Session, key: &str, surface: Surface, size: [f32; 2]) -> Result<Shown<'_>, String> {
        let (loaded, call) = find(surface)?;
        let slot = self.slots.entry(key.to_string()).or_insert_with(|| Slot::new(&loaded));
        if !Arc::ptr_eq(&slot.loaded, &loaded) {
            *slot = Slot::new(&loaded);
        }
        slot.shown = true;
        let mut edit = None;
        if let Surface::Control { value, edit: target, .. } = surface {
            edit = target.cloned();
            if slot.value.as_deref() != Some(value) {
                slot.value = Some(value.to_string());
                slot.retained.invalidate();
            }
        }
        let bound = Bound::of(loaded, session);
        while let Some(input) = self.slots.get_mut(key).and_then(|slot| slot.retained.start(size, self.now)) {
            let request =
                Request { key: Key::Slot(key.to_string()), bound: bound.clone(), call: with_input(call.clone(), input), edit: edit.clone(), gesture: None };
            self.send(request);
            self.wait(session, |panels| !panels.busy(key));
            if self.busy(key) {
                break;
            }
        }
        self.schedule(key);
        let slot = self.slots.get(key).ok_or("the panel closed")?;
        Ok(Shown { plugin: &slot.loaded.bundle.manifest.id, widgets: slot.retained.widgets(), error: slot.retained.error() })
    }

    fn schedule(&mut self, key: &str) {
        let due = self.slots.get(key).and_then(|slot| slot.retained.next_due());
        if let Some(due) = due
            && self.scheduled.is_none_or(|scheduled| due < scheduled)
        {
            self.scheduled = Some(due);
            self.waker()();
        }
    }

    fn send(&mut self, request: Request) {
        let plugin = request.bound.plugin().to_string();
        if !self.workers.get(&plugin).is_some_and(|worker| Arc::ptr_eq(&worker.loaded, &request.bound.loaded)) {
            match worker(&request.bound.loaded, self.done.0.clone(), self.waker()) {
                Ok(started) => {
                    self.workers.insert(plugin.clone(), started);
                }
                Err(error) => {
                    let _ = self.done.0.send(Done { request, reply: Err(error.to_string()) });
                    return;
                }
            }
        }
        lock(&FORWARDED.busy).push(plugin.clone());
        if let Some(worker) = self.workers.get(&plugin)
            && let Err(unsent) = worker.requests.send(request)
        {
            idle(&plugin);
            let _ = self.done.0.send(Done { request: unsent.0, reply: Err("the plugin's worker stopped".into()) });
        }
    }

    pub(crate) fn post(&mut self, request: Request) {
        let Some(waiting) = self.lanes.get_mut(&request.key) else {
            self.lanes.insert(request.key.clone(), VecDeque::new());
            return self.send(request);
        };
        let moving = |call: &Call| matches!(call, Call::Tool { event, .. } if event.kind == ToolEventKind::Move);
        let kind = std::mem::discriminant(&request.call);
        let replaced = match request.call {
            Call::Overlay { .. } | Call::Drag { .. } => waiting.iter().position(|other| std::mem::discriminant(&other.call) == kind),
            _ if moving(&request.call) && waiting.back().is_some_and(|last| moving(&last.call)) => Some(waiting.len() - 1),
            _ => None,
        };
        match replaced {
            Some(at) => waiting[at] = request,
            None => waiting.push_back(request),
        }
    }

    fn advance(&mut self, key: &Key) {
        match self.lanes.get_mut(key).and_then(VecDeque::pop_front) {
            Some(next) => self.send(next),
            None => {
                self.lanes.remove(key);
            }
        }
    }

    fn collect(&mut self, session: &mut Session) -> bool {
        let mut any = false;
        while let Ok(done) = self.done.1.try_recv() {
            any = true;
            self.apply(session, done);
        }
        any
    }

    fn apply(&mut self, session: &mut Session, Done { request, reply }: Done) {
        let stale = request.bound.stale(session);
        if let Some(why) = stale {
            log::info!("plugin `{}`: a reply for {:?} was dropped because {why}", request.bound.plugin(), request.key);
        }
        let Request { key, bound, call, edit, .. } = request;
        if let Key::Slot(slot_key) = &key {
            let Some(slot) = self.slots.get_mut(slot_key).filter(|slot| Arc::ptr_eq(&slot.loaded, &bound.loaded)) else { return };
            match (stale, reply) {
                (None, Ok(Reply::Panel(reply, edited))) => {
                    slot.retained.finish(Ok(reply), self.now);
                    if let (Some(value), Some(edit)) = (edited, &edit)
                        && let Err(error) = edit.set(session, value)
                    {
                        self.notice = Some(error);
                    }
                }
                (None, Err(error)) => slot.retained.finish(Err(error), self.now),
                _ => slot.retained.abandon(),
            }
            if self.slots.get(slot_key).is_some_and(|slot| slot.retained.pending()) {
                self.waker()();
            }
            return;
        }
        self.advance(&key);
        if stale.is_some() {
            return;
        }
        match (call, reply) {
            (Call::Overlay { effect, cx }, reply) => {
                let overlay = match reply {
                    Ok(Reply::Overlay(overlay)) => overlay,
                    Ok(_) => Overlay::default(),
                    Err(error) => {
                        log::debug!("plugin `{}` effect `{effect}` draws no overlay: {error}", bound.plugin());
                        Overlay::default()
                    }
                };
                let Key::Effect(plugin, instance) = key else { return };
                if let Some(slot) = self.overlays.get_mut(&(plugin.clone(), instance)).filter(|slot| Arc::ptr_eq(&slot.loaded, &bound.loaded)) {
                    slot.shown = Some(Arc::new(Drawn { overlay, plugin, cx, loaded: bound.loaded, effect, place: slot.place }));
                }
            }
            (Call::Tool { .. }, Ok(Reply::Overlay(overlay))) => {
                if let Some(tool) = self.tool.as_mut().filter(|tool| tool.key == key && Arc::ptr_eq(&tool.loaded, &bound.loaded)) {
                    tool.overlay = overlay;
                }
            }
            (Call::Drag { .. }, Ok(Reply::Value(Some(value)))) => {
                if let Some(edit) = edit
                    && let Err(error) = edit.set(session, value)
                {
                    self.notice = Some(error);
                }
            }
            (_, Err(error)) => self.notice = Some(error),
            _ => {}
        }
    }

    pub(crate) fn wait(&mut self, session: &mut Session, done: impl Fn(&Panels) -> bool) {
        while !done(self) {
            let deadline = *self.deadline.get_or_insert_with(|| Instant::now() + WAIT);
            serve(session);
            if self.collect(session) {
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
