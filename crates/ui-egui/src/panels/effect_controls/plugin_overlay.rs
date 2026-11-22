use std::sync::Arc;

use effectcraft_plugin::ui::{Paint, Shape};
use effectcraft_plugin::viewer::{Bind, Mark, Shown, Space};

use super::common::{Actions, gesture_key};
use super::{Color32, EffectcraftApp, EvalCtx, Layer, Mat3, PropGroup, Rect, Sense, Stroke, Tokens, fw, vec2};
use crate::panels::viewer::ViewerMap;

const CURVE_STEPS: usize = 48;
const HANDLE_REACH: f32 = 16.0;

fn colour(t: &Tokens, paint: Paint) -> Color32 {
    match paint {
        Paint::Text => t.text,
        Paint::Weak => t.text_dim,
        Paint::Accent => t.accent,
        Paint::Background => t.panel_bg,
        Paint::Surface => t.field_bg,
        Paint::Border => t.field_border,
        Paint::Error => t.danger,
        Paint::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
    }
}

pub(crate) fn draw_marks(t: &Tokens, painter: &egui::Painter, map: &ViewerMap, marks: &[Mark], layer: Option<&Mat3>) {
    for mark in marks {
        let to_screen = |p: [f32; 2]| -> Option<egui::Pos2> {
            let p = [f64::from(p[0]), f64::from(p[1])];
            let comp = match mark.space {
                Space::Comp => p,
                Space::Layer => fw::layer_to_comp(layer?, p),
            };
            let s = map.to_screen(comp);
            s.is_finite().then_some(s)
        };
        let path = |points: Vec<[f32; 2]>| points.into_iter().map(to_screen).collect::<Option<Vec<_>>>();
        let outline = |points: Vec<[f32; 2]>, fill: Option<Paint>, stroke: Option<(Paint, f32)>| {
            let Some(points) = path(points) else { return };
            let fill = fill.map_or(Color32::TRANSPARENT, |paint| colour(t, paint));
            let stroke = stroke.map_or(Stroke::NONE, |(paint, width)| Stroke::new(width.clamp(0.0, 16.0), colour(t, paint)));
            painter.add(egui::Shape::Path(egui::epaint::PathShape { points, closed: true, fill, stroke: stroke.into() }));
        };
        match &mark.shape {
            Shape::Line { points, color, width } => {
                if let Some(points) = path(points.clone()) {
                    painter.add(egui::Shape::line(points, Stroke::new(width.clamp(0.0, 16.0), colour(t, *color))));
                }
            }
            Shape::Bezier { points: [a, b, c, d], color, width } => {
                let at = |s: f32| {
                    let r = 1.0 - s;
                    std::array::from_fn(|i| r * r * r * a[i] + 3.0 * r * r * s * b[i] + 3.0 * r * s * s * c[i] + s * s * s * d[i])
                };
                if let Some(points) = path((0..=CURVE_STEPS).map(|i| at(i as f32 / CURVE_STEPS as f32)).collect()) {
                    painter.add(egui::Shape::line(points, Stroke::new(width.clamp(0.0, 16.0), colour(t, *color))));
                }
            }
            Shape::Rect { min, max, fill, stroke, .. } => outline(vec![*min, [max[0], min[1]], *max, [min[0], max[1]]], *fill, *stroke),
            Shape::Circle { center, radius, fill, stroke } => {
                let around = (0..CURVE_STEPS).map(|i| {
                    let a = i as f32 / CURVE_STEPS as f32 * std::f32::consts::TAU;
                    [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
                });
                outline(around.collect(), *fill, *stroke)
            }
            Shape::Polygon { points, fill } => {
                if let Some(points) = path(points.clone()) {
                    painter.add(crate::panels::plugin_view::filled_polygon(points, colour(t, *fill)));
                }
            }
            Shape::Text { pos, text, color, size } => {
                if let Some(at) = to_screen(*pos) {
                    painter.text(at, egui::Align2::LEFT_TOP, text, Tokens::ui(size.clamp(6.0, 48.0)), colour(t, *color));
                }
            }
            Shape::Image { .. } => {}
        }
    }
}

struct Grab {
    shown: Arc<Shown>,
    from: [f64; 2],
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
    let shown = effectcraft_plugin::viewer::overlay(ectx, layer, g)?;
    let t = app.tokens;
    let (painter, fm) = (ui.painter_at(map.area), &crate::panels::viewer::fx2c(ectx, layer));
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
        if resp.drag_started()
            && let Some(from) = ui.input(|i| i.pointer.press_origin()).and_then(pointer_in_layer)
        {
            resp.request_focus();
            ui.data_mut(|d| d.insert_temp(id, Arc::new(Grab { shown: shown.clone(), from })));
        }
        let grab = ui.data(|d| d.get_temp::<Arc<Grab>>(id));
        if resp.dragged()
            && let Some(grab) = grab
            && let Some(now) = resp.interact_pointer_pos().and_then(pointer_in_layer)
        {
            let key = gesture_key(ui, id, resp.drag_started());
            push(app, actions, grab.shown.drag_commands(index, [now[0] - grab.from[0], now[1] - grab.from[1]], layer, g, &key));
        }
        if resp.drag_stopped() {
            ui.data_mut(|d| d.remove::<Arc<Grab>>(id));
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
                push(app, actions, shown.drag_commands(index, nudge, layer, g, &key));
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
