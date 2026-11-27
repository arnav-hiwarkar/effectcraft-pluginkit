use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use craft_ext_core::catalogue::{Revocation, Severity};
use craft_ext_market::SNAPSHOT;
use craft_ext_store::{APPS_DIR, Consent, location};

use craft_plugin_host::PluginImage;
use craft_plugin_host::ui::PanelPlace;
use craft_plugin_host::{Bundle, BundleError, EffectRegistry, Limits, Permission, Plugin, Refusal, Registry, RunMode, TaskState};
use effectcraft_engine::{EngineError, PluginContext, PluginItem, Session};
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
    command_states: Mutex<BTreeMap<String, (bool, Option<bool>)>>,
}

impl Loaded {
    pub(crate) fn set_command_state(&self, command: &str, enabled: bool, checked: Option<bool>) -> Result<(), String> {
        let commands = &self.plugin.contributions().commands;
        if !commands.iter().any(|spec| spec.id == command) {
            let known: Vec<&str> = commands.iter().map(|spec| spec.id.as_str()).collect();
            return Err(format!("this plugin has no command `{command}`; its commands are {known:?}"));
        }
        let mut states = self.command_states.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        states.insert(command.to_string(), (enabled, checked));
        Ok(())
    }

    fn command_state(&self, command: &str) -> (bool, Option<bool>) {
        let states = self.command_states.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        states.get(command).copied().unwrap_or((true, None))
    }

