use std::sync::Arc;

use effectcraft_plugin::viewer::{Bind, Mark, Shown, Space};

use super::common::{Actions, gesture_key};
use super::{Color32, EffectcraftApp, EvalCtx, Layer, Mat3, PropGroup, Rect, Sense, Stroke, Tokens, fw, vec2};
use crate::panels::viewer::ViewerMap;

const HANDLE_REACH: f32 = 16.0;

pub(crate) fn draw_marks(t: &Tokens, painter: &egui::Painter, map: &ViewerMap, marks: &[Mark], layer: Option<&Mat3>) {
    let theme = crate::panels::plugin_view::theme(t);
    craft_plugin_ui::draw_marks(&theme, painter, marks, crate::panels::plugin_view::triangulate, |space, p| {
        let comp = match space {
            Space::Comp => p,
            Space::Layer => fw::layer_to_comp(layer?, p),
        };
        Some(map.to_screen(comp))
    });
}

#[derive(Clone)]
struct Grab {
    shown: Arc<Shown>,
    from: [f64; 2],
    pass: u64,
}

fn focused_handle_key() -> egui::Id {
    egui::Id::unique("plugin-handle-focused")
}

pub(crate) fn handle_has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused()).is_some_and(|focused| ctx.data(|d| d.get_temp::<egui::Id>(focused_handle_key())) == Some(focused))
}

pub(super) fn effect_overlay(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    map: &ViewerMap,
    ectx: &EvalCtx,
    layer: &Layer,
    g: &PropGroup,
    actions: &mut Actions,
) -> Option<Arc<Shown>> {
    let shown = app.plugin_ui.panels.overlay(&mut app.session, ectx, layer, g)?;
    let t = app.tokens;
    let fm = &crate::panels::viewer::fx2c(ectx, layer);
    let painter = ui.painter_at(map.area);
    draw_marks(&t, &painter, map, &shown.overlay.marks, Some(fm));
    for (index, handle) in shown.overlay.handles.iter().enumerate() {
        let s = map.to_screen(fw::layer_to_comp(fm, handle.at));
        if !s.is_finite() {
            continue;
        }
        let rect = Rect::from_center_size(s, vec2(HANDLE_REACH, HANDLE_REACH));
        let id = egui::Id::unique(("viewer-plugin-handle", layer.id.0, g.uid, index));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        let param = handle.bind.params()[0];
        let name = match handle.name.as_str() {
            "" => g.prop(param).map_or(param, |prop| prop.name.as_str()),
            named => named,
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, name));
        app.auto.add_response(&format!("viewer.pluginHandle.{}.{index}", g.uid), &resp, name);
        let lit = resp.hovered() || resp.dragged() || resp.has_focus();
        let col = if lit { t.accent } else { Color32::WHITE };
        match handle.bind {
            Bind::Point { .. } => {
                fw::draw_crosshair(&painter, s + vec2(1.0, 1.0), 5.0, Color32::from_black_alpha(160));
                fw::draw_crosshair(&painter, s, 5.0, col);
            }
            _ => {
                let square = Rect::from_center_size(s, vec2(7.0, 7.0));
                painter.rect_filled(square, 1.0, col);
                painter.rect_stroke(square, 1.0, Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
            }
        }
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(focused_handle_key(), id));
            painter.circle_stroke(s, HANDLE_REACH * 0.6, Stroke::new(1.5, t.accent));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
            resp.clone().on_hover_text(name);
        }
        if resp.clicked() {
            resp.request_focus();
        }
        let pointer_in_layer = |pos: egui::Pos2| fw::comp_to_layer(fm, map.to_comp(pos));
        let pass = ui.ctx().cumulative_pass_nr();
        if resp.drag_started()
            && let Some(from) = ui.input(|i| i.pointer.press_origin()).and_then(pointer_in_layer)
        {
            resp.request_focus();
            ui.data_mut(|d| d.insert_temp(id, Grab { shown: shown.clone(), from, pass }));
        }
        let grab = ui
            .data(|d| d.get_temp::<Grab>(id))
            .filter(|grab| (resp.dragged() || resp.drag_stopped()) && grab.pass + 1 >= pass && shown.continues(&grab.shown, index));
        match grab {
            Some(grab) => {
                ui.data_mut(|d| d.insert_temp(id, Grab { pass, ..grab.clone() }));
                if let Some(now) = resp.interact_pointer_pos().and_then(pointer_in_layer) {
                    let key = gesture_key(ui, id, resp.drag_started());
                    let delta = [now[0] - grab.from[0], now[1] - grab.from[1]];
                    let commands = app.plugin_ui.panels.drag(&grab.shown, index, delta, g, &key);
                    push(app, actions, commands);
                }
            }
            None => ui.data_mut(|d| d.remove::<Grab>(id)),
        }
        if resp.has_focus() && ui.memory(|m| m.had_focus_last_frame(id)) {
            let arrows = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(id, arrows));
            let nudges: Vec<[f64; 2]> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Key { key, pressed: true, modifiers, .. } => {
                            let step = if modifiers.shift { 10.0 } else { 1.0 };
                            match key {
                                egui::Key::ArrowLeft => Some([-step, 0.0]),
                                egui::Key::ArrowRight => Some([step, 0.0]),
                                egui::Key::ArrowUp => Some([0.0, -step]),
                                egui::Key::ArrowDown => Some([0.0, step]),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .collect()
            });
            for nudge in nudges {
                let key = gesture_key(ui, id.with("nudge"), true);
                let commands = app.plugin_ui.panels.drag(&shown, index, nudge, g, &key);
                push(app, actions, commands);
            }
        }
    }
    Some(shown)
}

fn push(app: &mut EffectcraftApp, actions: &mut Actions, commands: Result<Actions, String>) {
    match commands {
        Ok(commands) => actions.extend(commands),
        Err(error) => app.ui.status = error,
    }
}

#[cfg(test)]
mod tests {
    use effectcraft_plugin::ui::{Paint, Shape};
    use effectcraft_plugin::viewer::{Mark, Space};
    use egui::epaint::Primitive;

    use super::draw_marks;
    use crate::panels::viewer::ViewerMap;
    use crate::theme::Tokens;

    #[test]
    fn paths_with_repeated_points_draw_no_vertex_that_is_not_a_number() {
        let line = |points: Vec<[f32; 2]>| Mark { shape: Shape::Line { points, color: Paint::Accent, width: 2.0 }, space: Space::Comp };
        let marks = vec![
            line(vec![[5.0, 5.0]; 3]),
            line(vec![[1.0, 1.0], [1.0, 1.0], [9.0, 9.0], [9.0, 9.0]]),
            Mark { shape: Shape::Polygon { points: vec![[2.0, 2.0], [2.0, 2.0], [8.0, 2.0]], fill: Paint::Accent }, space: Space::Comp },
        ];
        let tokens = Tokens::for_kind(Default::default());
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        let map = ViewerMap { origin: egui::Pos2::ZERO, zoom: 4.0, comp: [25.0; 2], area };
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw_marks(&tokens, ui.painter(), &map, &marks, None));
        output.textures_delta.clear();
        let vertices: Vec<egui::epaint::Vertex> = ctx
            .tessellate(output.shapes, output.pixels_per_point)
            .into_iter()
            .flat_map(|clipped| match clipped.primitive {
                Primitive::Mesh(mesh) => mesh.vertices,
                Primitive::Callback(_) => Vec::new(),
            })
            .collect();
        let broken = vertices.iter().filter(|vertex| !(vertex.pos.x.is_finite() && vertex.pos.y.is_finite())).count();
        assert_eq!((broken, vertices.is_empty()), (0, false));
    }
}
