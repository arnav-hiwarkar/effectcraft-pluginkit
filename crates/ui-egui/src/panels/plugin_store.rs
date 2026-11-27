use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use craft_ext_manager::ui::{self, Theme};
use craft_ext_manager::{Action, Effect, Job, Model, Outcome, Scope, StoreBackend, Tab, Word};
use egui::RichText;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::EffectcraftApp;
use crate::i18n::{tr, tr_args};
use crate::theme::Tokens;

#[derive(Deserialize, Clone)]
pub(crate) struct Listed {
    pub id: String,
    pub name: String,
    pub state: State,
    pub error: Option<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Running,
    NeedsApproval,
    Disabled,
    Quarantined,
    Failed,
}

#[derive(Deserialize, Clone)]
struct ListedTask {
    plugin: String,
    id: u64,
    task: String,
    status: TaskStatus,
    progress: f64,
    message: String,
    error: Option<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

struct Manager {
    model: Model,
    backend: Arc<Mutex<StoreBackend>>,
    jobs: Sender<Outcome>,
    done: Receiver<Outcome>,
    generation: u64,
}

impl Manager {
    fn start() -> Result<Manager, String> {
        let backend = StoreBackend::open_default(vec![effectcraft_plugin::APP_ID.to_string()])?;
        let scope = Scope::App { id: effectcraft_plugin::APP_ID.to_string(), version: effectcraft_plugin::APP_VERSION.to_string() };
        let (jobs, done) = channel();
        Ok(Manager { model: Model::new(scope), backend: Arc::new(Mutex::new(backend)), jobs, done, generation: store_generation() })
    }

    fn advance(&mut self, queued: Vec<Action>) -> Vec<Effect> {
        let mut actions = queued;
        while let Ok(outcome) = self.done.try_recv() {
            actions.push(Action::Finished(outcome));
        }
        let generation = store_generation();
        if generation != self.generation {
            self.generation = generation;
            actions.push(Action::Refresh);
        }
        actions.into_iter().flat_map(|action| self.model.apply(action)).collect()
    }

    fn spawn(&self, job: Job, ctx: &egui::Context) {
        let backend = Arc::clone(&self.backend);
        let done = self.jobs.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("plugin-store-job".into()).spawn(move || {
            let mut backend = backend.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = done.send(job.execute(&mut *backend));
            ctx.request_repaint();
        });
        if let Err(error) = spawned {
            log::warn!("plugin store job thread: {error}");
        }
    }
}

fn store_generation() -> u64 {
    effectcraft_plugin::plugins().map_or(0, effectcraft_plugin::Plugins::store_generation)
}

#[derive(Default)]
pub(crate) struct PluginStore {
    open: bool,
    manager: Option<Result<Manager, String>>,
    queued: Vec<Action>,
}

impl PluginStore {
    pub(crate) fn open_manager(&mut self) {
        self.open = true;
    }

    pub(crate) fn find(&mut self, query: &str) {
        self.open = true;
        self.queued.extend([Action::SetTab(Tab::Discover), Action::SetQuery(query.to_string())]);
    }

    pub(crate) fn install_file(&mut self, path: PathBuf) {
        self.open = true;
        self.queued.push(Action::InstallFile(path));
    }
}

impl EffectcraftApp {
    pub fn open_plugin_packages(&mut self, paths: &[String]) {
        paths.iter().for_each(|path| self.plugin_store.install_file(PathBuf::from(path)));
    }
}

pub(crate) fn install_packages(app: &mut EffectcraftApp, paths: &[String]) {
    app.open_plugin_packages(paths);
}

pub(crate) fn listed(app: &mut EffectcraftApp) -> Vec<Listed> {
    const LIST_AGE: std::time::Duration = std::time::Duration::from_secs(1);
    if let Some((read, listed)) = &app.plugin_ui.listed
        && read.elapsed() < LIST_AGE
    {
        return listed.clone();
    }
    let listed: Vec<Listed> = app.session.execute("plugin.list", json!({})).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    app.plugin_ui.listed = Some((std::time::Instant::now(), listed.clone()));
    listed
}

fn theme(t: &Tokens) -> Theme {
    Theme {
        text: t.text,
        muted: t.text_dim,
        accent: t.accent,
        warn: t.warning,
        error: t.danger,
        official: t.accent,
        verified: t.cache_green,
        community: t.text_dim,
        local: t.text_faint,
    }
}

fn words(word: Word) -> String {
    tr(word.english()).to_string()
}

pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context) {
    if !app.plugin_store.open {
        return;
    }
    let mut manager = match app.plugin_store.manager.take() {
        Some(manager) => manager,
        None => {
            app.plugin_store.queued.insert(0, Action::Refresh);
            Manager::start()
        }
    };
    let queued = std::mem::take(&mut app.plugin_store.queued);
    let mut effects = manager.as_mut().map(|manager| manager.advance(queued)).unwrap_or_default();
    let t = app.tokens;
    let mut open = true;
    let mut commands: Vec<(&'static str, Value)> = Vec::new();
    egui::Window::new(tr("Plugins"))
        .id(egui::Id::unique("plugin-store"))
        .order(egui::Order::Middle)
        .collapsible(false)
        .open(&mut open)
        .default_size([760.0, 560.0])
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel_bg))
        .show(ctx, |ui| match &mut manager {
            Ok(manager) => {
                tasks(app, ui, &mut commands);
                not_started(app, ui, &mut commands);
                effects.extend(ui::show(&mut manager.model, ui, &theme(&t), &words));
            }
            Err(error) => {
                ui.label(RichText::new(error.as_str()).color(t.danger));
            }
        });
    app.plugin_store.open &= open;
    app.plugin_store.manager = Some(manager);
    commands.into_iter().for_each(|(id, params)| invoke(app, ctx, id, params));
    effects.into_iter().for_each(|effect| carry_out(app, ctx, effect));
}

