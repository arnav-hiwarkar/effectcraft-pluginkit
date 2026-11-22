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
    pub version: String,
    pub author: String,
    pub description: String,
    pub sandboxed: bool,
    pub can_unsandbox: bool,
    pub permissions: Vec<Permission>,
    pub state: State,
    pub error: Option<String>,
    pub session_only: bool,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Permission {
    pub name: String,
    pub summary: String,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Running,
    NeedsApproval,
    Disabled,
    Failed,
}

type Actions = Vec<(&'static str, Value)>;

pub(crate) fn install_packages(app: &mut EffectcraftApp, paths: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    for path in paths {
        match app.session.execute("plugin.install", json!({"path": path})) {
            Ok(_) => {
                app.plugin_ui.manager_open = true;
                app.plugin_ui.listed = None;
            }
            Err(error) => errors.push(tr_args("Cannot install plug-in {}: {}", &[path, &error])),
        }
    }
    errors
}

impl EffectcraftApp {
    pub fn open_plugin_packages(&mut self, paths: &[String]) {
        if let Some(error) = install_packages(self, paths).into_iter().next() {
            self.ui.status = error;
        }
    }
}

pub(crate) fn installed(app: &mut EffectcraftApp) -> Vec<Listed> {
    list(app).unwrap_or_default()
}

const LIST_AGE: std::time::Duration = std::time::Duration::from_secs(1);

fn list(app: &mut EffectcraftApp) -> Result<Vec<Listed>, String> {
    if let Some((read, listed)) = &app.plugin_ui.listed
        && read.elapsed() < LIST_AGE
    {
        return listed.clone();
    }
    let listed = app.session.execute("plugin.list", json!({})).map_err(|e| e.to_string()).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()));
    app.plugin_ui.listed = Some((std::time::Instant::now(), listed.clone()));
    listed
}

pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context) {
    if !app.plugin_ui.manager_open {
        return;
    }
    let listed = list(app);
    let t = app.tokens;
    let mut open = true;
    let mut actions = Actions::new();
    egui::Window::new(tr("Plug-ins"))
        .id(egui::Id::unique("plugin-manager"))
        .order(egui::Order::Middle)
        .collapsible(false)
        .open(&mut open)
        .default_size([520.0, 520.0])
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel_bg))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                action(app, ui, "plugins.install", "Install Plug-in...", ("plugin.install", json!({})), &mut actions);
                action(app, ui, "plugins.installFolder", "Install Folder...", ("plugin.install", json!({"folder": true})), &mut actions);
                action(app, ui, "plugins.reload", "Reload", ("plugin.reload", json!({})), &mut actions);
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match &listed {
                Ok(plugins) if plugins.is_empty() => {
                    ui.label(RichText::new(tr("No plug-ins are installed.")).color(t.text_dim));
                }
                Ok(plugins) => plugins.iter().for_each(|plugin| entry(app, ui, plugin, &mut actions)),
                Err(error) => {
                    ui.label(RichText::new(error.as_str()).color(t.danger));
                }
            });
        });
    app.plugin_ui.manager_open &= open;
    for (id, params) in actions {
        app.plugin_ui.listed = None;
        if let Err(error) = crate::menus::invoke(app, ctx, id, params) {
            app.ui.status = error;
        }
    }
}

fn action(app: &mut EffectcraftApp, ui: &mut egui::Ui, auto: &str, label: &'static str, command: (&'static str, Value), actions: &mut Actions) {
    let response = ui.button(tr(label));
    app.auto.add_response(auto, &response, label);
    if response.clicked() {
        actions.push(command);
    }
}

fn entry(app: &mut EffectcraftApp, ui: &mut egui::Ui, plugin: &Listed, actions: &mut Actions) {
    let t = app.tokens;
    egui::Frame::group(ui.style()).fill(t.field_bg).stroke(egui::Stroke::new(1.0, t.field_border)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&plugin.name).font(Tokens::semibold(14.0)).color(t.text));
            ui.label(RichText::new(&plugin.version).color(t.text_dim));
        });
        if !plugin.author.is_empty() {
            ui.label(RichText::new(tr_args("by {}", &[&plugin.author])).color(t.text_dim));
        }
        if !plugin.description.is_empty() {
            ui.label(RichText::new(&plugin.description).color(t.text));
        }
        if plugin.sandboxed {
            ui.label(RichText::new(tr("Sandboxed: it can only do what the permissions below allow.")).color(t.text_dim));
        } else {
            ui.label(
                RichText::new(tr(
                    "⚠ Not sandboxed: this plug-in runs with full access to this computer, including all your files. Allow it only if you trust its author.",
                ))
                .color(t.warning)
                .strong(),
            );
        }
        ui.label(RichText::new(state_text(plugin)).color(if plugin.state == State::Failed { t.danger } else { t.text }));
        permissions(ui, plugin, &t);
        ui.horizontal(|ui| {
            let scope = if plugin.session_only { format!("plugins.{}.session", plugin.id) } else { format!("plugins.{}", plugin.id) };
            let id = json!({"id": plugin.id});
            if matches!(plugin.state, State::NeedsApproval | State::Disabled) {
                action(app, ui, &format!("{scope}.approve"), "Allow and Enable", ("plugin.approve", id.clone()), actions);
            }
            if plugin.state == State::Running {
                action(app, ui, &format!("{scope}.disable"), "Disable", ("plugin.disable", id.clone()), actions);
            }
            if plugin.can_unsandbox && plugin.state != State::NeedsApproval {
                let (auto, label, sandboxed) = if plugin.sandboxed { ("unsandbox", "Run Unsandboxed", false) } else { ("sandbox", "Run Sandboxed", true) };
                action(app, ui, &format!("{scope}.{auto}"), label, ("plugin.setSandbox", json!({"id": plugin.id, "sandboxed": sandboxed})), actions);
            }
            if !plugin.session_only {
                action(app, ui, &format!("{scope}.remove"), "Remove", ("plugin.uninstall", id), actions);
            }
        });
    });
    ui.add_space(6.0);
}

fn state_text(plugin: &Listed) -> String {
    let state = match plugin.state {
        State::Running => tr("Running").to_string(),
        State::NeedsApproval => tr("Needs your approval before it can run").to_string(),
        State::Disabled => tr("Disabled").to_string(),
        State::Failed => tr_args("Failed: {}", &[&plugin.error.as_deref().unwrap_or_default()]),
    };
    if plugin.session_only { tr_args("{} (loaded for this session only)", &[&state]) } else { state }
}

fn permissions(ui: &mut egui::Ui, plugin: &Listed, t: &Tokens) {
    if plugin.permissions.is_empty() {
        ui.label(RichText::new(tr("It asks for no permissions.")).color(t.text_dim));
        return;
    }
    ui.label(RichText::new(tr("It asks to:")).color(t.text));
    for permission in &plugin.permissions {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("• {}", permission.summary)).color(t.text));
            ui.label(RichText::new(&permission.name).font(Tokens::mono(11.0)).color(t.text_dim));
        });
    }
}
