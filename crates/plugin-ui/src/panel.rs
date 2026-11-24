use craft_plugin::{Drag, Dropped, Key, Modifiers, Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, Widget};
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, vec2};

use crate::{Host, Theme, filled_polygon};

pub(crate) const MAX_EXTENT: f32 = 16_384.0;
const MAX_FONT: f32 = 512.0;
pub(crate) const MAX_LINE_POINTS: usize = 65_536;

pub fn draw<H: Host>(ui: &mut egui::Ui, widgets: &[Widget], theme: &Theme, scope: &str, host: &mut H) -> Vec<UiEvent> {
    let mut renderer = Renderer { theme, host, scope, events: Vec::new(), last: None, labels: 0 };
    renderer.all(ui, widgets);
    renderer.events
}

struct Renderer<'a, H> {
    theme: &'a Theme,
    host: &'a mut H,
    scope: &'a str,
    events: Vec<UiEvent>,
    last: Option<egui::Response>,
    labels: u32,
}

fn plugin_id(widget: &Widget) -> Option<u64> {
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
        | Widget::Grid { id, .. } => Some(*id),
        _ => None,
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

impl<H: Host> Renderer<'_, H> {
    fn all(&mut self, ui: &mut egui::Ui, widgets: &[Widget]) {
        for widget in widgets {
            self.scoped(ui, widget);
        }
    }

    fn scoped(&mut self, ui: &mut egui::Ui, widget: &Widget) {
        match plugin_id(widget) {
            Some(id) => {
                let scope = egui::Id::unique(("plugin-ui", self.scope, id));
                ui.scope_builder(UiBuilder::new().scope_id(scope), |ui| self.widget(ui, widget));
            }
            None => self.widget(ui, widget),
        }
    }

    fn emit(&mut self, id: u64, kind: UiEventKind) {
        self.events.push(UiEvent { id, kind });
    }

    fn register(&mut self, id: u64, response: &egui::Response, label: &str) {
        self.host.register(&format!("{}.{id}", self.scope), response, label);
        self.last = Some(response.clone());
    }

    fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        self.host.texture(ctx, key)
    }

    fn text(&self, text: &str, style: TextStyle) -> RichText {
        let t = self.theme;
        match style {
            TextStyle::Body => RichText::new(text).font(FontId::proportional(12.0)).color(t.text),
            TextStyle::Heading => RichText::new(text).font(FontId::new(15.0, t.heading.clone())).color(t.text),
            TextStyle::Weak => RichText::new(text).font(FontId::proportional(12.0)).color(t.weak),
            TextStyle::Mono => RichText::new(text).font(FontId::monospace(12.0)).color(t.text),
            TextStyle::Error => RichText::new(text).font(FontId::proportional(12.0)).color(t.error),
        }
    }

    fn widget(&mut self, ui: &mut egui::Ui, widget: &Widget) {
        match widget {
            Widget::Label { text, style } => {
                let response = ui.label(self.text(text, *style));
                self.host.register(&format!("{}.label.{}", self.scope, self.labels), &response, text);
                self.labels += 1;
                self.last = Some(response);
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
            Widget::Number { id, text, value, speed, range, decimals } => {
                let mut number = *value;
                let response = ui
                    .horizontal(|ui| {
                        let label = ui.label(self.text(text, TextStyle::Body));
                        let mut drag = egui::DragValue::new(&mut number).speed(finite_or(*speed, 1.0));
                        if let Some([low, high]) = range {
                            drag = drag.range(finite_or(*low, f64::MIN)..=finite_or(*high, f64::MAX));
                        }
                        if let Some(decimals) = decimals {
                            drag = drag.fixed_decimals(usize::from(*decimals));
                        }
                        let response = ui.add(drag).labelled_by(label.id);
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            response.request_focus();
                        }
                        response
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
            Widget::DropTarget { id, children } => self.drop_target(ui, *id, children),
            Widget::DragSource { id, drag, children } => self.drag_source(ui, *id, drag, children),
        }
    }

    fn drop_target(&mut self, ui: &mut egui::Ui, id: u64, children: &[Widget]) {
        let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
        let frame = egui::Frame::new().inner_margin(6.0).corner_radius(4.0).stroke(Stroke::new(1.0, self.theme.border));
        let inner = frame.show(ui, |ui| self.all(ui, children));
        let rect = inner.response.rect;
        let response = ui.interact(rect, egui::Id::unique(("plugin-drop", self.scope, id)), Sense::hover());
        let name = self.host.tr("Drop target");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, &name));
        let hovered = ui.rect_contains_pointer(rect);
        if hovered && (dragging || ui.input(|i| !i.raw.hovered_files.is_empty())) {
            ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, self.theme.accent), StrokeKind::Inside);
        }
        self.register(id, &response, "Drop target");
        if !hovered {
            return;
        }
        let files: Vec<String> =
            ui.input(|i| i.raw.dropped_files.iter().map(|file| file.path().to_string_lossy().into_owned()).filter(|path| !path.is_empty()).collect());
        if !files.is_empty() {
            self.dropped(id, Dropped::Files { paths: files });
        }
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        let dropped = egui::DragAndDrop::payload::<H::Payload>(ui.ctx()).and_then(|payload| self.host.dropped(&payload));
        if let Some(dropped) = dropped {
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.dropped(id, dropped);
        }
    }

    fn dropped(&mut self, id: u64, dropped: Dropped) {
        if let Dropped::Files { paths } = &dropped {
            self.host.accept_files(paths);
        }
        self.emit(id, UiEventKind::Drop(dropped));
    }

    fn drag_source(&mut self, ui: &mut egui::Ui, id: u64, drag: &Drag, children: &[Widget]) {
        let Some(payload) = self.host.drag(drag) else { return self.all(ui, children) };
        let source = egui::Id::unique(("plugin-drag", self.scope, id));
        let response = ui.dnd_drag_source(source, payload, |ui| self.all(ui, children)).response;
        self.register(id, &response, "Drag source");
    }

    fn image(&mut self, ui: &mut egui::Ui, id: u64, key: &str, size: [f32; 2]) {
        let size = vec2(extent(size[0]), extent(size[1]));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, key));
        match self.texture(ui.ctx(), key) {
            Some(texture) => ui.painter().image(texture, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE),
            None => ui.painter().rect_filled(rect, 2.0, self.theme.surface),
        };
        if response.hovered() {
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, self.theme.accent), StrokeKind::Inside);
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
                self.host.register(&format!("{}.{id}.{index}", self.scope), &tab, option);
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
                self.scoped(ui, child);
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
                self.host.register(&format!("{}.{id}.{index}", self.scope), &item, option);
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
        let name = self.host.tr("Plug-in drawing");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, &name));
        self.register(id, &response, "Canvas");
        let painter = ui.painter_at(rect);
        for shape in shapes {
            self.shape(&painter, rect.min, shape);
        }
        for pointer in pointers(ui, &response, rect) {
            self.emit(id, UiEventKind::Pointer(pointer));
        }
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.has_focus() {
            let arrows = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: false, escape: false };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, arrows));
            painter.rect_stroke(rect, 2.0, Stroke::new(2.0, self.theme.accent), StrokeKind::Inside);
            for key in keys(ui) {
                self.emit(id, UiEventKind::Key(key));
            }
        }
        self.last = Some(response);
    }

    fn shape(&mut self, painter: &egui::Painter, origin: egui::Pos2, shape: &Shape) -> Option<()> {
        let at = |p: [f32; 2]| Some(origin + vec2(drawable(p[0])?, drawable(p[1])?));
        let stroke = |paint: Paint, width: f32| Stroke::new(extent(width), self.theme.paint(paint));
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
                    painter.rect_filled(rect, extent(*radius), self.theme.paint(*fill));
                }
                if let Some((paint, width)) = outline {
                    painter.rect_stroke(rect, extent(*radius), stroke(*paint, *width), StrokeKind::Inside);
                }
            }
            Shape::Circle { center, radius, fill, stroke: outline } => {
                let fill = fill.map_or(Color32::TRANSPARENT, |paint| self.theme.paint(paint));
                let outline = outline.map_or(Stroke::NONE, |(paint, width)| stroke(paint, width));
                painter.circle(at(*center)?, extent(*radius), fill, outline);
            }
            Shape::Text { pos, text, color, size } => {
                painter.text(at(*pos)?, Align2::LEFT_TOP, text, FontId::proportional(font_size(*size)), self.theme.paint(*color));
            }
            Shape::Polygon { points, fill } => {
                let points = points.iter().take(MAX_LINE_POINTS).copied().map(at).collect::<Option<Vec<_>>>()?;
                painter.add(filled_polygon(points, self.theme.paint(*fill), self.host.triangulate()));
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

fn modifiers(held: egui::Modifiers) -> Modifiers {
    Modifiers { shift: held.shift, alt: held.alt, command: held.command }
}

fn keys(ui: &egui::Ui) -> Vec<Key> {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers: held, .. } if *key != egui::Key::Tab => {
                    Some(Key { name: key.name().to_string(), modifiers: modifiers(*held) })
                }
                _ => None,
            })
            .collect()
    })
}

