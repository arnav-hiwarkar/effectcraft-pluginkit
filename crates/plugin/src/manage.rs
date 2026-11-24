use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use craft_plugin_host::PluginImage;
use craft_plugin_host::{Bundle, BundleError, EffectRegistry, Limits, Permission, Plugin, Refusal, Registry, RunMode, TaskState};
use effectcraft_engine::{EngineError, PluginItem, Session};
use serde::Serialize;
use serde_json::{Value, json};

use crate::data_only;
use crate::effects;
use crate::gate::SessionHost;

pub struct Loaded {
    pub bundle: Bundle,
    pub plugin: Plugin,
    pub data_dir: PathBuf,
    pub revision: u64,
    render_errors: Mutex<BTreeMap<String, String>>,
}

impl Loaded {
    pub(crate) fn report(&self, effect: &str, error: Option<String>) -> bool {
        let Ok(mut errors) = self.render_errors.lock() else { return false };
        match error {
            Some(error) => {
                let new = errors.get(effect) != Some(&error);
                if new {
                    log::warn!("plugin `{}` effect `{effect}`: {error}", self.bundle.manifest.id);
                }
                errors.insert(effect.to_string(), error);
                new
            }
            None => errors.remove(effect).is_some(),
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Running,
    NeedsApproval,
    Disabled,
    Quarantined,
    Failed,
}

#[derive(Serialize, Clone, Debug)]
pub struct PermissionInfo {
    pub name: &'static str,
    pub summary: &'static str,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PluginTask {
    pub plugin: String,
    #[serde(flatten)]
    pub state: TaskState,
}

#[derive(Serialize, Clone, Debug)]
pub struct Contribution {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub sandboxed: bool,
    pub can_unsandbox: bool,
    pub permissions: Vec<PermissionInfo>,
    pub state: PluginState,
    pub error: Option<String>,
    pub session_only: bool,
    pub effects: Vec<Contribution>,
    pub panels: Vec<Contribution>,
    pub commands: Vec<Contribution>,
    pub path: PathBuf,
}

struct Inner {
    registry: Registry,
    session_only: Vec<Bundle>,
    loaded: BTreeMap<String, Arc<Loaded>>,
    data: BTreeSet<String>,
    failed: BTreeMap<String, String>,
    refused: BTreeMap<String, Refusal>,
    safe_mode: bool,
}

pub struct Plugins {
    inner: Mutex<Inner>,
}

fn load(bundle: &Bundle, digest: &str, grants: BTreeSet<Permission>, data_dir: PathBuf) -> Result<Arc<Loaded>, String> {
    let plugin = Plugin::open(bundle, digest, grants, data_dir.clone(), Limits::default()).map_err(|e| e.to_string())?;
    let mut hasher = DefaultHasher::new();
    bundle.manifest.version.hash(&mut hasher);
    digest.hash(&mut hasher);
    craft_plugin_host::validate(&bundle.manifest, plugin.contributions())?;
    effects::Engine.admit(plugin.contributions())?;
    let loaded = Loaded { bundle: bundle.clone(), plugin, data_dir, revision: hasher.finish(), render_errors: Mutex::new(BTreeMap::new()) };
    Ok(Arc::new(loaded))
}

impl Inner {
    fn start(&mut self, bundle: &Bundle) {
        let id = bundle.manifest.id.clone();
        self.stop(&id);
        let (bundle, grants, digest) = if self.is_session_only(&id) {
            match Bundle::read(&bundle.dir) {
                Ok((read, digest)) if read.manifest.id == id => {
                    self.session_only.iter_mut().filter(|kept| kept.manifest.id == id).for_each(|kept| *kept = read.clone());
                    (read.clone().with_mode(RunMode::Sandboxed), read.manifest.permissions.clone(), digest)
                }
                Ok(_) => {
                    self.failed.insert(id, "its manifest now names another plug-in; load it for this session again".into());
                    return;
                }
                Err(error) => {
                    self.failed.insert(id, error.to_string());
                    return;
                }
            }
        } else {
            match self.registry.approved(bundle) {
                Ok(approved) => (approved.bundle, approved.grants, approved.digest),
                Err(refusal) => {
                    self.refused.insert(id, refusal);
                    return;
                }
            }
        };
        if bundle.manifest.is_data_only() {
            match data_only::check(&bundle) {
                Ok(checked) => {
                    data_only::register(&id, checked);
                    self.data.insert(id);
                }
                Err(error) => {
                    log::warn!("plugin `{id}` did not load: {error}");
                    self.failed.insert(id, error);
                }
            }
            return;
        }
        let unsandboxed = bundle.mode == RunMode::Unsandboxed;
        if unsandboxed {
            self.mark(Some(&id));
        }
        let loaded = load(&bundle, &digest, grants, self.registry.data_dir(&id));
        let replaced = loaded.as_ref().ok().filter(|_| unsandboxed).and_then(|_| self.registry.grants(&bundle).err());
        if let Some(refusal) = replaced {
            log::warn!("plugin `{id}` changed while it loaded, so it was not started: {refusal}");
            self.refused.insert(id, refusal);
            self.mark(None);
            return;
        }
        match loaded {
            Ok(loaded) => {
                effects::Engine.register(&loaded);
                self.loaded.insert(id, loaded);
            }
            Err(error) => {
                log::warn!("plugin `{id}` did not load: {error}");
                self.failed.insert(id, error);
                if unsandboxed {
                    self.mark(None);
                }
            }
        }
    }

    fn stop(&mut self, id: &str) {
        self.failed.remove(id);
        self.refused.remove(id);
        if self.data.remove(id) {
            data_only::unregister(id);
        }
        if let Some(loaded) = self.loaded.remove(id) {
            loaded.plugin.cancel_all();
            effects::Engine.unregister(id);
            if loaded.bundle.mode == RunMode::Unsandboxed {
                self.mark(None);
            }
        }
    }

    fn mark(&mut self, starting: Option<&str>) {
        let live = self.loaded.iter().filter(|(_, loaded)| loaded.bundle.mode == RunMode::Unsandboxed).map(|(id, _)| id.as_str());
        let unsandboxed: BTreeSet<String> = live.chain(starting).map(str::to_string).collect();
        if let Err(error) = self.registry.mark_running(&unsandboxed) {
            log::warn!("the plug-in run marker was not written: {error}");
        }
    }

    fn is_session_only(&self, id: &str) -> bool {
        self.session_only.iter().any(|bundle| bundle.manifest.id == id)
    }

    fn forget_session_copy(&mut self, id: &str) -> bool {
        let before = self.session_only.len();
        self.session_only.retain(|bundle| bundle.manifest.id != id);
        self.session_only.len() != before
    }

    fn installed(&self) -> Vec<Bundle> {
        self.registry.bundles().into_iter().flatten().collect()
    }

    fn reload(&mut self) {
        let running: Vec<String> = self.loaded.keys().chain(&self.data).cloned().collect();
        running.iter().for_each(|id| self.stop(id));
        self.failed.clear();
        self.refused.clear();
        let mut bundles = self.installed();
        bundles.extend(self.session_only.iter().cloned());
        bundles.sort_by_key(|bundle| bundle.manifest.is_data_only());
        for bundle in bundles {
            self.start(&bundle);
        }
    }

    fn find(&self, id: &str) -> Result<Bundle, String> {
        self.installed()
            .into_iter()
            .chain(self.session_only.iter().cloned())
            .find(|bundle| bundle.manifest.id == id)
            .ok_or_else(|| format!("plugin `{id}` is not installed"))
    }

    fn info(&self, bundle: &Bundle, session_only: bool) -> PluginInfo {
        let manifest = &bundle.manifest;
        let loaded = self.loaded.get(&manifest.id);
        let failed = self.failed.get(&manifest.id).cloned().or_else(|| loaded.and_then(|loaded| loaded.plugin.failure()));
        let running = (loaded.is_some() || self.data.contains(&manifest.id)) && failed.is_none();
        let (state, error) = match (running, failed, self.refused.get(&manifest.id)) {
            (true, ..) => (PluginState::Running, None),
            (false, Some(error), _) => (PluginState::Failed, Some(error)),
            (false, None, Some(Refusal::Disabled)) => (PluginState::Disabled, None),
            (false, None, Some(refusal @ Refusal::Quarantined(_))) => (PluginState::Quarantined, Some(refusal.to_string())),
            (false, None, Some(Refusal::NotApproved)) => (PluginState::NeedsApproval, None),
            (false, None, Some(refusal)) => (PluginState::NeedsApproval, Some(refusal.to_string())),
            (false, None, None) if self.safe_mode => (PluginState::Disabled, Some(SAFE_MODE_NOTE.to_string())),
            (false, None, None) => (PluginState::NeedsApproval, None),
        };
        let found = loaded.map(|l| l.plugin.contributions().clone()).unwrap_or_default();
        let named = |id: &str, title: &str| Contribution { id: id.to_string(), title: title.to_string() };
        PluginInfo {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            author: manifest.author.clone(),
            description: manifest.description.clone(),
            sandboxed: if session_only { true } else { self.registry.mode(manifest) == RunMode::Sandboxed },
            can_unsandbox: !session_only && manifest.can_choose_unsandboxed(),
            permissions: manifest.permissions.iter().map(|p| PermissionInfo { name: p.name(), summary: p.summary() }).collect(),
            state,
            error,
            session_only,
            effects: found.effects.iter().map(|e| named(&effects::effect_id(&manifest.id, &e.id), &e.name)).collect(),
            panels: found.panels.iter().map(|p| named(&p.id, &p.title)).collect(),
            commands: found.commands.iter().map(|c| named(&c.id, &c.title)).collect(),
            path: bundle.dir.clone(),
        }
    }
}

fn approve_snapshot(registry: &mut Registry, snapshot: &Bundle, digest: &str) -> Result<(), String> {
    registry.approve(snapshot).map_err(|e| e.to_string())?;
    let recorded = registry.consent(&snapshot.manifest.id).and_then(|consent| consent.approved.as_ref()).map(|approved| approved.digest.as_str());
    if recorded == Some(digest) {
        return Ok(());
    }
    registry.disable(&snapshot.manifest.id).map_err(|e| e.to_string())?;
    Err(format!("plugin `{}` changed while it was installed, so it was not approved", snapshot.manifest.id))
}

fn local_effect<'a>(plugin: &str, loaded: &'a Loaded, effect_id: &str) -> Option<&'a str> {
    loaded.plugin.contributions().effects.iter().map(|e| e.id.as_str()).find(|local| effects::effect_id(plugin, local) == effect_id)
}

