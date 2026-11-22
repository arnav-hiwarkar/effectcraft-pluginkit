use std::collections::HashMap;
use std::time::Duration;

use effectcraft_engine::PluginItem;
use effectcraft_plugin::Surface;
use effectcraft_plugin::ui::{Drag, Dropped, Key, Modifiers, Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, Widget};
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
    pub panels: effectcraft_plugin::Panels,
    pub manager_open: bool,
    pub listed: Option<(std::time::Instant, Result<Vec<super::plugin_manager::Listed>, String>)>,
    pub images: Images,
    pub control_heights: HashMap<u64, f32>,
}

pub(crate) fn tick(app: &mut EffectcraftApp, ctx: &egui::Context) {
    let panels = &mut app.plugin_ui.panels;
    if !panels.has_waker() {
        let waker = ctx.clone();
        panels.set_waker(move || waker.request_repaint());
    }
    if let Some(due) = panels.frame(&mut app.session, now(ctx)) {
        ctx.request_repaint_after(due);
    }
}

fn now(ctx: &egui::Context) -> Duration {
    Duration::from_secs_f64(ctx.input(|i| i.time).max(0.0))
}

pub(crate) fn submit(app: &mut EffectcraftApp, ui: &egui::Ui, key: &str, events: Vec<UiEvent>, keys: &str) {
    if !events.is_empty() || !keys.is_empty() {
        app.plugin_ui.panels.submit(key, events, keys);
        ui.ctx().request_repaint();
    }
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
    let tokens = app.tokens;
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    let surface = Surface::Panel { plugin: &item.plugin, panel: &item.id };
    app.plugin_ui.images.prune();
    let events = match app.plugin_ui.panels.show(&mut app.session, &scope, surface, [rect.width(), rect.height()]) {
        Ok(shown) => {
            if let Some(error) = shown.error {
                body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
            }
            let scroll = egui::ScrollArea::both().id_salt(("plugin-panel", key)).auto_shrink([false, false]);
            let source = Source { plugin: shown.plugin, images: &mut app.plugin_ui.images };
            scroll.show(&mut body, |ui| draw(ui, shown.widgets, &tokens, &mut app.auto, &scope, source)).inner
        }
        Err(error) => {
            body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
            Vec::new()
        }
    };
    let keys = typed(ui, rect);
    submit(app, ui, &scope, events, &keys);
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

impl Renderer<'_> {
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
            Widget::Label { id, text, style } => {
                let response = ui.label(self.text(text, *style));
                if *id != 0 {
                    self.auto.add_response(&format!("{}.{id}", self.scope), &response, text);
                }
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
            Widget::DropTarget { id, children } => self.drop_target(ui, *id, children),
            Widget::DragSource { id, drag, children } => self.drag_source(ui, *id, drag, children),
        }
    }

    fn drop_target(&mut self, ui: &mut egui::Ui, id: u64, children: &[Widget]) {
        let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
        let frame = egui::Frame::new().inner_margin(6.0).corner_radius(4.0).stroke(Stroke::new(1.0, self.tokens.field_border));
        let inner = frame.show(ui, |ui| self.all(ui, children));
        let rect = inner.response.rect;
        let response = ui.interact(rect, egui::Id::unique(("plugin-drop", self.scope, id)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, tr("Drop target")));
        let hovered = ui.rect_contains_pointer(rect);
        if hovered && (dragging || ui.input(|i| !i.raw.hovered_files.is_empty())) {
            ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, self.tokens.accent), StrokeKind::Inside);
        }
        self.register(id, &response, "Drop target");
        if !hovered {
            return;
        }
        let files: Vec<String> = ui.input(|i| i.raw.dropped_files.iter().map(|file| file.path().to_string_lossy().into_owned()).filter(|path| !path.is_empty()).collect());
        if !files.is_empty() {
            self.emit(id, UiEventKind::Drop(Dropped::Files { paths: files }));
        }
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        let dropped = egui::DragAndDrop::payload::<super::DragPayload>(ui.ctx()).and_then(|payload| match payload.as_ref() {
            super::DragPayload::Files(paths) => Some(Dropped::Files { paths: paths.clone() }),
            super::DragPayload::Item(item) => Some(Dropped::Items { ids: vec![*item] }),
            super::DragPayload::Plugin { plugin, data } => Some(Dropped::Data { plugin: plugin.clone(), data: data.clone() }),
            _ => None,
        });
        if let Some(dropped) = dropped {
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.emit(id, UiEventKind::Drop(dropped));
        }
    }

    fn drag_source(&mut self, ui: &mut egui::Ui, id: u64, drag: &Drag, children: &[Widget]) {
        let payload = match drag {
            Drag::File { root, path } => match effectcraft_plugin::drag_path(self.source.plugin, *root, path) {
                Some(path) => super::DragPayload::Files(vec![path]),
                None => return self.all(ui, children),
            },
            Drag::Data { data } => super::DragPayload::Plugin { plugin: self.source.plugin.to_string(), data: data.clone() },
        };
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
        for pointer in pointers(ui, &response, rect) {
            self.emit(id, UiEventKind::Pointer(pointer));
        }
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.has_focus() {
            let arrows = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: false, escape: false };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, arrows));
            painter.rect_stroke(rect, 2.0, Stroke::new(2.0, self.tokens.accent), StrokeKind::Inside);
            for key in keys(ui) {
                self.emit(id, UiEventKind::Key(key));
            }
        }
        self.last = Some(response);
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
        if over && i.smooth_scroll_delta != egui::Vec2::ZERO
            && let Some(pos) = i.pointer.latest_pos()
        {
            out.push(make(pos, PointerPhase::Hover, i.smooth_scroll_delta));
        }
    });
    ui.data_mut(|data| data.insert_temp(held_id, held));
    out
}