fn invoke(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str, params: Value) {
    app.plugin_ui.listed = None;
    if let Err(error) = crate::menus::invoke(app, ctx, id, params) {
        app.session.toast(error);
    }
}

fn carry_out(app: &mut EffectcraftApp, ctx: &egui::Context, effect: Effect) {
    match effect {
        Effect::Work(job) => {
            if let Some(Ok(manager)) = &app.plugin_store.manager {
                manager.spawn(job, ctx);
            }
        }
        Effect::OpenSettings { plugin, panel } => invoke(app, ctx, "window.pluginPanel", json!({"plugin": plugin, "panel": panel, "floating": true})),
        Effect::RestartPlugin(plugin) => invoke(app, ctx, "plugin.restart", json!({"id": plugin})),
        Effect::PickFile => invoke(app, ctx, "plugin.installFromFile", json!({})),
        Effect::Notice(text) => app.session.toast(text),
    }
}

fn tasks(app: &mut EffectcraftApp, ui: &mut egui::Ui, commands: &mut Vec<(&'static str, Value)>) {
    let running: Vec<ListedTask> = app.session.execute("plugin.tasks", json!({})).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    if running.is_empty() {
        return;
    }
    let t = app.tokens;
    ui.label(RichText::new(tr("Tasks")).font(Tokens::semibold(14.0)).color(t.text));
    for task in &running {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} · {}", task.plugin, task.task)).color(t.text));
            match task.status {
                TaskStatus::Running => {
                    let bar = egui::ProgressBar::new(task.progress as f32).desired_width(180.0).text(task.message.as_str()).show_percentage();
                    let response = ui.add(bar);
                    app.auto.add_response(&format!("plugins.{}.task.{}.progress", task.plugin, task.id), &response, "Task progress");
                    let cancel = ui.button(tr("Cancel"));
                    app.auto.add_response(&format!("plugins.{}.task.{}.cancel", task.plugin, task.id), &cancel, "Cancel");
                    if cancel.clicked() {
                        commands.push(("plugin.cancelTask", json!({"plugin": task.plugin, "id": task.id})));
                    }
                }
                TaskStatus::Done => {
                    ui.label(RichText::new(tr("Finished")).color(t.text_dim));
                }
                TaskStatus::Cancelled => {
                    ui.label(RichText::new(tr("Cancelled")).color(t.text_dim));
                }
                TaskStatus::Failed => {
                    ui.label(RichText::new(tr_args("Failed: {}", &[&task.error.clone().unwrap_or_default()])).color(t.danger));
                }
            }
        });
    }
    ui.separator();
}

fn not_started(app: &mut EffectcraftApp, ui: &mut egui::Ui, commands: &mut Vec<(&'static str, Value)>) {
    let failed: Vec<Listed> = listed(app).into_iter().filter(|plugin| plugin.state == State::Failed).collect();
    if failed.is_empty() {
        return;
    }
    let t = app.tokens;
    for plugin in &failed {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr_args("{} did not start: {}", &[&plugin.name, &plugin.error.as_deref().unwrap_or_default()])).color(t.danger));
            let restart = ui.button(tr(Word::FixRestart.english()));
            app.auto.add_response(&format!("plugins.{}.restart", plugin.id), &restart, "Restart");
            if restart.clicked() {
                commands.push(("plugin.restart", json!({"id": plugin.id})));
            }
        });
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_opens_the_manager_on_discover_searching_that_plugin() {
        let mut app = EffectcraftApp::new(effectcraft_host::session());
        crate::menus::invoke(&mut app, &egui::Context::default(), "plugin.find", json!({"id": "acme.glow"})).unwrap();
        assert!(app.plugin_store.open);
        assert_eq!(app.plugin_store.queued, vec![Action::SetTab(Tab::Discover), Action::SetQuery("acme.glow".into())]);
    }

    #[test]
    fn installing_from_a_picked_file_hands_it_to_the_manager() {
        let mut app = EffectcraftApp::new(effectcraft_host::session());
        crate::menus::invoke(&mut app, &egui::Context::default(), "plugin.installFromFile", json!({"path": "/tmp/glow.craftplugin"})).unwrap();
        assert_eq!(app.plugin_store.queued, vec![Action::InstallFile(PathBuf::from("/tmp/glow.craftplugin"))]);
    }
}
