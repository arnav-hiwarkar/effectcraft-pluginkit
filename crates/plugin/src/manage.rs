use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use craft_plugin_host::ui::{UiInput, Widget};
use craft_plugin_host::{Bundle, BundleError, Call, Limits, Manifest, Plugin, Registry, RunMode};
use craft_plugin_host::{ControlsReply, PluginImage};
use effectcraft_engine::{EngineError, PluginItem, Session};
use serde::Serialize;
use serde_json::{Value, json};

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
    pub(crate) fn report(&self, effect: &str, error: Option<String>) {
        let Ok(mut errors) = self.render_errors.lock() else { return };
        match error {
            Some(error) => {
                if errors.get(effect) != Some(&error) {
                    log::warn!("plugin `{}` effect `{effect}`: {error}", self.bundle.manifest.id);
                }
                errors.insert(effect.to_string(), error);
            }
            None => {
                errors.remove(effect);
            }
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Running,
    NeedsApproval,
    Disabled,
    Failed,
}

#[derive(Serialize, Clone, Debug)]
pub struct PermissionInfo {
    pub name: &'static str,
    pub summary: &'static str,
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
    failed: BTreeMap<String, String>,
}

pub struct Plugins {
    inner: Mutex<Inner>,
}

fn load(bundle: &Bundle, data_dir: PathBuf) -> Result<Arc<Loaded>, String> {
    let entry = bundle.entry_path().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
    let mut hasher = DefaultHasher::new();
    (&bundle.manifest.version, &bytes).hash(&mut hasher);
    let grants = bundle.manifest.permissions.clone();
    let plugin = Plugin::load(bundle, grants, data_dir.clone(), Limits::default()).map_err(|e| e.to_string())?;
    if let Some(effect) = plugin.contributions().effects.iter().find(|e| !plain_id(&e.id)) {
        return Err(format!("effect id `{}` may only use lower case letters, digits, `-` and `_`", effect.id));
    }
    let loaded = Loaded { bundle: bundle.clone(), plugin, data_dir, revision: hasher.finish(), render_errors: Mutex::new(BTreeMap::new()) };
    Ok(Arc::new(loaded))
}

impl Inner {
    fn start(&mut self, bundle: &Bundle) {
        let id = bundle.manifest.id.clone();
        self.stop(&id);
        let bundle = if self.is_session_only(&id) { bundle.clone().with_mode(RunMode::Sandboxed) } else { self.registry.running(bundle) };
        match load(&bundle, self.registry.data_dir(&id)) {
            Ok(loaded) => {
                effects::register(&loaded);
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
        if self.loaded.remove(id).is_some() {
            effects::unregister(id);
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
        let running: Vec<String> = self.loaded.keys().cloned().collect();
        running.iter().for_each(|id| self.stop(id));
        self.failed.clear();
        for bundle in self.installed() {
            if self.registry.grants(&bundle.manifest).is_some() {
                self.start(&bundle);
            }
        }
        for bundle in self.session_only.clone() {
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
        let error = self.failed.get(&manifest.id).cloned();
        let state = match (loaded, &error, self.registry.consent(&manifest.id)) {
            (Some(_), ..) => PluginState::Running,
            (None, Some(_), _) => PluginState::Failed,
            (None, None, Some(consent)) if covers(consent, manifest) => PluginState::Disabled,
            _ => PluginState::NeedsApproval,
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

fn local_effect<'a>(plugin: &str, loaded: &'a Loaded, effect_id: &str) -> Option<&'a str> {
    loaded.plugin.contributions().effects.iter().map(|e| e.id.as_str()).find(|local| effects::effect_id(plugin, local) == effect_id)
}

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn covers(consent: &craft_plugin_host::Consent, manifest: &Manifest) -> bool {
    manifest.permissions.is_subset(&consent.granted) && (manifest.has_wasm() || consent.unsandboxed.as_ref() == Some(&manifest.version))
}

fn reserved(manifest: &Manifest) -> Result<(), String> {
    if manifest.id.starts_with("ec.") {
        return Err("plugin ids starting with `ec.` are reserved for built-in effects".into());
    }
    Ok(())
}

impl Plugins {
    pub fn open(root: &Path) -> Plugins {
        let mut inner = Inner { registry: Registry::open(root), session_only: Vec::new(), loaded: BTreeMap::new(), failed: BTreeMap::new() };
        inner.reload();
        Plugins { inner: Mutex::new(inner) }
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
            Some(PluginItem { plugin: plugin.clone(), plugin_name: loaded.bundle.manifest.name.clone(), id: panel.clone(), title: title.clone(), panel: true })
        })
    }

    pub fn install(&self, source: &Path) -> Result<PluginInfo, String> {
        let incoming = Bundle::open(source).map_err(|e| e.to_string())?;
        reserved(&incoming.manifest)?;
        let mut inner = self.lock();
        inner.stop(&incoming.manifest.id);
        inner.session_only.retain(|b| b.manifest.id != incoming.manifest.id);
        let bundle = inner.registry.install(source).map_err(|e| e.to_string())?;
        if inner.registry.grants(&bundle.manifest).is_some() {
            inner.start(&bundle);
        }
        Ok(inner.info(&bundle, false))
    }

    pub fn approve(&self, id: &str) -> Result<PluginInfo, String> {
        let mut inner = self.lock();
        let bundle = inner.find(id)?;
        inner.registry.approve(&bundle.manifest).map_err(|e| e.to_string())?;
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
        reserved(&bundle.manifest)?;
        if !bundle.manifest.has_wasm() {
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
            let item = |panel: bool, local: &str, title: &str| PluginItem {
                plugin: id.clone(),
                plugin_name: loaded.bundle.manifest.name.clone(),
                id: local.to_string(),
                title: title.to_string(),
                panel,
            };
            items.extend(found.panels.iter().map(|p| item(true, &p.id, &p.title)));
            items.extend(found.commands.iter().map(|c| item(false, &c.id, &c.title)));
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

pub fn install(session: &mut Session, config_dir: Option<&Path>) -> &'static Plugins {
    let plugins = PLUGINS.get_or_init(|| {
        let root = match config_dir {
            Some(dir) => dir.join(PLUGINS_DIR),
            None => std::env::temp_dir().join("effectcraft-plugins"),
        };
        let plugins = Plugins::open(&root);
        for dir in std::env::var_os(DEVELOPMENT_PATH).iter().flat_map(std::env::split_paths) {
            if let Err(error) = plugins.load_for_session(&dir) {
                log::warn!("{DEVELOPMENT_PATH}: {}: {error}", dir.display());
            }
        }
        plugins
    });
    session.plugin_commands = Some(command);
    session.plugin_items = plugins.items();
    plugins
}

fn running(plugin: &str) -> Result<Arc<Loaded>, String> {
    plugins().and_then(|p| p.get(plugin)).ok_or_else(|| format!("plugin `{plugin}` is not running"))
}

pub fn panel_ui(session: &mut Session, plugin: &str, panel: &str, input: UiInput) -> Result<Vec<Widget>, String> {
    let loaded = running(plugin)?;
    let call = Call::Panel { panel: panel.to_string(), input };
    loaded.plugin.call(&call, None, &mut SessionHost::new(session, &loaded)).map_err(|e| e.to_string())
}

pub fn controls(session: &mut Session, effect_id: &str, param: &str, value: &str, input: UiInput) -> Result<(String, ControlsReply), String> {
    let plugins = plugins().ok_or("plugins are not available")?;
    let (owner, plugin, local) = {
        let inner = plugins.lock();
        inner
            .loaded
            .iter()
            .find_map(|(id, loaded)| Some((id.clone(), loaded.clone(), local_effect(id, loaded, effect_id)?.to_string())))
            .ok_or_else(|| format!("the plug-in that provides `{effect_id}` is not running"))?
    };
    let call = Call::Controls { effect: local, param: param.to_string(), value: value.to_string(), input };
    let reply = plugin.plugin.call(&call, None, &mut SessionHost::new(session, &plugin)).map_err(|e| e.to_string())?;
    Ok((owner, reply))
}

pub fn image(plugin: &str, key: &str) -> Option<Arc<PluginImage>> {
    plugins()?.get(plugin)?.plugin.image(key)
}

pub fn run_command(session: &mut Session, plugin: &str, command: &str) -> Result<(), String> {
    let loaded = running(plugin)?;
    let call = Call::Command { command: command.to_string() };
    loaded.plugin.call(&call, None, &mut SessionHost::new(session, &loaded)).map_err(|e| e.to_string())
}

fn command(session: &mut Session, id: &str, params: &Value) -> effectcraft_engine::Result<Value> {
    let failed = |message: String| EngineError::Other(message);
    let plugins = plugins().ok_or_else(|| failed("plugins are not available".into()))?;
    let text = |key: &str| params.get(key).and_then(Value::as_str).ok_or_else(|| failed(format!("`{id}` needs a `{key}` string")));
    let info = |info: PluginInfo| serde_json::to_value(info).map_err(|e| failed(e.to_string()));
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
        _ => Err(failed(format!("unknown plugin command `{id}`"))),
    };
    session.plugin_items = plugins.items();
    result
}
