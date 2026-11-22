use std::collections::HashMap;

use effectcraft_engine::PluginItem;
use effectcraft_plugin::ui::{Modifiers, Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, UiInput, Widget};
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
const MAX_LINE_POINTS: usize = 65_536;

#[derive(Default)]
pub(crate) struct PluginUi {
    pub events: HashMap<u32, Vec<UiEvent>>,
    pub manager_open: bool,
    pub listed: Option<(std::time::Instant, Result<Vec<super::plugin_manager::Listed>, String>)>,
    pub images: Images,
    pub control_events: HashMap<u64, Vec<UiEvent>>,
    pub control_heights: HashMap<u64, f32>,
}

#[derive(Default)]
pub(crate) struct Images {
    textures: HashMap<(String, String), (u64, egui::TextureHandle)>,
}

impl Images {
    fn texture(&mut self, ctx: &egui::Context, plugin: &str, key: &str) -> Option<egui::TextureId> {
        let Some(image) = effectcraft_plugin::image(plugin, key) else {
            self.textures.remove(&(plugin.to_string(), key.to_string()));
            return None;
        };
        let slot = (plugin.to_string(), key.to_string());
        if let Some((generation, texture)) = self.textures.get(&slot)
            && *generation == image.generation
        {
            return Some(texture.id());
        }
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
        let texture = ctx.load_texture(format!("plugin.{plugin}.{key}"), pixels, egui::TextureOptions::LINEAR);
        let id = texture.id();
        self.textures.insert(slot, (image.generation, texture));
        Some(id)
    }

    pub fn prune(&mut self) {
        self.textures.retain(|(plugin, _), _| effectcraft_plugin::plugins().is_some_and(|p| p.get(plugin).is_some()));
    }
}

pub(crate) struct Source<'a> {
    pub plugin: &'a str,
    pub images: &'a mut Images,
}

fn claimants(app: &EffectcraftApp, kind: PanelKind) -> impl Iterator<Item = &PluginItem> {
    app.session.plugin_items.iter().filter(move |item| item.panel && PanelKind::plugin(&item.plugin, &item.id) == kind)
}

pub fn running_panel(app: &EffectcraftApp, kind: PanelKind) -> Option<&PluginItem> {
    let mut claimants = claimants(app, kind);
    let only = claimants.next()?;
    claimants.next().is_none().then_some(only)
}