#[cfg(test)]
pub(crate) mod glide_tests;

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
    fn an_open_section_inside_a_row_stays_open_when_a_sibling_appears_above_the_row() {
        let tokens = Tokens::for_kind(Default::default());
        let section =
            Widget::Collapsing { id: 2, title: "More".into(), open: false, children: vec![Widget::Button { id: 3, text: "Inside".into(), enabled: true }] };
        let mut harness = Harness::new_ui_state(
            move |ui, widgets: &mut Vec<Widget>| {
                let mut auto = Registry::default();
                let mut images = Images::default();
                draw(ui, widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin: "test", images: &mut images });
            },
            vec![Widget::Row { children: vec![section] }],
        );
        harness.get_by_label("More").click();
        harness.run();
        assert!(harness.query_by_label("Inside").is_some());
        harness.state_mut().insert(0, Widget::Label { id: 0, text: "Name taken".into(), style: TextStyle::Error });
        harness.run();
        assert!(harness.query_by_label("Name taken").is_some());
        assert!(harness.query_by_label("Inside").is_some());
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

    #[test]
    fn clicking_an_image_with_no_upload_yet_still_sends_a_click_with_its_id() {
        let image = Widget::Image { id: 9, key: "thumb/0".into(), size: [40.0, 30.0] };
        let tip = Widget::Tooltip { text: "Linear".into() };
        let cell = Widget::Column { children: vec![image, tip] };
        let grid = Widget::Grid { id: 8, columns: 3, children: vec![cell] };
        let mut harness = harness(vec![Widget::Scroll { id: 7, height: 100.0, children: vec![grid] }]);
        harness.get_by_label("thumb/0").click();
        harness.run();
        assert_eq!(harness.state(), &vec![UiEvent { id: 9, kind: UiEventKind::Click }]);
    }

    #[test]
    fn clicking_a_tab_inside_a_scroll_area_and_a_grid_sends_its_index() {
        let tabs = Widget::Tabs { id: 5, options: vec!["Red".into(), "Green".into()], selected: 0 };
        let grid = Widget::Grid { id: 6, columns: 2, children: vec![tabs, Widget::Progress { fraction: 0.5, text: "Half".into() }] };
        let mut harness = harness(vec![Widget::Scroll { id: 4, height: 80.0, children: vec![grid] }]);
        harness.get_by_label("Green").click();
        harness.run();
        assert_eq!(harness.state(), &vec![UiEvent { id: 5, kind: UiEventKind::Number(1.0) }]);
    }

    #[test]
    fn a_collapsing_section_closed_by_default_hides_its_children_until_opened() {
        let button = Widget::Button { id: 3, text: "Inside".into(), enabled: true };
        let mut harness = harness(vec![Widget::Collapsing { id: 2, title: "More".into(), open: false, children: vec![button] }]);
        harness.run();
        assert!(harness.query_by_label("Inside").is_none());
        harness.get_by_label("More").click();
        harness.run();
        harness.run();
        assert!(harness.query_by_label("Inside").is_some());
    }

    #[test]
    fn dragging_on_a_canvas_sends_down_then_drags_then_up_with_the_held_modifiers() {
        let mut harness = harness(vec![Widget::Canvas { id: 11, size: [100.0, 100.0], shapes: vec![] }]);
        harness.run();
        let rect = harness.get_by_label("Plug-in drawing").rect();
        harness.drag_at(rect.center());
        harness.run();
        harness.hover_at(rect.center() + vec2(10.0, 0.0));
        harness.run();
        harness.hover_at(rect.center() + vec2(20.0, 0.0));
        harness.run();
        harness.drop_at(rect.center() + vec2(20.0, 0.0));
        harness.run();
        let phases: Vec<PointerPhase> = harness
            .state()
            .iter()
            .filter_map(|event| match event.kind {
                UiEventKind::Pointer(pointer) if event.id == 11 => Some(pointer.phase),
                _ => None,
            })
            .collect();
        assert_eq!(phases.first(), Some(&PointerPhase::Down), "{phases:?}");
        assert!(phases.contains(&PointerPhase::Drag), "{phases:?}");
        assert_eq!(phases.last(), Some(&PointerPhase::Up), "{phases:?}");
    }

    fn canvas_events(harness: &Harness<'static, Vec<UiEvent>>) -> Vec<UiEventKind> {
        harness.state().iter().filter(|event| event.id == 11).map(|event| event.kind.clone()).collect()
    }

    #[test]
    fn a_canvas_reached_with_tab_gets_arrow_keys_with_their_modifiers_and_keeps_the_focus() {
        let button = Widget::Button { id: 10, text: "Before".into(), enabled: true };
        let mut harness = harness(vec![button, Widget::Canvas { id: 11, size: [100.0, 60.0], shapes: vec![] }]);
        harness.run();
        harness.key_press(egui::Key::Tab);
        harness.run();
        harness.key_press(egui::Key::Tab);
        harness.run();
        assert!(harness.get_by_label("Plug-in drawing").is_focused());
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowLeft);
        harness.run();
        harness.key_press(egui::Key::ArrowDown);
        harness.run();
        let shift = Modifiers { shift: true, alt: false, command: false };
        assert_eq!(
            canvas_events(&harness),
            vec![
                UiEventKind::Key(Key { name: "Left".into(), modifiers: shift }),
                UiEventKind::Key(Key { name: "Down".into(), modifiers: Modifiers::default() })
            ]
        );
        assert!(harness.get_by_label("Plug-in drawing").is_focused());
    }

    #[test]
    fn every_pointer_move_of_a_frame_reaches_the_canvas_in_order() {
        let mut harness = harness(vec![Widget::Canvas { id: 11, size: [100.0, 100.0], shapes: vec![] }]);
        harness.run();
        let origin = harness.get_by_label("Plug-in drawing").rect().min;
        harness.drag_at(origin + vec2(10.0, 10.0));
        harness.run();
        harness.state_mut().clear();
        for x in [20.0, 30.0, 40.0] {
            harness.event(egui::Event::PointerMoved(origin + vec2(x, 10.0)));
        }
        harness.run();
        let drags: Vec<[f32; 2]> = canvas_events(&harness)
            .into_iter()
            .filter_map(|kind| match kind {
                UiEventKind::Pointer(Pointer { phase: PointerPhase::Drag, pos, .. }) => Some(pos),
                _ => None,
            })
            .collect();
        assert_eq!(drags, vec![[20.0, 10.0], [30.0, 10.0], [40.0, 10.0]]);
    }

    #[test]
    fn dropping_project_files_on_a_drop_target_sends_their_paths_with_its_id() {
        let target = Widget::DropTarget { id: 12, children: vec![Widget::Label { id: 13, text: "Drop footage here".into(), style: TextStyle::Body }] };
        let mut harness = harness(vec![target]);
        harness.run();
        let at = harness.get_all_by_value("Drop footage here").next().map(|node| node.rect().center()).unwrap_or_default();
        egui::DragAndDrop::set_payload(&harness.ctx, crate::panels::DragPayload::Files(vec!["/footage/a.mov".into()]));
        harness.hover_at(at);
        harness.run();
        harness.drop_at(at);
        harness.run();
        assert_eq!(harness.state(), &vec![UiEvent { id: 12, kind: UiEventKind::Drop(Dropped::Files { paths: vec!["/footage/a.mov".into()] }) }]);
    }
}