const SAFE_MODE_NOTE: &str = "EffectCraft started in safe mode, so no plug-ins were started";

impl Plugins {
    pub fn open(root: &Path, safe_mode: bool) -> Plugins {
        let mut registry = Registry::open(root);
        match registry.recover() {
            Ok(quarantined) => quarantined.iter().for_each(|id| log::warn!("plug-in `{id}` was turned off: EffectCraft closed unexpectedly while it ran")),
            Err(error) => log::warn!("the last run's plug-in marker was not read: {error}"),
        }
        let (loaded, failed, refused) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let mut inner = Inner { registry, session_only: Vec::new(), loaded, data: BTreeSet::new(), failed, refused, safe_mode };
        if !safe_mode {
            inner.reload();
        }
        Plugins { inner: Mutex::new(inner) }
    }

    pub fn end_run(&self) {
        if let Err(error) = self.lock().registry.end_run() {
            log::warn!("the plug-in run marker was not removed: {error}");
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        let inner = self.lock();
        let for_session = |id: &str| inner.session_only.iter().any(|bundle| bundle.manifest.id == id);
        let installed = inner.installed().into_iter().filter(|bundle| !for_session(&bundle.manifest.id));
        let installed = installed.map(|bundle| inner.info(&bundle, false));
        let session_only = inner.session_only.iter().map(|bundle| inner.info(bundle, true));
        installed.chain(session_only).collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Loaded>> {
        self.lock().loaded.get(id).cloned()
    }

    fn running_plugins(&self) -> Vec<Arc<Loaded>> {
        self.lock().loaded.values().cloned().collect()
    }

    pub fn tasks(&self) -> Vec<PluginTask> {
        let running = self.running_plugins();
        let of = |loaded: &Arc<Loaded>| {
            let plugin = loaded.bundle.manifest.id.clone();
            loaded.plugin.tasks().into_iter().map(move |state| PluginTask { plugin: plugin.clone(), state })
        };
        running.iter().flat_map(of).collect()
    }

    pub fn render_error(&self, effect_id: &str) -> Option<String> {
        let inner = self.lock();
        inner.loaded.iter().find_map(|(plugin, loaded)| {
            let local = local_effect(plugin, loaded, effect_id)?;
            loaded.render_errors.lock().ok()?.get(local).cloned()
        })
    }

    pub fn designer(&self, effect_id: &str) -> Option<PluginItem> {
        let inner = self.lock();
        inner.loaded.iter().find_map(|(plugin, loaded)| {
            let local = local_effect(plugin, loaded, effect_id)?;
            let found = loaded.plugin.contributions();
            let panel = found.effects.iter().find(|e| e.id == local)?.designer.as_ref()?;
            let title = &found.panels.iter().find(|p| &p.id == panel)?.title;
            Some(PluginItem {
                plugin: plugin.clone(),
                plugin_name: loaded.bundle.manifest.name.clone(),
                id: panel.clone(),
                title: title.clone(),
                panel: true,
                shortcut: None,
            })
        })
    }

    pub fn install(&self, source: &Path) -> Result<PluginInfo, String> {
        let incoming = Bundle::manifest_of(source).map_err(|e| e.to_string())?;
        craft_plugin_host::validate(&incoming, &Default::default())?;
        let mut inner = self.lock();
        inner.stop(&incoming.id);
        inner.session_only.retain(|b| b.manifest.id != incoming.id);
        let rejects = |bundle: &Bundle| if bundle.manifest.is_data_only() { data_only::check(bundle).map(drop) } else { Ok(()) };
        let bundle = inner.registry.install_checked(source, rejects).map_err(|e| e.to_string())?;
        let (snapshot, digest) = Bundle::read(&bundle.dir).map_err(|e| e.to_string())?;
        if snapshot.manifest.is_data_only() {
            approve_snapshot(&mut inner.registry, &snapshot, &digest)?;
        }
        inner.start(&snapshot);
        Ok(inner.info(&snapshot, false))
    }

    pub fn approve(&self, id: &str) -> Result<PluginInfo, String> {
        let mut inner = self.lock();
        let bundle = inner.find(id)?;
        inner.registry.approve(&bundle).map_err(|e| e.to_string())?;
        inner.start(&bundle);
        Ok(inner.info(&bundle, false))
    }

    pub fn set_sandboxed(&self, id: &str, sandboxed: bool) -> Result<PluginInfo, String> {
        let mut inner = self.lock();
        let bundle = inner.find(id)?;
        if !inner.registry.consent(id).is_some_and(|consent| consent.enabled) {
            return Err(format!("allow plugin `{id}` before choosing how it runs"));
        }
        inner.registry.set_unsandboxed(&bundle.manifest, !sandboxed).map_err(|e| e.to_string())?;
        inner.start(&bundle);
        Ok(inner.info(&bundle, false))
    }

    pub fn disable(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        inner.stop(id);
        let had_session_copy = inner.forget_session_copy(id);
        inner.refused.insert(id.to_string(), Refusal::Disabled);
        match inner.registry.disable(id) {
            Err(BundleError::NotInstalled(_)) if had_session_copy => Ok(()),
            other => other.map_err(|e| e.to_string()),
        }
    }

    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        inner.stop(id);
        let had_session_copy = inner.forget_session_copy(id);
        match inner.registry.uninstall(id) {
            Err(BundleError::NotInstalled(_)) if had_session_copy => Ok(()),
            other => other.map_err(|e| e.to_string()),
        }
    }

    pub fn load_for_session(&self, dir: &Path) -> Result<PluginInfo, String> {
        let bundle = Bundle::open(dir).map_err(|e| e.to_string())?;
        craft_plugin_host::validate(&bundle.manifest, &Default::default())?;
        if !bundle.manifest.has_wasm() && !bundle.manifest.is_data_only() {
            return Err("a native plugin runs only after it is installed and approved".into());
        }
        let mut inner = self.lock();
        inner.session_only.retain(|b| b.manifest.id != bundle.manifest.id);
        inner.session_only.push(bundle.clone());
        inner.start(&bundle);
        let info = inner.info(&bundle, true);
        match &info.error {
            Some(error) => Err(error.clone()),
            None => Ok(info),
        }
    }

    pub fn reload(&self) {
        self.lock().reload();
    }

    fn items(&self) -> Vec<PluginItem> {
        let inner = self.lock();
        let mut items = Vec::new();
        for (id, loaded) in &inner.loaded {
            let found = loaded.plugin.contributions();
            let item = |panel: bool, local: &str, title: &str, shortcut: &Option<String>| PluginItem {
                plugin: id.clone(),
                plugin_name: loaded.bundle.manifest.name.clone(),
                id: local.to_string(),
                title: title.to_string(),
                panel,
                shortcut: shortcut.clone(),
            };
            items.extend(found.panels.iter().map(|p| item(true, &p.id, &p.title, &None)));
            items.extend(found.commands.iter().map(|c| item(false, &c.id, &c.title, &c.shortcut)));
        }
        items
    }
}

static PLUGINS: OnceLock<Plugins> = OnceLock::new();

pub fn plugins() -> Option<&'static Plugins> {
    PLUGINS.get()
}

