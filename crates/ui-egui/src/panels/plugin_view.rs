use std::collections::HashMap;

use effectcraft_engine::PluginItem;
use effectcraft_plugin::ui::{Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, UiInput, Widget};
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, vec2};
use serde_json::json;

use crate::EffectcraftApp;
use crate::automation::Registry;
use crate::dock::{PanelKind, plugin_key};
use crate::i18n::{tr, tr_args};
use crate::theme::Tokens;

const MAX_EXTENT: f32 = 16_384.0;
const MAX_FONT: f32 = 512.0;

#[derive(Default)]
pub(crate) struct PluginUi {
    pub events: HashMap<u32, Vec<UiEvent>>,
    pub manager_open: bool,
}

pub fn running_panel(app: &EffectcraftApp, kind: PanelKind) -> Option<&PluginItem> {
    app.session.plugin_items.iter().find(|item| item.panel && PanelKind::plugin(&item.plugin, &item.id) == kind)
}

pub fn panel(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let Some(item) = running_panel(app, kind).cloned() else { return not_running(app, ui, rect, kind) };
    let PanelKind::Plugin(key) = kind else { return };
    let scope = format!("plugin.{}.{}", item.plugin, item.id);
    app.auto.add(&scope, rect, &item.title);
    let events = app.plugin_ui.events.remove(&key).unwrap_or_default();
    let input = UiInput { events, size: [rect.width(), rect.height()] };
    let tokens = app.tokens;
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    match effectcraft_plugin::panel_ui(&mut app.session, &item.plugin, &item.id, input) {
        Ok(widgets) => {
            let scroll = egui::ScrollArea::both().id_salt(("plugin-panel", key)).auto_shrink([false, false]);
            let events = scroll.show(&mut body, |ui| draw(ui, &widgets, &tokens, &mut app.auto, &scope)).inner;
            if !events.is_empty() {
                app.plugin_ui.events.insert(key, events);
                ui.ctx().request_repaint();
            }
        }
        Err(error) => {
            body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

fn not_running(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let owner = kind.plugin_owner().and_then(|key| super::plugin_manager::installed(app).into_iter().find(|p| plugin_key(&p.id) == key));
    let message = match owner {
        Some(plugin) => tr_args("{} is not running. Turn it on in Plug-ins ▸ Manage Plug-ins.", &[&plugin.name]),
        None => tr("The plug-in that provides this panel is not installed. Install it in Plug-ins ▸ Manage Plug-ins.").to_string(),
    };
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(12.0)));
    body.label(RichText::new(message).color(app.tokens.text_dim));
    let manage = body.button(tr("Manage Plug-ins..."));
    app.auto.add_response(&format!("{}.manage", kind.id()), &manage, "Manage Plug-ins...");
    if manage.clicked() {
        let ctx = ui.ctx().clone();
        if let Err(error) = crate::menus::invoke(app, &ctx, "window.plugins", json!({})) {
            app.ui.status = error;
        }
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, widgets: &[Widget], tokens: &Tokens, auto: &mut Registry, scope: &str) -> Vec<UiEvent> {
    let mut renderer = Renderer { tokens, auto, scope, events: Vec::new() };
    renderer.all(ui, widgets);
    renderer.events
}

struct Renderer<'a> {
    tokens: &'a Tokens,
    auto: &'a mut Registry,
    scope: &'a str,
    events: Vec<UiEvent>,
}

fn salt(widget: &Widget, index: usize) -> u64 {
    match widget {
        Widget::Button { id, .. }
        | Widget::Checkbox { id, .. }
        | Widget::Slider { id, .. }
        | Widget::Number { id, .. }
        | Widget::TextEdit { id, .. }
        | Widget::Popup { id, .. }
        | Widget::Color { id, .. }
        | Widget::Canvas { id, .. } => *id,
        _ => index as u64,
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

fn extent(value: f32) -> f32 {
    value.clamp(0.0, MAX_EXTENT)
}

fn font_size(size: f32) -> f32 {
    if size.is_finite() { size.clamp(1.0, MAX_FONT) } else { 12.0 }
}

impl Renderer<'_> {
    fn all(&mut self, ui: &mut egui::Ui, widgets: &[Widget]) {
        for (index, widget) in widgets.iter().enumerate() {
            ui.push_id(salt(widget, index), |ui| self.widget(ui, widget));
        }
    }

    fn emit(&mut self, id: u64, kind: UiEventKind) {
        self.events.push(UiEvent { id, kind });
    }

    fn register(&mut self, id: u64, response: &egui::Response, label: &str) {
        self.auto.add_response(&format!("{}.{id}", self.scope), response, label);
    }

    fn paint(&self, paint: Paint) -> Color32 {
        let t = self.tokens;
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

    fn text(&self, text: &str, style: TextStyle) -> RichText {
        let t = self.tokens;
        match style {
            TextStyle::Body => RichText::new(text).font(Tokens::ui(12.0)).color(t.text),
            TextStyle::Heading => RichText::new(text).font(Tokens::semibold(15.0)).color(t.text),
            TextStyle::Weak => RichText::new(text).font(Tokens::ui(12.0)).color(t.text_dim),
            TextStyle::Mono => RichText::new(text).font(Tokens::mono(12.0)).color(t.text),
        }
    }

    fn widget(&mut self, ui: &mut egui::Ui, widget: &Widget) {
        match widget {
            Widget::Label { text, style } => {
                ui.label(self.text(text, *style));
            }
            Widget::Button { id, text, enabled } => {
                let response = ui.add_enabled(*enabled, egui::Button::new(text));
                self.register(*id, &response, text);
                if response.clicked() {
                    self.emit(*id, UiEventKind::Click);
                }
            }
            Widget::Checkbox { id, text, value } => {
                let mut on = *value;
                let response = ui.checkbox(&mut on, text);
                self.register(*id, &response, text);
                if response.changed() {
                    self.emit(*id, UiEventKind::Bool(on));
                }
            }
            Widget::Slider { id, text, value, min, max } => {
                let mut number = *value;
                let range = finite_or(*min, 0.0)..=finite_or(*max, 1.0);
                let response = ui.add(egui::Slider::new(&mut number, range).text(text));
                self.register(*id, &response, text);
                if response.changed() {
                    self.emit(*id, UiEventKind::Number(number));
                }
            }
            Widget::Number { id, text, value, speed } => {
                let mut number = *value;
                let response = ui
                    .horizontal(|ui| {
                        let label = ui.label(self.text(text, TextStyle::Body));
                        ui.add(egui::DragValue::new(&mut number).speed(finite_or(*speed, 1.0))).labelled_by(label.id)
                    })
                    .inner;
                self.register(*id, &response, text);
                if response.changed() {
                    self.emit(*id, UiEventKind::Number(number));
                }
            }
            Widget::TextEdit { id, value, hint } => {
                let mut text = value.clone();
                let response = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint.as_str()));
                self.register(*id, &response, hint);
                if response.changed() {
                    self.emit(*id, UiEventKind::Text(text));
                }
            }
            Widget::Popup { id, text, options, selected } => self.popup(ui, *id, text, options, *selected as usize),
            Widget::Color { id, text, value } => {
                let mut color = *value;
                let response = ui
                    .horizontal(|ui| {
                        let label = ui.label(self.text(text, TextStyle::Body));
                        ui.color_edit_button_rgba_unmultiplied(&mut color).labelled_by(label.id)
                    })
                    .inner;
                self.register(*id, &response, text);
                if response.changed() {
                    self.emit(*id, UiEventKind::Color(color));
                }
            }
            Widget::Row { children } => {
                ui.horizontal(|ui| self.all(ui, children));
            }
            Widget::Column { children } => {
                ui.vertical(|ui| self.all(ui, children));
            }
            Widget::Group { title, children } => {
                egui::CollapsingHeader::new(title.as_str()).default_open(true).show(ui, |ui| self.all(ui, children));
            }
            Widget::Separator => {
                ui.separator();
            }
            Widget::Space { size } => ui.add_space(extent(*size)),
            Widget::Canvas { id, size, shapes } => self.canvas(ui, *id, *size, shapes),
        }
    }

    fn popup(&mut self, ui: &mut egui::Ui, id: u64, text: &str, options: &[String], selected: usize) {
        let mut chosen = None;
        let shown = options.get(selected).cloned().unwrap_or_default();
        let combo = craft_ui::ComboBox::new(id, text).selected_text(shown).show_ui(ui, |ui| {
            for (index, option) in options.iter().enumerate() {
                let item = ui.selectable_label(index == selected, option);
                self.auto.add_response(&format!("{}.{id}.{index}", self.scope), &item, option);
                if item.clicked() {
                    chosen = Some(index);
                }
            }
        });
        self.register(id, &combo.response, text);
        if let Some(index) = chosen.filter(|index| *index != selected) {
            self.emit(id, UiEventKind::Number(index as f64));
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, id: u64, size: [f32; 2], shapes: &[Shape]) {
        let (rect, response) = ui.allocate_exact_size(vec2(extent(size[0]), extent(size[1])), Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, tr("Plug-in drawing")));
        self.register(id, &response, "Canvas");
        let painter = ui.painter_at(rect);
        for shape in shapes {
            self.shape(&painter, rect.min, shape);
        }
        if let Some(pointer) = pointer(ui, &response, rect.min) {
            self.emit(id, UiEventKind::Pointer(pointer));
        }
    }

    fn shape(&self, painter: &egui::Painter, origin: egui::Pos2, shape: &Shape) {
        let at = |p: [f32; 2]| origin + vec2(p[0], p[1]);
        let stroke = |paint: Paint, width: f32| Stroke::new(width, self.paint(paint));
        match shape {
            Shape::Line { points, color, width } => {
                painter.add(egui::Shape::line(points.iter().copied().map(at).collect(), stroke(*color, *width)));
            }
            Shape::Bezier { points, color, width } => {
                painter.add(CubicBezierShape::from_points_stroke(points.map(at), false, Color32::TRANSPARENT, stroke(*color, *width)));
            }
            Shape::Rect { min, max, fill, stroke: outline, radius } => {
                let rect = Rect::from_two_pos(at(*min), at(*max));
                if let Some(fill) = fill {
                    painter.rect_filled(rect, *radius, self.paint(*fill));
                }
                if let Some((paint, width)) = outline {
                    painter.rect_stroke(rect, *radius, stroke(*paint, *width), StrokeKind::Inside);
                }
            }
            Shape::Circle { center, radius, fill, stroke: outline } => {
                let fill = fill.map_or(Color32::TRANSPARENT, |paint| self.paint(paint));
                let outline = outline.map_or(Stroke::NONE, |(paint, width)| stroke(paint, width));
                painter.circle(at(*center), *radius, fill, outline);
            }
            Shape::Text { pos, text, color, size } => {
                painter.text(at(*pos), Align2::LEFT_TOP, text, Tokens::ui(font_size(*size)), self.paint(*color));
            }
        }
    }
}

fn pointer(ui: &egui::Ui, response: &egui::Response, origin: egui::Pos2) -> Option<Pointer> {
    let (pressed, moving, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.is_moving(), i.pointer.interact_pos()));
    let phase = if pressed && response.contains_pointer() {
        PointerPhase::Down
    } else if response.clicked() || response.drag_stopped() {
        PointerPhase::Up
    } else if moving && response.is_pointer_button_down_on() {
        PointerPhase::Drag
    } else if moving && response.hovered() {
        PointerPhase::Hover
    } else {
        return None;
    };
    let local = at? - origin;
    Some(Pointer { pos: [local.x, local.y], phase })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn harness(widgets: Vec<Widget>) -> Harness<'static, Vec<UiEvent>> {
        let tokens = Tokens::for_kind(Default::default());
        Harness::new_ui_state(
            move |ui, events: &mut Vec<UiEvent>| {
                let mut auto = Registry::default();
                events.extend(draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel"));
            },
            Vec::new(),
        )
    }

    #[test]
    fn clicking_a_button_sends_a_click_with_its_id() {
        let mut harness = harness(vec![Widget::Button { id: 42, text: "Count".into(), enabled: true }]);
        harness.get_by_label("Count").click();
        harness.run();
        assert_eq!(harness.state(), &vec![UiEvent { id: 42, kind: UiEventKind::Click }]);
    }

    #[test]
    fn dragging_a_slider_sends_numbers_with_its_id_ending_at_the_dragged_value() {
        let mut harness = harness(vec![Widget::Slider { id: 7, text: "Speed".into(), value: 0.0, min: 0.0, max: 10.0 }]);
        harness.run();
        let rail = harness.get_by_role(egui::Role::Slider).rect();
        harness.drag_at(rail.left_center());
        harness.run();
        harness.hover_at(rail.right_center() + vec2(40.0, 0.0));
        harness.run();
        harness.drop_at(rail.right_center() + vec2(40.0, 0.0));
        harness.run();
        let events = harness.state();
        assert!(!events.is_empty());
        assert!(events.iter().all(|event| event.id == 7 && matches!(event.kind, UiEventKind::Number(_))), "{events:?}");
        assert_eq!(events.last().map(|event| &event.kind), Some(&UiEventKind::Number(10.0)));
    }
}
