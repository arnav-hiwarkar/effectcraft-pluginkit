use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use craft_plugin_host::ui::{AppEvent, ControlsReply, PanelReply, UiEvent, UiInput, Widget};
use craft_plugin_host::viewer::{Overlay, ToolEventKind};
use craft_plugin_host::{Access, AudioRequest, Call, FrameRequest, Host, MAX_BACKLOG, PickFile, Pixels, Retained, Samples};
use effectcraft_engine::Session;
use effectcraft_engine::project::{ItemId, Project};
use serde_json::{Value, json};

use crate::gate::SessionHost;
use crate::manage::{Loaded, plugins, running, running_plugins};
use crate::viewer::Shown as Drawn;

const WAIT: Duration = Duration::from_millis(4);
const POLL: Duration = Duration::from_micros(500);
const RECHECK: Duration = Duration::from_millis(10);

struct Job {
    project: Arc<AtomicBool>,
    run: Box<dyn FnOnce(&mut Session) + Send>,
}

type Waker = Arc<dyn Fn() + Send + Sync>;

struct Forwarded {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

static FORWARDED: Forwarded = Forwarded { jobs: Mutex::new(Vec::new()), ready: Condvar::new() };

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn serve_one(session: &mut Session) -> bool {
    let job = {
        let mut jobs = lock(&FORWARDED.jobs);
        let mine = jobs.iter().position(|job| Arc::ptr_eq(&job.project, &session.project_closed) || job.project.load(Ordering::SeqCst));
        mine.map(|at| jobs.remove(at))
    };
    job.map(|job| (job.run)(session)).is_some()
}

fn wait_for_work(timeout: Duration) {
    let jobs = lock(&FORWARDED.jobs);
    if jobs.is_empty() {
        let _ = FORWARDED.ready.wait_timeout(jobs, timeout);
    }
}

pub(crate) fn run_command(session: &mut Session, loaded: Arc<Loaded>, command: &str) -> Result<(), String> {
    let asking = Arc::new(());
    let plugin = loaded.bundle.manifest.id.clone();
    let bound = Bound::of(loaded, session, Arc::downgrade(&asking));
    let request = Request { key: Key::Command(plugin.clone()), bound, call: Call::Command { command: command.to_string() }, edit: None, gesture: None };
    let running = std::thread::Builder::new()
        .name(format!("craft-plugin-command-{plugin}"))
        .spawn(move || run(&request, &(Arc::new(|| {}) as Waker)).map(drop))
        .map_err(|error| error.to_string())?;
    while !running.is_finished() {
        if !serve_one(session) {
            wait_for_work(POLL);
        }
    }
    running.join().map_err(|_| "the plugin's command panicked".to_string())?
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
    pub(crate) loaded: Arc<Loaded>,
    project: Arc<AtomicBool>,
    surface: Weak<()>,
}

impl Bound {
    pub(crate) fn of(loaded: Arc<Loaded>, session: &Session, surface: Weak<()>) -> Bound {
        Bound { loaded, project: session.project_closed.clone(), surface }
    }

    pub(crate) fn asked_by(&self, surface: &Arc<()>) -> bool {
        std::ptr::eq(self.surface.as_ptr(), Arc::as_ptr(surface))
    }

    pub(crate) fn same(&self, other: &Bound) -> bool {
        Weak::ptr_eq(&self.surface, &other.surface)
    }

    pub(crate) fn from(&self, loaded: &Arc<Loaded>, session: &Session) -> bool {
        Arc::ptr_eq(&self.loaded, loaded) && Arc::ptr_eq(&self.project, &session.project_closed)
    }

    fn plugin(&self) -> &str {
        &self.loaded.bundle.manifest.id
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        let reloaded = || !plugins().and_then(|plugins| plugins.get(self.plugin())).is_some_and(|now| Arc::ptr_eq(&now, &self.loaded));
        let why = match () {
            _ if self.project.load(Ordering::SeqCst) => "its project closed",
            _ if self.surface.strong_count() == 0 => "the panel, overlay, tool or menu that asked for it closed",
            _ if reloaded() => "its plugin was reloaded or turned off",
            _ => return Ok(()),
        };
        Err(format!("the app dropped this request because {why}"))
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
        self.bound.check()?;
        let (reply, answer) = channel();
        let (bound, gesture) = (self.bound.clone(), self.gesture.clone());
        let project = bound.project.clone();
        let run = Box::new(move |session: &mut Session| {
            let answered = bound.check().map(|()| {
                let before = session.project.clone();
                let answered = run(&mut SessionHost::new(session, &bound.loaded));
                if let Some(gesture) = &gesture {
                    gesture.fold(session, &before);
                }
                answered
            });
            if let Err(error) = &answered {
                log::info!("plugin `{}`: {error}", bound.plugin());
            }
            let _ = reply.send(answered);
        });
        lock(&FORWARDED.jobs).push(Job { project, run });
        FORWARDED.ready.notify_all();
        (self.wake)();
        let answered = loop {
            match answer.recv_timeout(RECHECK) {
                Ok(answered) => break answered,
                Err(RecvTimeoutError::Timeout) => self.bound.check()?,
                Err(RecvTimeoutError::Disconnected) => return Err("the app closed before it answered".into()),
            }
        };
        self.bound.check()?;
        answered
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
        let pixels = render();
        self.bound.check().map(|()| pixels)
    }