pub fn panel(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let Some(item) = running_panel(app, kind).cloned() else { return not_running(app, ui, rect, kind) };
    let PanelKind::Plugin(key) = kind else { return };
    let scope = format!("plugin.{}.{}", item.plugin, item.id);
    app.auto.add(&scope, rect, &item.title);
    let events = app.plugin_ui.events.remove(&key).unwrap_or_default();
    let input = UiInput { events, size: [rect.width(), rect.height()], keys: typed(ui, rect) };
    let tokens = app.tokens;
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    match effectcraft_plugin::panel_ui(&mut app.session, &item.plugin, &item.id, input) {
        Ok(widgets) => {
            let scroll = egui::ScrollArea::both().id_salt(("plugin-panel", key)).auto_shrink([false, false]);
            app.plugin_ui.images.prune();
            let source = Source { plugin: &item.plugin, images: &mut app.plugin_ui.images };
            let events = scroll.show(&mut body, |ui| draw(ui, &widgets, &tokens, &mut app.auto, &scope, source)).inner;
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

fn typed(ui: &egui::Ui, rect: Rect) -> String {
    if ui.ctx().egui_wants_keyboard_input() || !ui.rect_contains_pointer(rect) {
        return String::new();
    }
    ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(text) = e { Some(text.as_str()) } else { None }).collect())
}

fn not_running(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let contested: Vec<String> = claimants(app, kind).map(|item| item.plugin_name.clone()).collect();
    let owner = kind.plugin_owner().and_then(|key| super::plugin_manager::installed(app).into_iter().find(|p| plugin_key(&p.id) == key));
    let message = match owner {
        _ if contested.len() > 1 => tr_args("{} claim the same panel. Disable one of them in Window ▸ Plug-ins ▸ Manage Plug-ins.", &[&contested.join(", ")]),
        Some(plugin) => tr_args("{} is not running. Turn it on in Window ▸ Plug-ins ▸ Manage Plug-ins.", &[&plugin.name]),
        None => tr("The plug-in that provides this panel is not installed. Install it in Window ▸ Plug-ins ▸ Manage Plug-ins.").to_string(),
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

pub(crate) fn draw(ui: &mut egui::Ui, widgets: &[Widget], tokens: &Tokens, auto: &mut Registry, scope: &str, source: Source) -> Vec<UiEvent> {
    let mut renderer = Renderer { tokens, auto, scope, source, events: Vec::new(), last: None };
    renderer.all(ui, widgets);
    renderer.events
}

struct Renderer<'a> {
    tokens: &'a Tokens,
    auto: &'a mut Registry,
    scope: &'a str,
    source: Source<'a>,
    events: Vec<UiEvent>,
    last: Option<egui::Response>,
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
        | Widget::Canvas { id, .. }
        | Widget::Image { id, .. }
        | Widget::Scroll { id, .. }
        | Widget::Collapsing { id, .. }
        | Widget::Tabs { id, .. }
        | Widget::Grid { id, .. } => *id,
        _ => index as u64,
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

fn extent(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, MAX_EXTENT) } else { 0.0 }
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
        self.last = Some(response.clone());
    }

    fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        self.source.images.texture(ctx, self.source.plugin, key)
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
            TextStyle::Error => RichText::new(text).font(Tokens::ui(12.0)).color(t.danger),
        }
    }

    fn widget(&mut self, ui: &mut egui::Ui, widget: &Widget) {
        match widget {
            Widget::Label { text, style } => {
                self.last = Some(ui.label(self.text(text, *style)));
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
            Widget::Image { id, key, size } => self.image(ui, *id, key, *size),
            Widget::Scroll { id, height, children } => {
                egui::ScrollArea::vertical().id_salt(id).max_height(extent(*height)).auto_shrink([false, true]).show(ui, |ui| self.all(ui, children));
            }
            Widget::Collapsing { id, title, open, children } => {
                egui::CollapsingHeader::new(title.as_str()).id_salt(id).default_open(*open).show(ui, |ui| self.all(ui, children));
            }
            Widget::Tabs { id, options, selected } => self.tabs(ui, *id, options, *selected as usize),
            Widget::Progress { fraction, text } => {
                let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
                self.last = Some(ui.add(egui::ProgressBar::new(fraction).text(text.as_str())));
            }
            Widget::Tooltip { text } => {
                if let Some(response) = self.last.take() {
                    response.on_hover_text(text.as_str());
                }
            }
            Widget::Grid { id, columns, children } => self.grid(ui, *id, (*columns).max(1) as usize, children),
        }
    }

    fn image(&mut self, ui: &mut egui::Ui, id: u64, key: &str, size: [f32; 2]) {
        let size = vec2(extent(size[0]), extent(size[1]));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, key));
        match self.texture(ui.ctx(), key) {
            Some(texture) => ui.painter().image(texture, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE),
            None => ui.painter().rect_filled(rect, 2.0, self.tokens.field_bg),
        };
        if response.hovered() {
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, self.tokens.accent), StrokeKind::Inside);
        }
        self.register(id, &response, key);
        if response.clicked() {
            self.emit(id, UiEventKind::Click);
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui, id: u64, options: &[String], selected: usize) {
        ui.horizontal(|ui| {
            for (index, option) in options.iter().enumerate() {
                let tab = ui.selectable_label(index == selected, option);
                self.auto.add_response(&format!("{}.{id}.{index}", self.scope), &tab, option);
                if tab.clicked() && index != selected {
                    self.emit(id, UiEventKind::Number(index as f64));
                }
                self.last = Some(tab);
            }
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, id: u64, columns: usize, children: &[Widget]) {
        egui::Grid::new(id).show(ui, |ui| {
            for (index, child) in children.iter().enumerate() {
                ui.push_id(salt(child, index), |ui| self.widget(ui, child));
                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
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
        self.last = Some(response.clone());
        if let Some(pointer) = pointer(ui, &response, rect.min) {
            self.emit(id, UiEventKind::Pointer(pointer));
        }
    }

    fn shape(&mut self, painter: &egui::Painter, origin: egui::Pos2, shape: &Shape) -> Option<()> {
        let at = |p: [f32; 2]| Some(origin + vec2(drawable(p[0])?, drawable(p[1])?));
        let stroke = |paint: Paint, width: f32| Stroke::new(extent(width), self.paint(paint));
        match shape {
            Shape::Line { points, color, width } => {
                let points = points.iter().take(MAX_LINE_POINTS).copied().map(at).collect::<Option<Vec<_>>>()?;
                painter.add(egui::Shape::line(points, stroke(*color, *width)));
            }
            Shape::Bezier { points, color, width } => {
                let points = [at(points[0])?, at(points[1])?, at(points[2])?, at(points[3])?];
                painter.add(CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke(*color, *width)));
            }
            Shape::Rect { min, max, fill, stroke: outline, radius } => {
                let rect = Rect::from_two_pos(at(*min)?, at(*max)?);
                if let Some(fill) = fill {
                    painter.rect_filled(rect, extent(*radius), self.paint(*fill));
                }
                if let Some((paint, width)) = outline {
                    painter.rect_stroke(rect, extent(*radius), stroke(*paint, *width), StrokeKind::Inside);
                }
            }
            Shape::Circle { center, radius, fill, stroke: outline } => {
                let fill = fill.map_or(Color32::TRANSPARENT, |paint| self.paint(paint));
                let outline = outline.map_or(Stroke::NONE, |(paint, width)| stroke(paint, width));
                painter.circle(at(*center)?, extent(*radius), fill, outline);
            }
            Shape::Text { pos, text, color, size } => {
                painter.text(at(*pos)?, Align2::LEFT_TOP, text, Tokens::ui(font_size(*size)), self.paint(*color));
            }
            Shape::Polygon { points, fill } => {
                let points = points.iter().take(MAX_LINE_POINTS).copied().map(at).collect::<Option<Vec<_>>>()?;
                painter.add(egui::Shape::convex_polygon(points, self.paint(*fill), Stroke::NONE));
            }
            Shape::Image { key, min, max } => {
                let rect = Rect::from_two_pos(at(*min)?, at(*max)?);
                let texture = self.texture(painter.ctx(), key)?;
                painter.image(texture, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        Some(())
    }
}

fn drawable(coordinate: f32) -> Option<f32> {
    coordinate.is_finite().then(|| coordinate.clamp(-MAX_EXTENT, MAX_EXTENT))
}

fn pointer(ui: &egui::Ui, response: &egui::Response, origin: egui::Pos2) -> Option<Pointer> {
    let owed_release = response.id.with("owed-release");
    let (pressed, released, moving, at, held, scroll) = ui.input(|i| {
        let held = Modifiers { shift: i.modifiers.shift, alt: i.modifiers.alt, command: i.modifiers.command };
        let scroll = if response.hovered() { i.smooth_scroll_delta } else { egui::Vec2::ZERO };
        (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.is_moving(), i.pointer.interact_pos(), held, scroll)
    });
    let make = |pos: [f32; 2], phase: PointerPhase| Pointer { pos, phase, modifiers: held, scroll: [scroll.x, scroll.y] };
    if let Some(pos) = ui.data_mut(|data| data.remove_temp::<[f32; 2]>(owed_release)) {
        return Some(make(pos, PointerPhase::Up));
    }
    if pressed && released && response.contains_pointer() {
        let local = at? - origin;
        ui.data_mut(|data| data.insert_temp(owed_release, [local.x, local.y]));
        ui.ctx().request_repaint();
    }
    let phase = if pressed && response.contains_pointer() {
        PointerPhase::Down
    } else if response.clicked() || response.drag_stopped() {
        PointerPhase::Up
    } else if moving && response.is_pointer_button_down_on() {
        PointerPhase::Drag
    } else if (moving || scroll != egui::Vec2::ZERO) && response.hovered() {
        PointerPhase::Hover
    } else {
        return None;
    };
    let local = at? - origin;
    Some(make([local.x, local.y], phase))
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
                let mut images = Images::default();
                events.extend(draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin: "test", images: &mut images }));
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

    #[test]
    fn a_canvas_with_extreme_or_broken_coordinates_still_draws_in_bounded_time() {
        let far = f32::MAX;
        let shapes = vec![
            Shape::Bezier { points: [[0.0, 0.0], [1e30, 1e30], [-1e30, 0.0], [10.0, 10.0]], color: Paint::Accent, width: 1e30 },
            Shape::Bezier { points: [[0.0, 0.0], [f32::NAN, 1.0], [2.0, f32::INFINITY], [10.0, 10.0]], color: Paint::Accent, width: 1.0 },
            Shape::Line { points: vec![[0.0, 0.0], [far, -far], [f32::NAN, 0.0]], color: Paint::Text, width: f32::NAN },
            Shape::Circle { center: [5.0, 5.0], radius: f32::INFINITY, fill: Some(Paint::Error), stroke: None },
            Shape::Rect { min: [-far, -far], max: [far, far], fill: Some(Paint::Surface), stroke: Some((Paint::Border, far)), radius: f32::NAN },
            Shape::Text { pos: [f32::NAN, 0.0], text: "x".into(), color: Paint::Text, size: f32::INFINITY },
        ];
        let mut harness = harness(vec![Widget::Canvas { id: 7, size: [f32::INFINITY, f32::NAN], shapes }]);
        harness.run();
        assert!(harness.state().iter().all(|event| event.id == 7));
    }
}