const PLUGINS_DIR: &str = "Plugins";
const DEVELOPMENT_PATH: &str = "EFFECTCRAFT_PLUGIN_PATH";
const SAFE_MODE_VARIABLE: &str = "EFFECTCRAFT_SAFE_MODE";
static SAFE_MODE: AtomicBool = AtomicBool::new(false);

pub fn start_in_safe_mode() {
    SAFE_MODE.store(true, Ordering::Relaxed);
}

pub fn end_run() {
    if let Some(plugins) = plugins() {
        plugins.end_run();
    }
}

pub fn install(session: &mut Session, config_dir: Option<&Path>) -> &'static Plugins {
    let plugins = PLUGINS.get_or_init(|| {
        let root = match config_dir {
            Some(dir) => dir.join(PLUGINS_DIR),
            None => std::env::temp_dir().join("effectcraft-plugins"),
        };
        let safe_mode = SAFE_MODE.load(Ordering::Relaxed) || std::env::var_os(SAFE_MODE_VARIABLE).is_some_and(|value| value == "1");
        let plugins = Plugins::open(&root, safe_mode);
        let development = std::env::var_os(DEVELOPMENT_PATH).filter(|_| !safe_mode);
        for dir in development.iter().flat_map(std::env::split_paths) {
            if let Err(error) = plugins.load_for_session(&dir) {
                log::warn!("{DEVELOPMENT_PATH}: {}: {error}", dir.display());
            }
        }
        plugins
    });
    session.plugin_commands = Some(command);
    session.set_plugin_items(plugins.items());
    plugins
}