    fn audio(&mut self, request: &AudioRequest) -> Result<Samples, String> {
        let request = request.clone();
        let mix = self.on_app(move |host| host.audio_job(&request))??;
        let samples = mix();
        self.bound.check().map(|()| samples)
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
    Command(String),
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
    match call {
        Call::Panel { .. } => plugin.call(call, None, &mut host).map(|reply| Reply::Panel(reply, None)),
        Call::Controls { .. } => plugin
            .call(call, None, &mut host)
            .map(|reply: ControlsReply| Reply::Panel(PanelReply { widgets: reply.widgets, ..Default::default() }, reply.value)),
        Call::Drag { .. } => plugin.call(call, None, &mut host).map(Reply::Value),
        Call::Command { .. } => plugin.call(call, None, &mut host).map(|()| Reply::Value(None)),
        _ => plugin.call(call, None, &mut host).map(Reply::Overlay),
    }
    .map_err(|error| error.to_string())
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
            let reply = request.bound.check().and_then(|()| run(&request, &wake));
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
    open: Arc<()>,
    shown: bool,
    value: Option<String>,
}

impl Slot {
    fn new(loaded: &Arc<Loaded>) -> Slot {
        Slot { retained: Retained::new(), loaded: loaded.clone(), open: Arc::default(), shown: false, value: None }
    }
}

pub(crate) struct Overlaid {
    pub(crate) bound: Bound,
    pub(crate) open: Arc<()>,
    pub(crate) request: Vec<u8>,
    pub(crate) shown: Option<Arc<Drawn>>,
    pub(crate) place: [u64; 2],
}

pub(crate) struct Tooling {
    pub(crate) key: Key,
    pub(crate) loaded: Arc<Loaded>,
    pub(crate) open: Arc<()>,
    pub(crate) overlay: Overlay,
    pub(crate) gesture: Option<Gesture>,
}

pub struct Shown<'a> {
    pub plugin: &'a str,
    pub widgets: &'a [Widget],
    pub error: Option<&'a str>,
}

struct Inbox(Sender<Done>, Receiver<Done>);

impl Default for Inbox {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Inbox(sender, receiver)
    }
}

#[derive(Default)]
pub struct Panels {
    slots: HashMap<String, Slot>,
    pub(crate) overlays: HashMap<(String, u64), Overlaid>,
    pub(crate) tool: Option<Tooling>,
    pub(crate) lanes: HashMap<Key, VecDeque<Request>>,
    workers: HashMap<String, Worker>,
    done: Inbox,
    wake: Option<Waker>,
    seen: Option<Seen>,
    now: Duration,
    deadline: Option<Instant>,
    scheduled: Option<Duration>,
    notice: Option<String>,
    open: Arc<()>,
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
        self.deadline = Some(Instant::now() + WAIT);
        self.work(session);
        let live = running_plugins();
        let current = |loaded: &Arc<Loaded>| live.iter().any(|running| Arc::ptr_eq(running, loaded));
        self.slots.retain(|_, slot| current(&slot.loaded));
        self.overlays.retain(|_, overlaid| current(&overlaid.bound.loaded));
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