fn pointers(ui: &egui::Ui, response: &egui::Response, rect: Rect) -> Vec<Pointer> {
    let held_id = response.id.with("held");
    let mut held = ui.data(|data| data.get_temp::<bool>(held_id)).unwrap_or(false);
    let visible = rect.intersect(ui.clip_rect());
    let over = response.contains_pointer();
    let mut out = Vec::new();
    ui.input(|i| {
        let make = |pos: egui::Pos2, phase, scroll: egui::Vec2| Pointer {
            pos: [pos.x - rect.min.x, pos.y - rect.min.y],
            phase,
            modifiers: modifiers(i.modifiers),
            scroll: [scroll.x, scroll.y],
        };
        for event in &i.events {
            match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } if over && visible.contains(*pos) => {
                    held = true;
                    out.push(make(*pos, PointerPhase::Down, egui::Vec2::ZERO));
                }
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } if held => {
                    held = false;
                    out.push(make(*pos, PointerPhase::Up, egui::Vec2::ZERO));
                }
                egui::Event::PointerMoved(pos) if held => out.push(make(*pos, PointerPhase::Drag, egui::Vec2::ZERO)),
                egui::Event::PointerMoved(pos) if over && visible.contains(*pos) => out.push(make(*pos, PointerPhase::Hover, egui::Vec2::ZERO)),
                _ => {}
            }
        }
        if over
            && i.smooth_scroll_delta != egui::Vec2::ZERO
            && let Some(pos) = i.pointer.latest_pos()
        {
            out.push(make(pos, PointerPhase::Hover, i.smooth_scroll_delta));
        }
    });
    ui.data_mut(|data| data.insert_temp(held_id, held));
    out
}