    pub(crate) fn runnable(&self, command: &str) -> Result<(), String> {
        if self.command_state(command).0 {
            return Ok(());
        }
        let commands = &self.plugin.contributions().commands;
        let title = commands.iter().find(|spec| spec.id == command).map_or(command, |spec| spec.title.as_str());
        Err(format!("{} turned its command `{title}` off for now, so it did not run", self.bundle.manifest.name))
    }

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
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

#[derive(PartialEq, Debug)]
struct Installed {
    consent: Option<Consent>,
    digest: Option<String>,
}

struct Inner {
    registry: Registry,
    suite: Option<PathBuf>,
    generation: u64,
    seen: BTreeMap<String, Installed>,
    catalogue_modified: Option<SystemTime>,
    revocations: Vec<Revocation>,
    last_poll: Instant,
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

fn load(registry: &Registry, bundle: &Bundle, digest: &str, grants: BTreeSet<Permission>) -> Result<Arc<Loaded>, String> {
    let id = &bundle.manifest.id;
    let data_dir = registry.data_dir(id);
    let plugin = Plugin::open_with_protected_paths(bundle, digest, grants, data_dir.clone(), Limits::default(), registry.protected_paths(id))
        .map_err(|e| e.to_string())?;
    let mut hasher = DefaultHasher::new();
    bundle.manifest.version.hash(&mut hasher);
    digest.hash(&mut hasher);
    craft_plugin_host::validate(&bundle.manifest, plugin.contributions())?;
    effects::Engine.admit(plugin.contributions())?;
    let loaded = Loaded {
        bundle: bundle.clone(),
        plugin,
        data_dir,
        revision: hasher.finish(),
        render_errors: Mutex::new(BTreeMap::new()),
        command_states: Mutex::default(),
    };
    Ok(Arc::new(loaded))
}

impl Inner {
    fn start(&mut self, bundle: &Bundle) {
        let id = bundle.manifest.id.clone();
        self.stop(&id);
        if self.safe_mode {
            return;
        }
        let (bundle, grants, digest) = if self.is_session_only(&id) {
            match Bundle::read(&bundle.dir) {
                Ok((read, digest)) if read.manifest.id == id => {
                    self.session_only.iter_mut().filter(|kept| kept.manifest.id == id).for_each(|kept| *kept = read.clone());
                    if let Some(revocation) = self.registry.revocation(&read.manifest)
                        && revocation.severity == Severity::Disable
                    {
                        self.refused.insert(id, Refusal::Revoked(Severity::Disable));
                        return;
                    }
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
        if !bundle.manifest.supports(APP_ID, APP_VERSION) {
            self.failed.insert(id, format!("this plug-in does not support EffectCraft {APP_VERSION}"));
            return;
        }
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
        let loaded = load(&self.registry, &bundle, &digest, grants);
        match loaded {
            Ok(loaded) => {
                effects::Engine.register(&loaded);
                self.loaded.insert(id, loaded);
            }
            Err(error) => {
                log::warn!("plugin `{id}` did not load: {error}");
                self.failed.insert(id, error);
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
        }
    }

    fn sync_store_if_changed(&mut self) {
        if self.store_changed() {
            self.reconcile_store(None);
        }
    }

    fn store_changed(&self) -> bool {
        self.suite.as_deref().is_some_and(|suite| self.registry.generation() != self.generation || catalogue_modified(suite) != self.catalogue_modified)
    }

    fn installed_state(&self, bundles: &[Bundle]) -> BTreeMap<String, Installed> {
        let state = |bundle: &Bundle| {
            let id = bundle.manifest.id.clone();
            let installed = Installed { consent: self.registry.consent(&id).cloned(), digest: bundle.digest().ok() };
            (id, installed)
        };
        bundles.iter().map(state).collect()
    }

    fn reconcile_store(&mut self, restart: Option<&str>) -> bool {
        self.reconcile_store_with(restart, craft_ext_market::cached_revocations)
    }

    fn reconcile_store_with(&mut self, restart: Option<&str>, read_revocations: impl FnOnce(&Path) -> Vec<Revocation>) -> bool {
        if self.refresh_store(read_revocations) {
            self.reload_bundles();
            return true;
        }
        let bundles = self.installed();
        let now = self.installed_state(&bundles);
        let mut changed: BTreeSet<&str> =
            self.seen.keys().chain(now.keys()).filter(|id| self.seen.get(*id) != now.get(*id) && !self.is_session_only(id)).map(String::as_str).collect();
        changed.extend(restart);
        let mut starts: Vec<Bundle> =
            bundles.into_iter().filter(|bundle| changed.contains(bundle.manifest.id.as_str()) && !self.is_session_only(&bundle.manifest.id)).collect();
        starts.extend(self.session_only.iter().filter(|bundle| restart == Some(bundle.manifest.id.as_str())).cloned());
        let changed: Vec<String> = changed.into_iter().map(String::from).collect();
        changed.iter().for_each(|id| self.stop(id));
        starts.sort_by_key(|bundle| bundle.manifest.is_data_only());
        starts.iter().for_each(|bundle| self.start(bundle));
        self.seen = now;
        !changed.is_empty()
    }

    fn refresh_store(&mut self, read_revocations: impl FnOnce(&Path) -> Vec<Revocation>) -> bool {
        let Some(suite) = &self.suite else { return false };
        self.generation = self.registry.generation();
        self.registry = Registry::for_app(suite, APP_ID);
        let modified = catalogue_modified(suite);
        let mut changed = false;
        if modified != self.catalogue_modified {
            let revocations = read_revocations(suite);
            changed = self.revocations != revocations;
            self.revocations = revocations;
            self.catalogue_modified = modified;
        }
        self.registry.set_revocations(self.revocations.clone());
        changed
    }

    fn poll_store(&mut self, read_revocations: impl FnOnce(&Path) -> Vec<Revocation>) -> bool {
        if self.last_poll.elapsed() < STORE_POLL_INTERVAL {
            return false;
        }
        self.last_poll = Instant::now();
        self.store_changed() && self.reconcile_store_with(None, read_revocations)
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
        self.refresh_store(craft_ext_market::cached_revocations);
        self.reload_bundles();
    }

    fn reload_bundles(&mut self) {
        let mut bundles = self.installed();
        self.seen = self.installed_state(&bundles);
        let running: Vec<String> = self.loaded.keys().chain(&self.data).cloned().collect();
        running.iter().for_each(|id| self.stop(id));
        self.failed.clear();
        self.refused.clear();
        bundles.retain(|bundle| !self.is_session_only(&bundle.manifest.id));
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
        let named = |id: &str, title: &str| Contribution { id: id.to_string(), title: title.to_string(), enabled: None, checked: None };
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
            commands: found
                .commands
                .iter()
                .map(|c| {
                    let (enabled, checked) = loaded.map_or((true, None), |loaded| loaded.command_state(&c.id));
                    Contribution { enabled: Some(enabled), checked, ..named(&c.id, &c.title) }
                })
                .collect(),
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
pub const APP_ID: &str = "effectcraft";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const STORE_POLL_INTERVAL: Duration = Duration::from_secs(1);

fn catalogue_modified(suite: &Path) -> Option<SystemTime> {
    fs::metadata(suite.join(craft_ext_store::CATALOGUE_DIR).join(SNAPSHOT)).ok()?.modified().ok()
}

fn issues_url(source: &str) -> Option<String> {
    let mut url = url::Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    if path.split('/').filter(|part| !part.is_empty()).count() < 2 {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    let suffix = if url.host_str() == Some("gitlab.com") { "/-/issues" } else { "/issues" };
    let path = if path.ends_with(suffix) { path.to_string() } else { format!("{path}{suffix}") };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

impl Plugins {
    pub fn open(root: &Path, safe_mode: bool) -> Plugins {
        let mut registry = Registry::open(root);
        match registry.recover() {
            Ok(quarantined) => quarantined.iter().for_each(|id| log::warn!("plug-in `{id}` was turned off: EffectCraft closed unexpectedly while it ran")),
            Err(error) => log::warn!("the last run's plug-in marker was not read: {error}"),
        }
        Self::with_registry(registry, None, safe_mode)
    }

    fn with_registry(registry: Registry, suite: Option<PathBuf>, safe_mode: bool) -> Plugins {
        let mut inner = Inner {
            registry,
            suite,
            generation: 0,
            seen: BTreeMap::new(),
            catalogue_modified: None,
            revocations: Vec::new(),
            last_poll: Instant::now(),
            session_only: Vec::new(),
            loaded: BTreeMap::new(),
            data: BTreeSet::new(),
            failed: BTreeMap::new(),
            refused: BTreeMap::new(),
            safe_mode,
        };
        inner.reload();
        Plugins { inner: Mutex::new(inner) }
    }

    fn presets_root(&self) -> PathBuf {
        let inner = self.lock();
        inner.suite.clone().unwrap_or_else(|| inner.registry.root().to_path_buf())
    }

    pub fn store_generation(&self) -> u64 {
        self.lock().generation
    }

    pub fn poll_store(&self, session: &mut Session) {
        let before = effectcraft_engine::effects::dynamic::generation();
        if self.lock().poll_store(craft_ext_market::cached_revocations) {
            self.refresh_session(session, before);
        }
    }

    fn refresh_session(&self, session: &mut Session, effects_before: u64) {
        session.set_plugin_items(self.items());
        if effectcraft_engine::effects::dynamic::generation() != effects_before {
            match session.edit("Update Plug-in Effects", None, |project, _| Ok(effectcraft_engine::upgrade_effects(project))) {
                Ok(upgraded) => {
                    upgraded.lost.iter().for_each(|lost| log::warn!("{lost} lost its animation when its plug-in started"));
                    upgraded.kept.into_iter().for_each(|kept| session.toast(kept));
                }
                Err(error) => log::warn!("the open project's plug-in effects were not updated: {error}"),
            }
        }
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
                ..Default::default()
            })
        })
    }

    pub fn install(&self, source: &Path) -> Result<PluginInfo, String> {
        let incoming = Bundle::manifest_of(source).map_err(|e| e.to_string())?;
        craft_plugin_host::validate(&incoming, &Default::default())?;
        let mut inner = self.lock();
        inner.sync_store_if_changed();
        let rejects = |bundle: &Bundle| if bundle.manifest.is_data_only() { data_only::check(bundle).map(drop) } else { Ok(()) };
        let bundle = inner.registry.install_checked(source, rejects).map_err(|e| e.to_string())?;
        inner.forget_session_copy(&incoming.id);
        let result = Bundle::read(&bundle.dir).map_err(|e| e.to_string()).and_then(|(snapshot, digest)| {
            if snapshot.manifest.is_data_only() {
                approve_snapshot(&mut inner.registry, &snapshot, &digest)?;
            }
            Ok(snapshot)
        });
        inner.reconcile_store(Some(&incoming.id));
        Ok(inner.info(&result?, false))
    }

    pub fn approve(&self, id: &str) -> Result<PluginInfo, String> {
        let mut inner = self.lock();
        inner.sync_store_if_changed();
        let bundle = inner.find(id)?;
        let result = inner.registry.approve(&bundle);
        inner.reconcile_store(result.is_ok().then_some(id));
        result.map_err(|e| e.to_string())?;
        Ok(inner.info(&bundle, false))
    }

    pub fn set_sandboxed(&self, id: &str, sandboxed: bool) -> Result<PluginInfo, String> {
        let mut inner = self.lock();
        inner.sync_store_if_changed();
        let bundle = inner.find(id)?;
        if !inner.registry.consent(id).is_some_and(|consent| consent.enabled) {
            return Err(format!("allow plugin `{id}` before choosing how it runs"));
        }
        let result = inner.registry.set_unsandboxed(&bundle.manifest, !sandboxed);
        inner.reconcile_store(result.is_ok().then_some(id));
        result.map_err(|e| e.to_string())?;
        Ok(inner.info(&bundle, false))
    }

    pub fn disable(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        let had_session_copy = inner.forget_session_copy(id);
        let result = inner.registry.disable(id);
        inner.reconcile_store(had_session_copy.then_some(id));
        match result {
            Err(BundleError::NotInstalled(_)) if had_session_copy => Ok(()),
            other => other.map_err(|e| e.to_string()),
        }
    }

    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        let had_session_copy = inner.forget_session_copy(id);
        let result = inner.registry.uninstall(id);
        inner.reconcile_store(had_session_copy.then_some(id));
        match result {
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
        inner.sync_store_if_changed();
        inner.session_only.retain(|b| b.manifest.id != bundle.manifest.id);
        inner.session_only.push(bundle.clone());
        inner.start(&bundle);
        let info = inner.info(&bundle, true);
        match &info.error {
            Some(error) => Err(error.clone()),
            None => Ok(info),
        }
    }

    fn restart(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        inner.sync_store_if_changed();
        let bundle = inner.find(id)?;
        inner.start(&bundle);
        let info = inner.info(&bundle, inner.is_session_only(id));
        if info.state == PluginState::Running {
            return Ok(());
        }
        Err(info.error.unwrap_or_else(|| format!("plugin `{id}` is not enabled")))
    }

    fn report_url(&self, id: Option<&str>) -> String {
        const DEFAULT_ISSUES: &str = "https://github.com/arnav-hiwarkar/craft-plugins/issues";
        let inner = self.lock();
        let Some(id) = id else { return DEFAULT_ISSUES.into() };
        let installed = inner.find(id).ok().and_then(|bundle| issues_url(&bundle.manifest.source));
        installed
            .or_else(|| {
                let suite = inner.suite.as_ref()?;
                let id = craft_ext_core::id::ExtensionId::new(id).ok()?;
                let index = craft_ext_market::suite_catalogue(suite).cached().ok()?;
                issues_url(&index.package(&id)?.repo)
            })
            .unwrap_or_else(|| DEFAULT_ISSUES.into())
    }

    pub fn reload(&self) {
        self.lock().reload();
    }

    pub(crate) fn items(&self) -> Vec<PluginItem> {
        let inner = self.lock();
        let mut items = Vec::new();
        for (id, loaded) in &inner.loaded {
            let found = loaded.plugin.contributions();
            let item = |local: &str, title: &str| PluginItem {
                plugin: id.clone(),
                plugin_name: loaded.bundle.manifest.name.clone(),
                id: local.to_string(),
                title: title.to_string(),
                ..Default::default()
            };
            let docked = found.panels.iter().filter(|p| !matches!(p.place, PanelPlace::Status | PanelPlace::Bar));
            items.extend(docked.map(|p| PluginItem { panel: true, multiple: p.multiple, ..item(&p.id, &p.title) }));
            items.extend(found.commands.iter().map(|c| {
                let (enabled, checked) = loaded.command_state(&c.id);
                PluginItem {
                    disabled: !enabled,
                    checked,
                    shortcut: c.shortcut.clone(),
                    menu: c.menu.clone(),
                    context: c
                        .context
                        .iter()
                        .map(|context| match context {
                            craft_plugin_host::ui::Context::Layers => PluginContext::Layers,
                            craft_plugin_host::ui::Context::Timeline => PluginContext::Timeline,
                            craft_plugin_host::ui::Context::Viewer => PluginContext::Viewer,
                        })
                        .collect(),
                    toolbar: c.toolbar,
                    icon: c.icon.clone(),
                    ..item(&c.id, &c.title)
                }
            }));
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

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn copy_data(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    match fs::symlink_metadata(from) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(Error::new(ErrorKind::NotADirectory, from.display().to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    match fs::symlink_metadata(to) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => return Err(Error::new(ErrorKind::NotADirectory, to.display().to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let name = to.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let staging = to.with_file_name(format!(".{name}.migrating"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    copy_tree(from, &staging)?;
    fs::rename(&staging, to)
}

fn prepare_migration_bundle(target: &mut Registry, bundle: &Bundle) -> Result<Option<(String, String)>, Box<dyn std::error::Error>> {
    let id = &bundle.manifest.id;
    let dest = target.root().join(id);
    let digest = bundle.digest()?;
    let migrated = if dest.try_exists()? {
        Bundle::open(&dest)?
    } else {
        match target.install_checked(&bundle.dir, |_| match dest.try_exists() {
            Ok(false) => Ok(()),
            Ok(true) => Err(format!("the shared store already has a plug-in `{id}`")),
            Err(error) => Err(error.to_string()),
        }) {
            Ok(installed) => installed,
            Err(BundleError::Rejected(_)) => Bundle::open(&dest)?,
            Err(BundleError::Refused(_, Refusal::Revoked(_))) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    };
    if migrated.digest()? != digest {
        log::warn!("the shared store already has a different plug-in `{id}`");
        return Ok(None);
    }
    Ok(Some((id.clone(), digest)))
}

fn merge_migration_bundle(
    source: &Registry,
    target: &Registry,
    id: String,
    digest: &str,
    consents: &mut BTreeMap<String, Consent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, installed_digest) = Bundle::read(&target.root().join(&id))?;
    if installed_digest != digest {
        return Err(format!("the shared plug-in `{id}` changed during migration").into());
    }
    copy_data(&source.data_dir(&id), &target.data_dir(&id)).map_err(|error| format!("could not migrate data for plug-in `{id}`: {error}"))?;
    if let Some(consent) = source.consent(&id) {
        consents.entry(id).or_insert_with(|| consent.clone());
    }
    Ok(())
}

fn migrate(legacy: &Path, suite: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let backup = legacy.with_extension("migrated");
    if !legacy.is_dir() || backup.exists() {
        return Ok(());
    }
    let state_dir = suite.join(APPS_DIR).join(APP_ID);
    fs::create_dir_all(&state_dir)?;
    let migration_lock = File::options().read(true).write(true).create(true).truncate(false).open(state_dir.join("migration.lock"))?;
    migration_lock.lock()?;
    if !legacy.is_dir() || backup.exists() {
        return Ok(());
    }

    let mut target = Registry::for_app(suite, APP_ID);
    if target.root().canonicalize().ok().as_deref() == Some(&legacy.canonicalize()?) {
        return Err("the old plug-in folder is also the shared bundle folder".into());
    }
    target.set_revocations(craft_ext_market::cached_revocations(suite));
    let source = Registry::open(legacy);
    let candidates: Vec<_> = source
        .bundles()
        .into_iter()
        .filter_map(|bundle| match bundle.map_err(Into::into).and_then(|bundle| prepare_migration_bundle(&mut target, &bundle)) {
            Ok(candidate) => candidate,
            Err(error) => {
                log::warn!("a plug-in in the old store was not moved into the shared store: {error}");
                None
            }
        })
        .collect();

    let lock = File::options().read(true).write(true).create(true).truncate(false).open(state_dir.join("state.lock"))?;
    lock.lock()?;
    let state = state_dir.join("state.json");
    let mut consents: BTreeMap<String, Consent> = match File::open(&state) {
        Ok(file) => serde_json::from_reader(file)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error.into()),
    };
    let mut retry = false;
    for (id, digest) in candidates {
        if let Err(error) = merge_migration_bundle(&source, &target, id, &digest, &mut consents) {
            retry = true;
            log::warn!("a plug-in in the old store was not moved into the shared store: {error}");
        }
    }
    let mut staging = tempfile::NamedTempFile::new_in(&state_dir)?;
    serde_json::to_writer_pretty(&mut staging, &consents)?;
    staging.as_file().sync_all()?;
    staging.persist(&state)?;

    let generation = suite.join(craft_ext_store::GENERATION_FILE);
    let generation_lock = File::options().read(true).write(true).create(true).truncate(false).open(generation.with_extension("lock"))?;
    generation_lock.lock()?;
    let mut staging = tempfile::NamedTempFile::new_in(suite)?;
    std::io::Write::write_all(&mut staging, target.generation().saturating_add(1).to_string().as_bytes())?;
    staging.persist(&generation)?;
    if !retry {
        fs::rename(legacy, backup)?;
    }
    Ok(())
}

fn resolve_suite_root(private: Option<&Path>, extensions_override: bool) -> Result<PathBuf, String> {
    match private.filter(|_| !extensions_override) {
        Some(root) => Ok(root.to_path_buf()),
        None => location::default_root().map_err(|error| error.to_string()),
    }
}

/// The suite folder the running plug-ins come from, when they come from a shared suite store
/// rather than a private test store.
pub fn suite_root() -> Option<PathBuf> {
    plugins()?.lock().suite.clone()
}

enum Store<'a> {
    Shared { legacy: Option<PathBuf>, private: Option<PathBuf> },
    Isolated(&'a Path),
}

fn open_store(store: Store, safe_mode: bool) -> Plugins {
    match store {
        Store::Isolated(root) => Plugins::open(root, safe_mode),
        Store::Shared { legacy, private } => match resolve_suite_root(private.as_deref(), std::env::var_os(location::OVERRIDE_VAR).is_some()) {
            Ok(suite) => {
                if let Some(legacy) = legacy
                    && let Err(error) = migrate(&legacy, &suite)
                {
                    log::warn!("the old plug-in store was not migrated: {error}");
                }
                Plugins::with_registry(Registry::for_app(&suite, APP_ID), Some(suite), safe_mode)
            }
            Err(error) => {
                log::warn!("the shared plug-in store could not be opened: {error}");
                Plugins::open(&legacy.unwrap_or_else(|| std::env::temp_dir().join("effectcraft-plugins")), safe_mode)
            }
        },
    }
}

fn install_store(session: &mut Session, store: Store) -> &'static Plugins {
    let plugins = PLUGINS.get_or_init(|| {
        let safe_mode = SAFE_MODE.load(Ordering::Relaxed) || std::env::var_os(SAFE_MODE_VARIABLE).is_some_and(|value| value == "1");
        let plugins = open_store(store, safe_mode);
        let development = std::env::var_os(DEVELOPMENT_PATH).filter(|_| !safe_mode);
        for dir in development.iter().flat_map(std::env::split_paths) {
            if let Err(error) = plugins.load_for_session(&dir) {
                log::warn!("{DEVELOPMENT_PATH}: {}: {error}", dir.display());
            }
        }
        plugins
    });
    session.plugin_commands = Some(command);
    session.plugin_hooks = Some(crate::hooks::ask);
    session.user_presets = Some(crate::presets::SuitePresets::open(&plugins.presets_root()));
    session.set_plugin_items(plugins.items());
    plugins
}

/// Plugins from the suite store shared with the launcher and the other Craft apps, moving the old
/// `<config_dir>/Plugins` folder into it once. `private_suite` replaces the shared suite for a
/// private data folder (tests, agents, a Nightly profile), unless `CRAFT_EXT_DIR` names one.
pub fn install_shared(session: &mut Session, config_dir: Option<&Path>, private_suite: Option<&Path>) -> &'static Plugins {
    let legacy = config_dir.map(|dir| dir.join(PLUGINS_DIR));
    install_store(session, Store::Shared { legacy, private: private_suite.map(Path::to_path_buf) })
}

/// Plugins from a private store under `config_dir` (or the temporary folder), for tests and
/// embedders that must not touch the shared one.
pub fn install(session: &mut Session, config_dir: Option<&Path>) -> &'static Plugins {
    let root = config_dir.map_or_else(|| std::env::temp_dir().join("effectcraft-plugins"), |dir| dir.join(PLUGINS_DIR));
    install_store(session, Store::Isolated(&root))
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

pub fn panel_spec(plugin: &str, panel: &str) -> Option<craft_plugin_host::ui::PanelSpec> {
    plugins()?.get(plugin)?.plugin.contributions().panels.iter().find(|spec| spec.id == panel).cloned()
}

pub fn panel_request(plugin: &str, panel: &str, instance: Option<&str>, close: bool) -> Result<craft_plugin_host::ui::PanelSpec, String> {
    let spec = panel_spec(plugin, panel).ok_or_else(|| format!("no running plug-in `{plugin}` has a panel `{panel}`"))?;
    if matches!(spec.place, PanelPlace::Status | PanelPlace::Bar) {
        return Err(format!(
            "panel `{panel}` is a status item or an option bar, which the app shows by itself while the plug-in runs or its tool is active; it cannot be opened or closed"
        ));
    }
    match (spec.multiple, instance) {
        (true, None) if close => Err(format!("say which `{panel}` to close: host::close_panel(\"{panel}\", Some(key))")),
        (false, Some(_)) => Err(format!("panel `{panel}` has one instance; declare it with Plugin::panel_each to open one per key")),
        _ => Ok(spec),
    }
}

pub fn placed_panels(place: PanelPlace) -> Vec<(String, craft_plugin_host::ui::PanelSpec)> {
    let at = |loaded: &Arc<Loaded>| {
        let panels = loaded.plugin.contributions().panels.iter().filter(|spec| spec.place == place);
        panels.map(|spec| (loaded.bundle.manifest.id.clone(), spec.clone())).collect::<Vec<_>>()
    };
    running_plugins().iter().flat_map(at).collect()
}

pub fn image(plugin: &str, key: &str) -> Option<Arc<PluginImage>> {
    plugins()?.get(plugin)?.plugin.image(key)
}

pub fn run_command(session: &mut Session, plugin: &str, command: &str) -> Result<(), String> {
    crate::panels::run_command(session, running(plugin)?, command)
}

pub fn pump(session: &mut Session) -> bool {
    let Some(plugins) = plugins() else { return false };
    plugins.poll_store(session);
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
        "plugin.restart" => plugins.restart(text("id")?).map(|()| Value::Null).map_err(failed),
        "plugin.report" => {
            let url = plugins.report_url(params.get("id").and_then(Value::as_str));
            session.events.push(effectcraft_engine::Event::OpenUrl(url.clone()));
            Ok(json!({"url": url}))
        }
        "plugin.reload" => {
            plugins.reload();
            Ok(Value::Null)
        }
        "plugin.press" => {
            let number = |key: &str| params.get(key).and_then(Value::as_u64).ok_or_else(|| failed(format!("`plugin.press` needs a `{key}` number")));
            crate::panels::press_now(session, number("layer")?, number("prop")?).map(|()| Value::Null).map_err(failed)
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
    plugins.refresh_session(session, effects_before);
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

    fn data_bundle(dir: &Path, id: &str) -> PathBuf {
        let bundle = dir.join(id);
        std::fs::create_dir_all(&bundle).unwrap();
        let manifest = format!(r#"{{"id": "{id}", "name": "Looks", "version": "1.0.0", "contributes": {{"luts": ["a.cube"]}}}}"#);
        std::fs::write(bundle.join("plugin.json"), manifest).unwrap();
        std::fs::write(bundle.join("a.cube"), "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        bundle
    }

    fn disable_revocation(id: &str) -> Revocation {
        Revocation {
            id: craft_ext_core::id::ExtensionId::new(id).ok(),
            versions: None,
            sha256: None,
            vendor: None,
            severity: Severity::Disable,
            reason: "stolen keys".into(),
            date: "2026-11-27".into(),
        }
    }

    #[test]
    fn the_old_plugin_folder_moves_into_the_suite_once_with_its_consent_and_data() {
        let root = tempfile::tempdir().unwrap();
        let (legacy, suite) = (root.path().join("Plugins"), root.path().join("suite"));
        let mut old = Registry::open(&legacy);
        let installed = old.install(&data_bundle(root.path(), "test.moved")).unwrap();
        old.approve(&installed).unwrap();
        std::fs::create_dir_all(old.data_dir("test.moved")).unwrap();
        std::fs::write(old.data_dir("test.moved").join("prefs.json"), "{}").unwrap();

        migrate(&legacy, &suite).unwrap();

        let moved = Registry::for_app(&suite, APP_ID);
        assert!(moved.consent("test.moved").is_some_and(|consent| consent.enabled));
        assert_eq!(std::fs::read_to_string(moved.data_dir("test.moved").join("prefs.json")).unwrap(), "{}");
        assert!(!legacy.exists() && legacy.with_extension("migrated").is_dir());

        std::fs::create_dir_all(&legacy).unwrap();
        migrate(&legacy, &suite).unwrap();
        assert!(legacy.is_dir());
    }

    #[test]
    fn one_unreadable_old_plugin_does_not_stop_the_others_from_moving() {
        let root = tempfile::tempdir().unwrap();
        let (legacy, suite) = (root.path().join("Plugins"), root.path().join("suite"));
        let mut old = Registry::open(&legacy);
        let installed = old.install(&data_bundle(root.path(), "test.good")).unwrap();
        old.approve(&installed).unwrap();
        std::fs::create_dir_all(old.data_dir("test.good")).unwrap();
        std::fs::write(old.data_dir("test.good").join("prefs.json"), "{}").unwrap();
        let broken = legacy.join("test.broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("plugin.json"), "not json").unwrap();
        let interrupted = Registry::for_app(&suite, APP_ID).data_dir("test.good").with_file_name(".test.good.migrating");
        std::fs::create_dir_all(&interrupted).unwrap();
        std::fs::write(interrupted.join("half.json"), "{").unwrap();

        migrate(&legacy, &suite).unwrap();

        let moved = Registry::for_app(&suite, APP_ID);
        assert!(moved.consent("test.good").is_some_and(|consent| consent.enabled));
        assert!(moved.data_dir("test.good").join("prefs.json").is_file());
        assert!(!moved.data_dir("test.good").join("half.json").exists());
        assert!(!interrupted.exists());
        assert!(!legacy.exists() && legacy.with_extension("migrated").join("test.broken").is_dir());
    }

    #[test]
    fn failed_data_copy_stays_disabled_and_retries_without_undoing_other_migrations() {
        let root = tempfile::tempdir().unwrap();
        let (legacy, suite) = (root.path().join("Plugins"), root.path().join("suite"));
        let mut source = Registry::open(&legacy);
        let target = Registry::for_app(&suite, APP_ID);
        for id in ["test.copy-good", "test.copy-failed"] {
            let bundle = source.install(&data_bundle(root.path(), id)).unwrap();
            source.approve(&bundle).unwrap();
            fs::create_dir_all(source.data_dir(id)).unwrap();
            fs::write(source.data_dir(id).join("settings.json"), id).unwrap();
        }
        let blocked = target.data_dir("test.copy-failed");
        fs::create_dir_all(blocked.parent().unwrap()).unwrap();
        fs::write(&blocked, "obstruction").unwrap();

        migrate(&legacy, &suite).unwrap();

        let mut target = Registry::for_app(&suite, APP_ID);
        assert!(target.consent("test.copy-good").unwrap().enabled);
        assert_eq!(fs::read_to_string(target.data_dir("test.copy-good").join("settings.json")).unwrap(), "test.copy-good");
        assert!(target.consent("test.copy-failed").is_none());
        assert!(source.data_dir("test.copy-failed").join("settings.json").is_file());
        assert!(!legacy.with_extension("migrated").exists());

        target.disable("test.copy-good").unwrap();
        fs::remove_file(blocked).unwrap();
        migrate(&legacy, &suite).unwrap();

        let target = Registry::for_app(&suite, APP_ID);
        assert!(!target.consent("test.copy-good").unwrap().enabled);
        assert!(target.consent("test.copy-failed").unwrap().enabled);
        assert_eq!(fs::read_to_string(target.data_dir("test.copy-failed").join("settings.json")).unwrap(), "test.copy-failed");
        assert!(!legacy.exists() && legacy.with_extension("migrated").is_dir());
    }

    #[test]
    fn migration_does_not_transfer_consent_to_a_changed_shared_bundle() {
        let root = tempfile::tempdir().unwrap();
        let mut source = Registry::open(&root.path().join("Plugins"));
        let mut target = Registry::for_app(&root.path().join("suite"), APP_ID);
        let bundle = source.install(&data_bundle(root.path(), "test.changed-migration")).unwrap();
        source.approve(&bundle).unwrap();
        let (id, digest) = prepare_migration_bundle(&mut target, &bundle).unwrap().unwrap();
        fs::write(target.root().join(&id).join("a.cube"), "LUT_1D_SIZE 2\n1 1 1\n0 0 0\n").unwrap();

        let mut consents = BTreeMap::new();
        let error = merge_migration_bundle(&source, &target, id, &digest, &mut consents).unwrap_err();
        assert_eq!(error.to_string(), "the shared plug-in `test.changed-migration` changed during migration");
        assert!(consents.is_empty());
    }

    #[test]
    fn a_store_change_restarts_only_the_plugins_it_touched() {
        let root = tempfile::tempdir().unwrap();
        let suite = root.path().join("suite");
        let mut registry = Registry::for_app(&suite, APP_ID);
        for id in ["test.kept", "test.turned"] {
            let installed = registry.install(&data_bundle(root.path(), id)).unwrap();
            registry.approve(&installed).unwrap();
        }
        let plugins = Plugins::with_registry(registry, Some(suite.clone()), false);

        plugins.disable("test.kept").unwrap();
        plugins.approve("test.kept").unwrap();
        {
            let mut inner = plugins.lock();
            assert_eq!(inner.generation, inner.registry.generation());
            assert!(!inner.reconcile_store(None));
        }

        Registry::for_app(&suite, APP_ID).disable("test.turned").unwrap();
        let mut inner = plugins.lock();
        assert_ne!(inner.generation, inner.registry.generation());
        assert!(inner.reconcile_store(None));
        assert!(inner.data.contains("test.kept"));
        assert!(!inner.data.contains("test.turned"));
        assert_eq!(inner.refused.get("test.turned"), Some(&Refusal::Disabled));
    }

    #[test]
    fn a_local_approval_also_applies_a_launcher_disable() {
        let root = tempfile::tempdir().unwrap();
        let suite = root.path().join("suite");
        let mut registry = Registry::for_app(&suite, APP_ID);
        let running = registry.install(&data_bundle(root.path(), "test.launcher-disabled")).unwrap();
        registry.approve(&running).unwrap();
        registry.install(&data_bundle(root.path(), "test.locally-approved")).unwrap();
        let plugins = Plugins::with_registry(registry, Some(suite.clone()), false);
        assert!(plugins.lock().data.contains("test.launcher-disabled"));

        Registry::for_app(&suite, APP_ID).disable("test.launcher-disabled").unwrap();
        assert_eq!(plugins.approve("test.locally-approved").unwrap().state, PluginState::Running);

        let inner = plugins.lock();
        assert!(!inner.data.contains("test.launcher-disabled"));
        assert_eq!(inner.refused.get("test.launcher-disabled"), Some(&Refusal::Disabled));
    }

    #[test]
    fn a_plugin_the_catalogue_disables_is_refused_at_load_without_a_network() {
        let root = tempfile::tempdir().unwrap();
        let suite = root.path().join("suite");
        let mut registry = Registry::for_app(&suite, APP_ID);
        let installed = registry.install(&data_bundle(root.path(), "test.revoked")).unwrap();
        registry.approve(&installed).unwrap();
        let plugins = Plugins::with_registry(registry, Some(suite), false);
        assert!(plugins.lock().data.contains("test.revoked"));

        let mut inner = plugins.lock();
        inner.registry.set_revocations(vec![disable_revocation("test.revoked")]);
        inner.reload_bundles();
        assert_eq!(inner.refused.get("test.revoked"), Some(&Refusal::Revoked(Severity::Disable)));
        assert!(!inner.data.contains("test.revoked"));
    }

    #[test]
    fn a_store_poll_applies_revocation_and_consent_changes_together() {
        let root = tempfile::tempdir().unwrap();
        let suite = root.path().join("suite");
        let mut registry = Registry::for_app(&suite, APP_ID);
        for id in ["test.poll-revoked", "test.poll-disabled"] {
            let installed = registry.install(&data_bundle(root.path(), id)).unwrap();
            registry.approve(&installed).unwrap();
        }
        let plugins = Plugins::with_registry(registry, Some(suite.clone()), false);
        let mut inner = plugins.lock();
        assert!(inner.data.contains("test.poll-revoked"));
        assert!(inner.data.contains("test.poll-disabled"));

        Registry::for_app(&suite, APP_ID).disable("test.poll-disabled").unwrap();
        let catalogue = suite.join(craft_ext_store::CATALOGUE_DIR);
        fs::create_dir_all(&catalogue).unwrap();
        fs::write(catalogue.join(SNAPSHOT), "").unwrap();
        inner.last_poll = Instant::now() - STORE_POLL_INTERVAL;
        assert!(inner.poll_store(|_| vec![disable_revocation("test.poll-revoked")]));

        assert!(!inner.data.contains("test.poll-revoked"));
        assert!(!inner.data.contains("test.poll-disabled"));
        assert_eq!(inner.refused.get("test.poll-revoked"), Some(&Refusal::Revoked(Severity::Disable)));
        assert_eq!(inner.refused.get("test.poll-disabled"), Some(&Refusal::Disabled));
    }

    #[test]
    fn a_private_suite_is_used_unless_the_extensions_folder_is_set() {
        let private = Path::new("/tmp/ec-config/Extensions");
        assert_eq!(resolve_suite_root(Some(private), false).as_deref(), Ok(private));
        assert_eq!(resolve_suite_root(Some(private), true), resolve_suite_root(None, false));
    }

    #[test]
    fn a_problem_report_goes_to_the_issues_of_the_plugins_source_repository() {
        assert_eq!(issues_url("https://github.com/acme/glow.git").as_deref(), Some("https://github.com/acme/glow/issues"));
        assert_eq!(issues_url("https://github.com/acme/glow/issues/").as_deref(), Some("https://github.com/acme/glow/issues"));
        assert_eq!(issues_url("https://gitlab.com/acme/glow").as_deref(), Some("https://gitlab.com/acme/glow/-/issues"));
        assert_eq!(issues_url("https://user:token@github.com/acme/glow"), None);
        assert_eq!(issues_url("file:///home/acme/glow"), None);
        assert_eq!(issues_url("https://github.com/acme"), None);
    }
}