    pub fn command(&mut self, session: &Session, plugin: &str, command: &str) -> Result<(), String> {
        let bound = Bound::of(running(plugin)?, session, Arc::downgrade(&self.open));
        self.post(Request { key: Key::Command(plugin.to_string()), bound, call: Call::Command { command: command.to_string() }, edit: None, gesture: None });
        Ok(())
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
        let bound = Bound::of(loaded, session, Arc::downgrade(&slot.open));
        while let Some(input) = self.slots.get_mut(key).and_then(|slot| slot.retained.start(size, self.now)) {
            let mut call = call.clone();
            if let Call::Panel { input: given, .. } | Call::Controls { input: given, .. } = &mut call {
                *given = input;
            }
            self.send(Request { key: Key::Slot(key.to_string()), bound: bound.clone(), call, edit: edit.clone(), gesture: None });
            self.wait(session, |panels| !panels.busy(key));
            if self.busy(key) || edit.is_some() || self.spent() {
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
        let (done, wake) = (self.done.0.clone(), self.waker());
        let fail = |request, error: String| {
            let _ = done.send(Done { request, reply: Err(error) });
        };
        if let Err(error) = request.bound.check() {
            return fail(request, error);
        }
        let plugin = request.bound.plugin().to_string();
        let worker = match self.workers.entry(plugin.clone()) {
            Entry::Occupied(found) if Arc::ptr_eq(&found.get().loaded, &request.bound.loaded) => found.into_mut(),
            entry => match worker(&request.bound.loaded, done.clone(), wake) {
                Ok(started) => entry.insert_entry(started).into_mut(),
                Err(error) => return fail(request, error.to_string()),
            },
        };
        if let Err(unsent) = worker.requests.send(request) {
            self.workers.remove(&plugin);
            fail(unsent.0, "the plugin's worker stopped".into());
        }
    }

    pub(crate) fn post(&mut self, request: Request) {
        let Some(waiting) = self.lanes.get_mut(&request.key) else {
            self.lanes.insert(request.key.clone(), VecDeque::new());
            return self.send(request);
        };
        let merges = |queued: &Request| match (&queued.call, &request.call) {
            (Call::Overlay { .. }, Call::Overlay { .. }) => true,
            (Call::Drag { key: handle, .. }, Call::Drag { key, .. }) => handle == key && queued.edit == request.edit,
            (Call::Tool { event: last, .. }, Call::Tool { event, .. }) => last.kind == ToolEventKind::Move && event.kind == ToolEventKind::Move,
            _ => false,
        };
        let kind = std::mem::discriminant(&request.call);
        let latest = waiting.iter().rposition(|queued| std::mem::discriminant(&queued.call) == kind);
        match latest.filter(|at| merges(&waiting[*at])) {
            Some(at) => waiting[at] = request,
            None => waiting.push_back(request),
        }
        let motion = |queued: &Request| match &queued.call {
            Call::Tool { event, .. } => event.kind == ToolEventKind::Move,
            Call::Drag { .. } => true,
            _ => false,
        };
        while waiting.len() > MAX_BACKLOG
            && let Some(oldest) = waiting.iter().position(motion)
        {
            waiting.remove(oldest);
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

    fn spent(&mut self) -> bool {
        Instant::now() >= *self.deadline.get_or_insert_with(|| Instant::now() + WAIT)
    }

    fn work(&mut self, session: &mut Session) -> bool {
        let mut any = false;
        while serve_one(session) || self.done.1.try_recv().map(|done| self.apply(session, done)).is_ok() {
            any = true;
            if self.spent() {
                self.waker()();
                break;
            }
        }
        any
    }

    fn apply(&mut self, session: &mut Session, Done { request, reply }: Done) {
        let stale = request.bound.check().err();
        if let Some(why) = &stale {
            log::info!("plugin `{}`: a reply for {:?} was dropped: {why}", request.bound.plugin(), request.key);
        }
        let Request { key, bound, call, edit, .. } = request;
        if let Key::Slot(slot_key) = &key {
            let Some(slot) = self.slots.get_mut(slot_key).filter(|slot| bound.asked_by(&slot.open)) else { return };
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
                if let Some(slot) = self.overlays.get_mut(&(plugin.clone(), instance)).filter(|slot| bound.asked_by(&slot.open)) {
                    slot.shown = Some(Arc::new(Drawn { overlay, plugin, cx, bound, effect, place: slot.place }));
                }
            }
            (Call::Tool { .. }, Ok(Reply::Overlay(overlay))) => {
                if let Some(tool) = self.tool.as_mut().filter(|tool| bound.asked_by(&tool.open)) {
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
        while !done(self) && !self.spent() {
            if !self.work(session) {
                wait_for_work(POLL);
            }
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use craft_plugin_host::viewer::ToolEvent;

    use super::*;

    fn legacy(session: &mut Session, id: &str) -> (tempfile::TempDir, Arc<Loaded>) {
        static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
        let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("temp dir"));
        let plugins = crate::manage::install(session, Some(config.path()));
        let bundle = tempfile::tempdir().expect("temp dir");
        std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/legacy/legacy.wasm"), bundle.path().join("legacy.wasm")).unwrap();
        let manifest = json!({"id": id, "name": "Legacy", "version": "1.0.0", "entry": {"wasm": "legacy.wasm"}, "permissions": []});
        std::fs::write(bundle.path().join("plugin.json"), manifest.to_string()).unwrap();
        plugins.load_for_session(bundle.path()).unwrap();
        let loaded = plugins.get(id).unwrap();
        (bundle, loaded)
    }

    fn request(panels: &Panels, session: &Session, key: Key, loaded: &Arc<Loaded>, call: Call) -> Request {
        Request { key, bound: Bound::of(loaded.clone(), session, Arc::downgrade(&panels.open)), call, edit: None, gesture: None }
    }

    fn idle(panels: &mut Panels, session: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !panels.idle() {
            assert!(Instant::now() < deadline, "the plugin did not answer within 20 s");
            panels.frame(session, Duration::ZERO);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_request_queued_for_a_plugin_that_was_reloaded_is_dropped_and_the_new_loads_worker_stays() {
        const RELOADED: &str = "test.queue-reloaded";
        let mut session = effectcraft_host::session();
        let (bundle, first) = legacy(&mut session, RELOADED);
        let mut panels = Panels::default();
        let command = || Call::Command { command: "none".into() };
        let old = Key::Effect(RELOADED.into(), 1);
        for _ in 0..2 {
            let queued = request(&panels, &session, old.clone(), &first, command());
            panels.post(queued);
        }
        plugins().unwrap().load_for_session(bundle.path()).unwrap();
        let second = plugins().unwrap().get(RELOADED).unwrap();
        let new = request(&panels, &session, Key::Effect(RELOADED.into(), 2), &second, command());
        panels.post(new);
        idle(&mut panels, &mut session);
        assert!(Arc::ptr_eq(&panels.workers[RELOADED].loaded, &second));
    }

    fn tool(kind: ToolEventKind, x: f64) -> Call {
        let event = ToolEvent {
            kind,
            comp: [x, 0.0],
            layer: None,
            layer_id: None,
            modifiers: Default::default(),
            pressure: 0.0,
            scroll: [0.0; 2],
            key: String::new(),
            zoom: 1.0,
            time: 0.0,
        };
        Call::Tool { tool: "t".into(), event }
    }

    #[test]
    fn a_frame_answers_queued_app_requests_only_until_its_budget_is_spent() {
        let mut session = effectcraft_host::session();
        let served = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let counted = served.clone();
            let run = Box::new(move |_: &mut Session| {
                std::thread::sleep(Duration::from_millis(20));
                counted.fetch_add(1, Ordering::SeqCst);
            });
            lock(&FORWARDED.jobs).push(Job { project: session.project_closed.clone(), run });
        }
        let mut panels = Panels::default();
        let wakes = Arc::new(AtomicUsize::new(0));
        let woken = wakes.clone();
        panels.set_waker(move || {
            woken.fetch_add(1, Ordering::SeqCst);
        });
        panels.frame(&mut session, Duration::ZERO);
        let first = (served.load(Ordering::SeqCst), wakes.load(Ordering::SeqCst));
        panels.frame(&mut session, Duration::ZERO);
        assert_eq!((first, served.load(Ordering::SeqCst)), ((1, 1), 2));
    }

    #[test]
    fn a_tool_lane_past_its_bound_drops_its_oldest_moves_and_keeps_every_key() {
        let mut session = effectcraft_host::session();
        let (_bundle, loaded) = legacy(&mut session, "test.queue-lane");
        let mut panels = Panels::default();
        let lane = Key::Tool("test.queue-lane".into(), "t".into());
        for (kind, x) in
            [(ToolEventKind::Down, 0.0)].into_iter().chain((0..50).flat_map(|x| [(ToolEventKind::Move, f64::from(x)), (ToolEventKind::Key, f64::from(x))]))
        {
            let posted = request(&panels, &session, lane.clone(), &loaded, tool(kind, x));
            panels.post(posted);
        }
        let queued: Vec<(ToolEventKind, f64)> = panels.lanes[&lane]
            .iter()
            .filter_map(|request| match &request.call {
                Call::Tool { event, .. } => Some((event.kind, event.comp[0])),
                _ => None,
            })
            .collect();
        let kept: Vec<(ToolEventKind, f64)> =
            (0..50).flat_map(|x| [(x >= 36).then_some((ToolEventKind::Move, f64::from(x))), Some((ToolEventKind::Key, f64::from(x)))]).flatten().collect();
        assert_eq!(queued, kept);
    }
}