pub(crate) fn running(plugin: &str) -> Result<Arc<Loaded>, String> {
    plugins().and_then(|p| p.get(plugin)).ok_or_else(|| format!("plugin `{plugin}` is not running"))
}

pub(crate) fn effect_owner(effect_id: &str) -> Result<(String, Arc<Loaded>, String), String> {
    let plugins = plugins().ok_or("plugins are not available")?;
    let inner = plugins.lock();
    inner
        .loaded
        .iter()
        .find_map(|(id, loaded)| Some((id.clone(), loaded.clone(), local_effect(id, loaded, effect_id)?.to_string())))
        .ok_or_else(|| format!("the plug-in that provides `{effect_id}` is not running"))
}

pub(crate) fn running_plugins() -> Vec<Arc<Loaded>> {
    plugins().map(Plugins::running_plugins).unwrap_or_default()
}

pub fn image(plugin: &str, key: &str) -> Option<Arc<PluginImage>> {
    plugins()?.get(plugin)?.plugin.image(key)
}

pub fn run_command(session: &mut Session, plugin: &str, command: &str) -> Result<(), String> {
    crate::panels::run_command(session, running(plugin)?, command)
}

pub fn pump(session: &mut Session) -> bool {
    let Some(plugins) = plugins() else { return false };
    let mut running = false;
    for loaded in plugins.running_plugins() {
        loaded.plugin.pump(&mut SessionHost::new(session, &loaded));
        running |= loaded.plugin.running_tasks() > 0;
    }
    running
}

