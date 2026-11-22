use effectcraft_engine::render::EvalCtx;
use effectcraft_plugin::viewer::{Cursor, Mark, Overlay, Space, ToolEvent, ToolEventKind};
use egui::{Pos2, Rect, Sense, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::EffectcraftApp;
use crate::panels::effect_controls::draw_marks;
use crate::panels::viewer::ViewerMap;
use crate::state::Tool;

const ICON_SIDE: f32 = 16.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginTool {
    pub plugin: String,
    pub tool: String,
    pub under: Tool,
}

pub(crate) fn active(app: &EffectcraftApp) -> Option<&PluginTool> {
    app.ui.plugin_tool.as_ref().filter(|pick| pick.under == app.ui.tool)
}

fn overlay_id() -> egui::Id {
    egui::Id::unique("viewer-plugin-tool-overlay")
}

fn pressed_id() -> egui::Id {
    egui::Id::unique("viewer-plugin-tool-pressed")
}

pub(crate) fn toolbar(app: &mut EffectcraftApp, ui: &mut egui::Ui, mut x: f32, cy: f32) -> f32 {
    let t = app.tokens;
    for (plugin, spec) in effectcraft_plugin::viewer::tools() {
        let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(26.0, 26.0));
        let on = active(app).is_some_and(|pick| pick.plugin == plugin && pick.tool == spec.id);
        let resp = ui.interact(rect, egui::Id::unique(("plugin-tool", &plugin, &spec.id)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &spec.name));
        if on {
            ui.painter().rect_filled(rect, 4.0, t.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, t.hover);
        }
        let origin = rect.center() - vec2(ICON_SIDE / 2.0, ICON_SIDE / 2.0);
        let icon: Vec<Mark> = spec.icon.iter().cloned().map(|shape| Mark { shape, space: Space::Comp }).collect();
        draw_marks(&t, &ui.painter().with_clip_rect(rect), &ViewerMap { origin, zoom: 1.0, comp: [ICON_SIDE; 2], area: rect }, &icon, None);
        crate::widgets::focus_outline(ui, &resp, &t);
        app.auto.add_response(&format!("tools.plugin.{plugin}.{}", spec.id), &resp, &spec.name);
        if resp.on_hover_text(&spec.name).clicked() {
            app.ui.plugin_tool = (!on).then(|| PluginTool { plugin, tool: spec.id, under: app.ui.tool });
            ui.data_mut(|d| d.remove::<Overlay>(overlay_id()));
        }
        x += 28.0;
    }
    x
}

fn cursor(cursor: Cursor) -> egui::CursorIcon {
    match cursor {
        Cursor::Crosshair => egui::CursorIcon::Crosshair,
        Cursor::Arrow => egui::CursorIcon::Default,
        Cursor::Move => egui::CursorIcon::Move,
        Cursor::Grab => egui::CursorIcon::Grab,
        Cursor::Text => egui::CursorIcon::Text,
    }
}

pub(crate) fn viewer(app: &mut EffectcraftApp, ui: &mut egui::Ui, ectx: &EvalCtx, map: &ViewerMap, painter: &egui::Painter) -> bool {
    let Some(pick) = active(app).cloned() else { return false };
    let ctx = ui.ctx().clone();
    let layer = app.session.state.selected_layers.first().and_then(|id| ectx.comp.layer(*id));
    let l2c = layer.map(|layer| crate::panels::viewer::l2c(ectx, layer).0);
    let hover = ctx.input(|i| i.pointer.hover_pos()).filter(|pos| map.area.contains(*pos));
    let mut pressed = ctx.data(|d| d.get_temp::<bool>(pressed_id())).unwrap_or(false);
    let (events, held) = ctx.input(|i| (i.events.clone(), i.modifiers));
    let typing = ctx.egui_wants_keyboard_input();
    for event in events {
        let found = |kind: ToolEventKind, pos: Pos2, modifiers: egui::Modifiers| {
            let comp = map.to_comp(pos);
            let modifiers = effectcraft_plugin::ui::Modifiers { shift: modifiers.shift, alt: modifiers.alt, command: modifiers.command };
            Some(ToolEvent {
                kind,
                comp,
                layer: l2c.as_ref().and_then(|m| super::fx_widgets::comp_to_layer(m, comp)),
                layer_id: layer.map(|layer| layer.id.0),
                modifiers,
                pressure: 0.0,
                scroll: [0.0; 2],
                key: String::new(),
                zoom: f64::from(map.zoom),
                time: ectx.time.seconds(),
            })
        };
        let primary = egui::PointerButton::Primary;
        let event = match event {
            egui::Event::PointerButton { pos, button, pressed: true, modifiers } if button == primary && map.area.contains(pos) => {
                pressed = true;
                found(ToolEventKind::Down, pos, modifiers)
            }
            egui::Event::PointerButton { pos, button, pressed: false, modifiers } if button == primary && pressed => {
                pressed = false;
                found(ToolEventKind::Up, pos, modifiers)
            }
            egui::Event::PointerMoved(pos) if pressed || map.area.contains(pos) => found(ToolEventKind::Move, pos, held),
            egui::Event::MouseWheel { delta, modifiers, .. } => hover.and_then(|pos| found(ToolEventKind::Scroll, pos, modifiers)).map(|mut event| {
                event.scroll = [delta.x, delta.y];
                event
            }),
            egui::Event::Key { key, pressed: true, modifiers, .. } if !typing => {
                hover.and_then(|pos| found(ToolEventKind::Key, pos, modifiers)).map(|mut event| {
                    event.key = format!("{key:?}");
                    event
                })
            }
            _ => None,
        };
        let Some(mut event) = event else { continue };
        event.pressure = f32::from(u8::from(pressed));
        match effectcraft_plugin::viewer::tool_event(&mut app.session, &pick.plugin, &pick.tool, &event) {
            Ok(overlay) => {
                ctx.data_mut(|d| d.insert_temp(overlay_id(), overlay));
            }
            Err(error) => app.ui.status = error,
        }
    }
    ctx.data_mut(|d| d.insert_temp(pressed_id(), pressed));
    if let Some(overlay) = ctx.data(|d| d.get_temp::<Overlay>(overlay_id())) {
        draw_marks(&app.tokens, painter, map, &overlay.marks, l2c.as_ref());
    }
    if hover.is_some() {
        let spec = effectcraft_plugin::viewer::tools().into_iter().find(|(plugin, spec)| *plugin == pick.plugin && spec.id == pick.tool);
        ctx.set_cursor_icon(cursor(spec.map_or(Cursor::Crosshair, |(_, spec)| spec.cursor)));
    }
    true
}

#[cfg(test)]
mod tests;
