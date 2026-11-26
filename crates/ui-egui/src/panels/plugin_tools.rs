use effectcraft_engine::render::EvalCtx;
use effectcraft_plugin::viewer::{Cursor, Mark, Space, ToolEvent, ToolEventKind};
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

pub(crate) fn pick(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str) -> Result<(), String> {
    let known = effectcraft_plugin::viewer::tools();
    let found = id.split_once('/').and_then(|(plugin, tool)| known.iter().find(|(p, spec)| p == plugin && spec.id == tool));
    let Some((plugin, spec)) = found else {
        let names: Vec<String> = known.iter().map(|(plugin, spec)| format!("{plugin}/{}", spec.id)).collect();
        return Err(format!("unknown plugin tool `{id}` (plugin/tool; loaded: {})", names.join(", ")));
    };
    app.ui.plugin_tool = Some(PluginTool { plugin: plugin.clone(), tool: spec.id.clone(), under: app.ui.tool });
    put_away(app, ctx);
    Ok(())
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
            app.ui.plugin_tool = (!on).then_some(PluginTool { plugin, tool: spec.id, under: app.ui.tool });
            put_away(app, ui.ctx());
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

fn put_away(app: &mut EffectcraftApp, ctx: &egui::Context) {
    app.plugin_ui.panels.put_tool_away();
    ctx.data_mut(|d| d.remove::<egui::Id>(pressed_id()));
}

fn pens_id() -> egui::Id {
    egui::Id::unique("viewer-plugin-tool-pens")
}

type PassPens = (u64, Vec<Option<effectcraft_plugin::viewer::Pen>>);

pub(crate) fn pens_of_this_pass(
    source: Option<&crate::PenSource>,
    ctx: &egui::Context,
    events: &[egui::Event],
) -> Vec<Option<effectcraft_plugin::viewer::Pen>> {
    let pass = ctx.cumulative_pass_nr();
    if let Some((paired, pens)) = ctx.data(|d| d.get_temp::<PassPens>(pens_id()))
        && paired == pass
    {
        return pens;
    }
    let pens = source.map_or_else(Vec::new, |source| source.pens(events));
    ctx.data_mut(|d| d.insert_temp(pens_id(), (pass, pens.clone())));
    pens
}

pub(crate) fn viewer(app: &mut EffectcraftApp, ui: &mut egui::Ui, ectx: &EvalCtx, map: &ViewerMap, painter: &egui::Painter) -> bool {
    let (events, held) = ui.ctx().input(|i| (i.events.clone(), i.modifiers));
    let pens = pens_of_this_pass(app.hooks.pen.as_ref(), ui.ctx(), &events);
    let Some(pick) = active(app).cloned() else {
        put_away(app, ui.ctx());
        return false;
    };
    let Some((_, spec)) = effectcraft_plugin::viewer::tools().into_iter().find(|(plugin, spec)| *plugin == pick.plugin && spec.id == pick.tool) else {
        app.ui.plugin_tool = None;
        put_away(app, ui.ctx());
        return false;
    };
    let ctx = ui.ctx().clone();
    let layer = app.session.state.selected_layers.first().and_then(|id| ectx.comp.layer(*id));
    let l2c = layer.map(|layer| crate::panels::viewer::l2c(ectx, layer).0);
    let hover = ctx.input(|i| i.pointer.hover_pos()).filter(|pos| map.area.contains(*pos));
    let me = ui.scope_id();
    let mut owner = ctx.data(|d| d.get_temp::<egui::Id>(pressed_id()));
    let typing = ctx.egui_wants_keyboard_input();
    let double = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
    let last_release =
        events.iter().rposition(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. }));
    for (nr, event) in events.into_iter().enumerate() {
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
                pen: None,
            })
        };
        let primary = egui::PointerButton::Primary;
        let pressed = owner == Some(me);
        let event = match event {
            egui::Event::PointerButton { pos, button, pressed: true, modifiers } if button == primary && map.area.contains(pos) => {
                owner = Some(me);
                found(ToolEventKind::Down, pos, modifiers)
            }
            egui::Event::PointerButton { pos, button, pressed: false, modifiers } if button == primary && pressed => {
                owner = None;
                found(ToolEventKind::Up, pos, modifiers)
            }
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, modifiers } if map.area.contains(pos) => {
                found(ToolEventKind::Context, pos, modifiers)
            }
            egui::Event::PointerMoved(pos) if pressed || (owner.is_none() && map.area.contains(pos)) => found(ToolEventKind::Move, pos, held),
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
        event.pen = pens.get(nr).copied().flatten();
        event.pressure = if owner == Some(me) { event.pen.map_or(1.0, |pen| pen.pressure) } else { 0.0 };
        let opened = (double && event.kind == ToolEventKind::Up && last_release == Some(nr)).then(|| ToolEvent { kind: ToolEventKind::Open, ..event.clone() });
        for event in std::iter::once(event).chain(opened) {
            if let Err(error) = app.plugin_ui.panels.tool(&app.session, &pick.plugin, &pick.tool, event) {
                app.ui.status = error;
            }
        }
    }
    ctx.data_mut(|d| {
        if let Some(owner) = owner {
            d.insert_temp(pressed_id(), owner);
        } else {
            d.remove::<egui::Id>(pressed_id());
        }
    });
    let tokens = app.tokens;
    if let Some(overlay) = app.plugin_ui.panels.tool_overlay(&mut app.session) {
        draw_marks(&tokens, painter, map, &overlay.marks, l2c.as_ref());
    }
    if hover.is_some() {
        ctx.set_cursor_icon(cursor(spec.cursor));
    }
    true
}

#[cfg(test)]
pub(crate) mod tests;