pub const DEFAULT_WAIT: f64 = 600.0;

pub fn wait_for_tasks(session: &mut Session, timeout: f64) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout.clamp(0.0, 86_400.0));
    while pump(session) {
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    true
}

pub fn start_task(session: &mut Session, plugin: &str, task: &str, args: Value) -> Result<u64, String> {
    let loaded = running(plugin)?;
    loaded.plugin.start(task, args, &SessionHost::new(session, &loaded)).map_err(|e| e.to_string())
}

pub fn cancel_task(plugin: &str, id: u64) -> Result<(), String> {
    match running(plugin)?.plugin.cancel(id) {
        true => Ok(()),
        false => Err(format!("plugin `{plugin}` has no task {id}")),
    }
}

fn command(session: &mut Session, id: &str, params: &Value) -> effectcraft_engine::Result<Value> {
    let failed = |message: String| EngineError::Other(message);
    let plugins = plugins().ok_or_else(|| failed("plugins are not available".into()))?;
    let text = |key: &str| params.get(key).and_then(Value::as_str).ok_or_else(|| failed(format!("`{id}` needs a `{key}` string")));
    let info = |info: PluginInfo| serde_json::to_value(info).map_err(|e| failed(e.to_string()));
    let effects_before = effectcraft_engine::effects::dynamic::generation();
    let result = match id {
        "plugin.list" => Ok(json!(plugins.list())),
        "plugin.install" => plugins.install(Path::new(text("path")?)).map_err(failed).and_then(info),
        "plugin.load" => plugins.load_for_session(Path::new(text("path")?)).map_err(failed).and_then(info),
        "plugin.approve" => plugins.approve(text("id")?).map_err(failed).and_then(info),
        "plugin.setSandbox" => {
            let sandboxed = params.get("sandboxed").and_then(Value::as_bool).ok_or_else(|| failed("`plugin.setSandbox` needs a `sandboxed` boolean".into()))?;
            plugins.set_sandboxed(text("id")?, sandboxed).map_err(failed).and_then(info)
        }
        "plugin.disable" => plugins.disable(text("id")?).map(|()| Value::Null).map_err(failed),
        "plugin.uninstall" => plugins.uninstall(text("id")?).map(|()| Value::Null).map_err(failed),
        "plugin.reload" => {
            plugins.reload();
            Ok(Value::Null)
        }
        "plugin.run" => run_command(session, text("plugin")?, text("command")?).map(|()| Value::Null).map_err(failed),
        "plugin.startTask" => {
            let args = params.get("args").cloned().unwrap_or(Value::Null);
            start_task(session, text("plugin")?, text("task")?, args).map(|id| json!(id)).map_err(failed)
        }
        "plugin.cancelTask" => {
            let task = params.get("id").and_then(Value::as_u64).ok_or_else(|| failed("`plugin.cancelTask` needs an `id` number".into()))?;
            cancel_task(text("plugin")?, task).map(|()| Value::Null).map_err(failed)
        }
        "plugin.log" => {
            let after = params.get("after").and_then(Value::as_u64).unwrap_or(0);
            let load = params.get("load").and_then(Value::as_u64);
            running(text("id")?).map(|loaded| json!(loaded.plugin.log_page(after, load))).map_err(failed)
        }
        "plugin.tasks" => {
            let waited = params.get("wait").and_then(Value::as_bool) == Some(true);
            let finished = !waited || wait_for_tasks(session, params.get("timeout").and_then(Value::as_f64).unwrap_or(DEFAULT_WAIT));
            pump(session);
            match finished {
                true => Ok(json!(plugins.tasks())),
                false => Err(failed("plug-in tasks were still running when the wait timed out".into())),
            }
        }
        _ => Err(failed(format!("unknown plugin command `{id}`"))),
    };
    session.set_plugin_items(plugins.items());
    if effectcraft_engine::effects::dynamic::generation() != effects_before {
        match session.edit("Update Plug-in Effects", None, |project, _| Ok(effectcraft_engine::upgrade_effects(project))) {
            Ok(upgraded) => {
                upgraded.lost.iter().for_each(|lost| log::warn!("{lost} lost its animation when its plug-in started"));
                upgraded.kept.into_iter().for_each(|kept| session.toast(kept));
            }
            Err(error) => log::warn!("the open project's plug-in effects were not updated: {error}"),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_refuses_a_bundle_replaced_after_it_was_classified() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("pack");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"{"id": "test.swapped", "name": "Swapped", "version": "1.0.0", "contributes": {"luts": ["a.cube"]}}"#;
        std::fs::write(dir.join("plugin.json"), manifest).unwrap();
        std::fs::write(dir.join("a.cube"), "").unwrap();
        let mut registry = Registry::open(&root.path().join("registry"));
        let installed = registry.install(&dir).unwrap().dir;
        let (snapshot, digest) = Bundle::read(&installed).unwrap();
        std::fs::write(installed.join("plugin.wasm"), b"\0asm").unwrap();
        let error = approve_snapshot(&mut registry, &snapshot, &digest).unwrap_err();
        assert!(error.contains("changed while it was installed"), "{error}");
        assert!(registry.consent("test.swapped").is_none_or(|consent| !consent.enabled));
    }
}
