use effectcraft_engine::{PluginContext, PluginItem};
use egui::{Rect, Sense, pos2, vec2};
use serde_json::Value;

use crate::EffectcraftApp;
use crate::automation::Registry;

fn params(item: &PluginItem) -> Value {
    effectcraft_engine::shortcuts::plugin_command_params(&item.plugin, &item.id)
}

pub(crate) fn context_section(ui: &mut egui::Ui, items: &[PluginItem], context: PluginContext, auto: &mut Registry, prefix: &str) -> Option<Value> {
    let mut chosen = None;
    let mut placed = items.iter().filter(|item| !item.panel && item.context.contains(&context)).peekable();
    if placed.peek().is_some() {
        ui.separator();
    }
    for item in placed {
        let response = ui.button(&item.title);
        auto.add_response(&format!("{prefix}.plugin.{}.{}", item.plugin, item.id), &response, &item.title);
        if response.clicked() {
            chosen = Some(params(item));
            ui.close();
        }
    }
    chosen
}

pub(crate) fn toolbar(app: &mut EffectcraftApp, ui: &mut egui::Ui, mut x: f32, cy: f32) -> f32 {
    let t = app.tokens;
    let mut chosen = None;
    for item in app.session.plugin_items.iter().filter(|item| !item.panel && item.toolbar) {
        let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(26.0, 26.0));
        let response = ui.interact(rect, egui::Id::unique(("plugin-command", &item.plugin, &item.id)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &item.title));
        if response.hovered() {
            ui.painter().rect_filled(rect, 4.0, t.hover);
        }
        let icon = Rect::from_center_size(rect.center(), vec2(16.0, 16.0));
        match item.icon.as_deref().and_then(crate::panels::plugin_view::named_icon) {
            Some(named) => crate::icons::paint(ui.painter(), icon, named, t.text),
            None => {
                let letter: String = item.title.chars().take(1).flat_map(char::to_uppercase).collect();
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(13.0), t.text);
            }
        }
        crate::widgets::focus_outline(ui, &response, &t);
        app.auto.add_response(&format!("tools.command.{}.{}", item.plugin, item.id), &response, &item.title);
        if response.on_hover_text(&item.title).clicked() {
            chosen = Some(params(item));
        }
        x += 28.0;
    }
    if let Some(params) = chosen {
        let ctx = ui.ctx().clone();
        if let Err(error) = super::invoke(app, &ctx, "plugin.run", params) {
            app.ui.status = error;
        }
    }
    x
}
