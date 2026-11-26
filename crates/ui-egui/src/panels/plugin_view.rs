use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_plugin::{PanelPlace, PanelSize, PanelSpec};
use craft_plugin_ui::Theme;
use effectcraft_engine::{PluginItem, Session};
use effectcraft_plugin::Surface;
use effectcraft_plugin::ui::{Drag, Dropped, UiEvent, Widget};
use egui::{Rect, RichText, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::EffectcraftApp;
use crate::automation::Registry;
use crate::dock::{PanelKind, plugin_key};
use crate::i18n::{tr, tr_args};
use crate::theme::Tokens;

#[derive(Default)]
pub(crate) struct PluginUi {
    pub panels: effectcraft_plugin::Panels,
    pub manager_open: bool,
    pub listed: Option<(std::time::Instant, Result<Vec<super::plugin_manager::Listed>, String>)>,
    pub images: Images,
    pub control_heights: HashMap<u64, f32>,
    pub pointer: Option<PanelKind>,
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
    if let Some(notice) = panels.take_notice() {
        app.ui.status = notice;
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
    frames: HashMap<(String, String), Preview>,
}

type Rendered = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;

struct Preview {
    asked: Option<u64>,
    shown: Result<Option<egui::TextureHandle>, String>,
    pending: Option<Rendered>,
    drawn: f64,
}

const PREVIEW_KEPT_SECONDS: f64 = 5.0;

fn project_state(session: &Session) -> u64 {
    let mut state = DefaultHasher::new();
    let reloads = session.footage_reloads.load(std::sync::atomic::Ordering::Relaxed);
    (session.revision, session.prefs_revision, reloads, session.active_comp_id().map(|comp| comp.0)).hash(&mut state);
    state.finish()
}

fn rendered(job: effectcraft_plugin::Job<effectcraft_plugin::Pixels>) -> Option<Rendered> {
    let slot = Rendered::default();
    let fill = slot.clone();
    let run = move || {
        let pixels = job();
        let image = effectcraft_engine::raster::Image { width: pixels.width, height: pixels.height, data: pixels.data };
        if let Ok(mut fill) = fill.lock() {
            *fill = Some((image.width, image.height, image.to_rgba8()));
        }
    };
    if cfg!(target_arch = "wasm32") {
        run();
    } else {
        std::thread::Builder::new().name("plugin-frame".into()).spawn(run).ok()?;
    }
    Some(slot)
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

    fn frame(
        &mut self,
        ctx: &egui::Context,
        session: &Session,
        plugin: &str,
        request: &craft_plugin::FrameRequest,
    ) -> &Result<Option<egui::TextureHandle>, String> {
        let now = ctx.input(|i| i.time);
        self.frames.retain(|_, preview| now - preview.drawn < PREVIEW_KEPT_SECONDS);
        let key = (plugin.to_string(), serde_json::to_string(request).unwrap_or_default());
        let preview = self.frames.entry(key).or_insert(Preview { asked: None, shown: Ok(None), pending: None, drawn: now });
        preview.drawn = now;
        if let Some(pending) = &preview.pending {
            match pending.lock().ok().and_then(|mut done| done.take()) {
                Some((width, height, rgba)) => {
                    let pixels = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                    preview.shown = Ok(Some(ctx.load_texture(format!("plugin.{plugin}.frame"), pixels, egui::TextureOptions::LINEAR)));
                    preview.pending = None;
                }
                None => ctx.request_repaint_after(Duration::from_millis(30)),
            }
        }
        let state = project_state(session);
        if preview.pending.is_none() && preview.asked != Some(state) {
            preview.asked = Some(state);
            match effectcraft_plugin::panel_frame(session, plugin, request) {
                Ok(job) => {
                    preview.pending = rendered(job);
                    ctx.request_repaint();
                }
                Err(error) => preview.shown = Err(error),
            }
        }
        &preview.shown
    }
}

pub(crate) struct Source<'a> {
    pub plugin: &'a str,
    pub images: &'a mut Images,
    pub session: Option<&'a Session>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelInstance {
    pub plugin: String,
    pub panel: String,
    pub instance: String,
}

impl PanelInstance {
    fn kind(&self) -> PanelKind {
        PanelKind::plugin_instance(&self.plugin, &self.panel, &self.instance)
    }

    fn of(&self, item: &PluginItem) -> bool {
        item.multiple && item.plugin == self.plugin && item.id == self.panel
    }
}

fn instance_of(app: &EffectcraftApp, kind: PanelKind) -> Option<&PanelInstance> {
    app.ui.plugin_instances.iter().find(|open| open.kind() == kind)
}

fn claimants(app: &EffectcraftApp, kind: PanelKind) -> impl Iterator<Item = &PluginItem> {
    let instance = instance_of(app, kind);
    app.session.plugin_items.iter().filter(move |item| {
        item.panel
            && match instance {
                Some(open) => open.of(item),
                None => PanelKind::plugin(&item.plugin, &item.id) == kind,
            }
    })
}

pub(crate) fn instance_titles(app: &EffectcraftApp) -> Vec<(PanelKind, String)> {
    let title = |open: &PanelInstance| {
        app.session.plugin_items.iter().find(|item| open.of(item)).map(|item| (open.kind(), format!("{} {}", item.title, open.instance)))
    };
    app.ui.plugin_instances.iter().filter_map(title).collect()
}

pub fn running_panel(app: &EffectcraftApp, kind: PanelKind) -> Option<&PluginItem> {
    let mut claimants = claimants(app, kind);
    let only = claimants.next()?;
    claimants.next().is_none().then_some(only)
}

struct Scoped {
    item: PluginItem,
    instance: Option<String>,
    scope: String,
    title: String,
}

fn scoped(app: &EffectcraftApp, kind: PanelKind) -> Option<Scoped> {
    let item = running_panel(app, kind)?.clone();
    let instance = instance_of(app, kind).map(|open| open.instance.clone());
    let (scope, title) = match &instance {
        Some(instance) => (format!("plugin.{}.{}.{instance}", item.plugin, item.id), format!("{} {instance}", item.title)),
        None => (format!("plugin.{}.{}", item.plugin, item.id), item.title.clone()),
    };
    Some(Scoped { item, instance, scope, title })
}

pub fn panel(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let Some(Scoped { item, instance, scope, title }) = scoped(app, kind) else { return not_running(app, ui, rect, kind) };
    let PanelKind::Plugin(key) = kind else { return };
    app.auto.add(&scope, rect, &title);
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    let surface = Surface::Panel { plugin: &item.plugin, panel: &item.id, instance: instance.as_deref() };
    app.plugin_ui.images.prune();
    let events = shown(app, &mut body, &scope, surface, rect, Some(key));
    let keys = typed(ui, rect);
    submit(app, ui, &scope, events, &keys);
    hold(app, ui, &scope, rect);
    if ui.rect_contains_pointer(rect) {
        app.plugin_ui.pointer = Some(kind);
    } else if app.plugin_ui.pointer == Some(kind) {
        app.plugin_ui.pointer = None;
    }
}

fn hold(app: &mut EffectcraftApp, ui: &egui::Ui, scope: &str, rect: Rect) {
    let held = ui.ctx().dragged_id().is_some() && ui.input(|i| i.pointer.press_origin()).is_some_and(|at| rect.contains(at));
    app.plugin_ui.panels.hold(scope, held);
}

pub(crate) fn key_target(app: &EffectcraftApp) -> Option<(String, Vec<String>)> {
    let focused = matches!(app.ui.focused, PanelKind::Plugin(_)).then_some(app.ui.focused);
    let kind = focused.or(app.plugin_ui.pointer.filter(|kind| crate::menus::panel_shown(app, *kind)))?;
    let Scoped { item, scope, .. } = scoped(app, kind)?;
    let keys = effectcraft_plugin::panel_spec(&item.plugin, &item.id)?.keys;
    Some((scope, keys))
}

pub(crate) fn panel_menu(app: &mut EffectcraftApp, ui: &mut egui::Ui, kind: PanelKind) -> bool {
    let Some(Scoped { item, scope, .. }) = scoped(app, kind) else { return false };
    let items = app.plugin_ui.panels.menu(&scope).to_vec();
    let help = effectcraft_plugin::panel_spec(&item.plugin, &item.id).and_then(|spec| spec.help);
    let tokens = app.tokens;
    let source = Source { plugin: &item.plugin, images: &mut app.plugin_ui.images, session: None };
    let mut host = AppHost { source, auto: &mut app.auto, tokens };
    let events = craft_plugin_ui::draw_panel_menu(ui, &items, &theme(&tokens), &format!("{scope}.menu"), &mut host);
    let mut picked = !events.is_empty();
    submit(app, ui, &scope, events, "");
    if let Some(url) = help {
        if !items.is_empty() {
            ui.separator();
        }
        let response = ui.button(tr("Help"));
        app.auto.add_response(&format!("{scope}.help"), &response, "Help");
        if response.clicked() {
            app.session.events.push(effectcraft_engine::Event::OpenUrl(url));
            picked = true;
        }
    }
    if !picked && items.is_empty() {
        ui.label(RichText::new(crate::i18n::label(app, "window.panel", kind.title())).weak());
    }
    picked
}

fn shown(app: &mut EffectcraftApp, body: &mut egui::Ui, scope: &str, surface: Surface, rect: Rect, scroll: Option<u32>) -> Vec<UiEvent> {
    let tokens = app.tokens;
    let failed = |body: &mut egui::Ui, error: &str| body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
    match app.plugin_ui.panels.show(&mut app.session, scope, surface, [rect.width(), rect.height()]) {
        Ok(shown) => {
            if let Some(error) = shown.error {
                failed(body, error);
            }
            let source = Source { plugin: shown.plugin, images: &mut app.plugin_ui.images, session: Some(&app.session) };
            match scroll {
                Some(key) => {
                    let area = egui::ScrollArea::both().id_salt(("plugin-panel", key)).auto_shrink([false, false]);
                    area.show(body, |ui| draw(ui, shown.widgets, &tokens, &mut app.auto, scope, source)).inner
                }
                None => draw(body, shown.widgets, &tokens, &mut app.auto, scope, source),
            }
        }
        Err(error) => {
            failed(body, &error);
            Vec::new()
        }
    }
}

pub(crate) const STATUS_H: f32 = 22.0;
const ROW_GAP: f32 = 8.0;

fn row_width(size: PanelSize) -> f32 {
    match size {
        PanelSize::Small => 160.0,
        PanelSize::Medium => 280.0,
        PanelSize::Large => 440.0,
    }
}

fn row(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, plugin: &str, spec: &PanelSpec) {
    let scope = format!("plugin.{plugin}.{}", spec.id);
    app.auto.add(&scope, rect, &spec.title);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout).id_salt(("plugin-row", &scope)));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    let events = shown(app, &mut body, &scope, Surface::Panel { plugin, panel: &spec.id, instance: None }, rect, None);
    submit(app, ui, &scope, events, "");
    hold(app, ui, &scope, rect);
}

pub(crate) fn status_items() -> Vec<(String, PanelSpec)> {
    effectcraft_plugin::placed_panels(PanelPlace::Status)
}

pub(crate) fn status_bar(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, items: &[(String, PanelSpec)]) {
    ui.painter().rect_filled(rect, 0.0, app.tokens.header_bg);
    let mut right = rect.max.x - ROW_GAP;
    for (plugin, spec) in items {
        let width = row_width(spec.size).min(right - rect.min.x);
        let item = Rect::from_min_max(pos2(right - width, rect.min.y), pos2(right, rect.max.y));
        row(app, ui, item, plugin, spec);
        right = item.min.x - ROW_GAP;
    }
}

pub(crate) fn option_bar(app: &mut EffectcraftApp, ui: &mut egui::Ui, x: f32, cy: f32) -> f32 {
    let Some(tool) = super::plugin_tools::active(app).cloned() else { return x };
    let tools = effectcraft_plugin::viewer::tools();
    let options = tools.into_iter().find(|(plugin, spec)| *plugin == tool.plugin && spec.id == tool.tool).and_then(|(_, spec)| spec.options);
    let Some(spec) = options.and_then(|panel| effectcraft_plugin::panel_spec(&tool.plugin, &panel)).filter(|spec| spec.place == PanelPlace::Bar) else {
        return x;
    };
    let width = row_width(spec.size);
    row(app, ui, Rect::from_min_size(pos2(x, cy - 13.0), vec2(width, 26.0)), &tool.plugin, &spec);
    x + width + ROW_GAP
}

pub(crate) fn open(app: &mut EffectcraftApp, ctx: &egui::Context, plugin: &str, panel: &str, instance: Option<&str>, close: bool) -> Result<Value, String> {
    use crate::dock::WorkspaceExt;
    use craft_ui::docking::Action;
    let spec = effectcraft_plugin::panel_spec(plugin, panel).ok_or_else(|| format!("no running plug-in `{plugin}` has a panel `{panel}`"))?;
    if matches!(spec.place, PanelPlace::Status | PanelPlace::Bar) {
        return Ok(json!({"place": spec.place, "open": true}));
    }
    let instance = match (spec.multiple, instance) {
        (true, Some(key)) => Some(key.to_string()),
        (true, None) if close => return Err(format!("say which `{panel}` to close: host::close_panel(\"{panel}\", Some(key))")),
        (true, None) => (1..).map(|n: u32| n.to_string()).find(|key| !app.ui.dock.is_open(PanelKind::plugin_instance(plugin, panel, key))),
        (false, None) => None,
        (false, Some(_)) => return Err(format!("panel `{panel}` has one instance; declare it with Plugin::panel_each to open one per key")),
    };
    let kind = match &instance {
        Some(key) => {
            let open = PanelInstance { plugin: plugin.into(), panel: panel.into(), instance: key.clone() };
            let kind = open.kind();
            if !close && !app.ui.plugin_instances.contains(&open) {
                app.ui.plugin_instances.push(open);
            }
            kind
        }
        None => PanelKind::plugin(plugin, panel),
    };
    if running_panel(app, kind).is_none() {
        return Err(format!("no running plug-in `{plugin}` has a panel `{panel}`"));
    }
    let was_open = app.ui.dock.is_open(kind);
    if close {
        if was_open {
            app.dock_command(ctx, Action::Close { panel: kind })?;
        }
        return Ok(json!({"panel": kind.id(), "open": false}));
    }
    let placed_before = app.ui.dock.hidden.iter().any(|(hidden, _)| *hidden == kind);
    if spec.place == PanelPlace::Bottom && !was_open && !placed_before && app.ui.dock.is_open(PanelKind::Timeline) {
        app.dock_command(ctx, Action::Open { panel: kind, anchor: Some(PanelKind::Timeline) })?;
    }
    app.show_panel(kind);
    if spec.place == PanelPlace::Float && !was_open {
        let rect = float_rect(ctx.content_rect(), spec.size);
        app.dock_command(ctx, Action::Float { panel: kind, rect })?;
    }
    Ok(json!({"panel": kind.id(), "open": true, "instance": instance}))
}

pub(crate) fn paint_icon(app: &mut EffectcraftApp, painter: &egui::Painter, rect: Rect, kind: PanelKind, color: egui::Color32) -> bool {
    let Some(item) = running_panel(app, kind) else { return false };
    let Some(icon) = effectcraft_plugin::panel_spec(&item.plugin, &item.id).and_then(|spec| spec.icon) else { return false };
    let (plugin, title) = (item.plugin.clone(), item.title.clone());
    let source = Source { plugin: &plugin, images: &mut app.plugin_ui.images, session: Some(&app.session) };
    let mut host = AppHost { source, auto: &mut app.auto, tokens: app.tokens };
    craft_plugin_ui::paint_icon(&mut host, painter, rect, &icon, &title, color);
    true
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

pub(crate) fn named_icon(name: &str) -> Option<crate::icons::Icon> {
    use crate::icons::Icon;
    Some(match name {
        "brush" => Icon::Brush,
        "pen" => Icon::Pen,
        "eraser" => Icon::Eraser,
        "sparkles" => Icon::Sparkle,
        "sun" => Icon::Sun,
        "image" => Icon::Image,
        "film" => Icon::Footage,
        "music" => Icon::Audio,
        "text" => Icon::Type,
        "square" => Icon::Rectangle,
        "circle" => Icon::Ellipse,
        "star" => Icon::Star,
        "grid" => Icon::Grid,
        "list" => Icon::Hamburger,
        "folder" => Icon::Folder,
        "search" => Icon::Search,
        "settings" => Icon::Gear,
        "play" => Icon::Play,
        "pause" => Icon::Pause,
        "plus" => Icon::Plus,
        "minus" => Icon::Minus,
        "trash" => Icon::Trash,
        "eye" => Icon::Eye,
        "lock" => Icon::Lock,
        "link" => Icon::Link,
        "clock" => Icon::Stopwatch,
        "chart" => Icon::Graph,
        "camera" => Icon::Camera,
        "rotate" => Icon::Rotate,
        "zoom" => Icon::Zoom,
        "code" => Icon::Code,
        "refresh" => Icon::Loop,
        "close" => Icon::Close,
        _ => return None,
    })
}

pub(crate) fn float_rect(screen: Rect, size: craft_plugin::PanelSize) -> [f32; 4] {
    let [width, height]: [f32; 2] = match size {
        craft_plugin::PanelSize::Small => [260.0, 220.0],
        craft_plugin::PanelSize::Medium => [360.0, 320.0],
        craft_plugin::PanelSize::Large => [520.0, 440.0],
    };
    let [width, height] = [width.min(screen.width()), height.min(screen.height())];
    let c = screen.center();
    [c.x - width / 2.0, c.y - height / 2.0, width, height]
}

pub(crate) fn theme(t: &Tokens) -> Theme {
    Theme {
        text: t.text,
        weak: t.text_dim,
        accent: t.accent,
        background: t.panel_bg,
        surface: t.field_bg,
        border: t.field_border,
        error: t.danger,
        good: t.cache_green,
        warn: t.warning,
        faint: t.text_faint,
        selected: t.row_selected,
        series: std::array::from_fn(|i| t.labels[i + 1]),
        heading: Tokens::semibold(0.0).family,
    }
}

pub(crate) fn form(app: &mut EffectcraftApp, ctx: &egui::Context) {
    let tokens = app.tokens;
    let (auto, images) = (&mut app.auto, &mut app.plugin_ui.images);
    let answered = effectcraft_plugin::asked_form(&app.session, |asked| {
        let mut answer = None;
        super::dialogs::modal(ctx, &asked.form.title, egui::vec2(420.0, 160.0), &tokens, |ui| {
            let mut host = AppHost { source: Source { plugin: &asked.plugin, images, session: None }, auto, tokens };
            answer = craft_plugin_ui::draw_form(ui, &asked.form, &mut asked.values, "plugin.form", &mut host);
        });
        answer.map(|answer| (answer == craft_plugin_ui::Answer::Ok).then(|| asked.values.clone()))
    });
    if let Some(values) = answered.flatten()
        && let Err(error) = effectcraft_plugin::answer_form(&app.session, values)
    {
        app.ui.status = error;
    }
}

pub(crate) fn param_button(app: &mut EffectcraftApp, ui: &mut egui::Ui, param: &craft_plugin::Param, enabled: bool, scope: &str) -> bool {
    let source = Source { plugin: "", images: &mut app.plugin_ui.images, session: None };
    craft_plugin_ui::param_button(ui, param, enabled, scope, &mut AppHost { source, auto: &mut app.auto, tokens: app.tokens })
}

pub(crate) fn triangulate(outline: &[[f64; 2]]) -> Vec<[usize; 3]> {
    effectcraft_model::triangulate::triangulate(outline, &[])
}

pub(crate) fn draw(ui: &mut egui::Ui, widgets: &[Widget], tokens: &Tokens, auto: &mut Registry, scope: &str, source: Source) -> Vec<UiEvent> {
    craft_plugin_ui::draw(ui, widgets, &theme(tokens), scope, &mut AppHost { source, auto, tokens: *tokens })
}

struct AppHost<'a> {
    source: Source<'a>,
    auto: &'a mut Registry,
    tokens: Tokens,
}

fn round(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

fn app_gradient(gradient: &craft_plugin::Gradient) -> effectcraft_engine::keyframe::Gradient {
    effectcraft_engine::keyframe::Gradient {
        colors: gradient.colors.iter().map(|(at, color)| (*at, color.map(|c| c as f32))).collect(),
        opacities: gradient.opacities.iter().map(|(at, opacity)| (*at, *opacity as f32)).collect(),
    }
}

fn plugin_gradient(gradient: &effectcraft_engine::keyframe::Gradient) -> craft_plugin::Gradient {
    craft_plugin::Gradient {
        colors: gradient.colors.iter().map(|(at, color)| (*at, color.map(round))).collect(),
        opacities: gradient.opacities.iter().map(|(at, opacity)| (*at, round(*opacity))).collect(),
    }
}

impl craft_plugin_ui::Host for AppHost<'_> {
    type Payload = super::DragPayload;

    fn tr(&self, english: &str) -> String {
        tr(english).to_string()
    }

    fn register(&mut self, id: &str, response: &egui::Response, label: &str) {
        self.auto.add_response(id, response, label);
    }

    fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        self.source.images.texture(ctx, self.source.plugin, key)
    }

    fn triangulate(&self) -> craft_plugin_ui::Triangulate {
        triangulate
    }

    fn drag(&self, drag: &Drag) -> Option<super::DragPayload> {
        match drag {
            Drag::File { root, path } => effectcraft_plugin::drag_path(self.source.plugin, *root, path).map(|path| super::DragPayload::Files(vec![path])),
            Drag::Data { data } => Some(super::DragPayload::Plugin { plugin: self.source.plugin.to_string(), data: data.clone() }),
        }
    }

    fn dropped(&self, payload: &super::DragPayload) -> Option<Dropped> {
        match payload {
            super::DragPayload::Files(paths) => Some(Dropped::Files { paths: paths.clone() }),
            super::DragPayload::Item(item) => Some(Dropped::Items { ids: vec![*item] }),
            super::DragPayload::Plugin { plugin, data } => Some(Dropped::Data { plugin: plugin.clone(), data: data.clone() }),
            _ => None,
        }
    }

    fn accept_files(&mut self, paths: &[String]) {
        effectcraft_plugin::accept_drop(self.source.plugin, paths);
    }

    fn icon(&mut self, painter: &egui::Painter, rect: Rect, name: &str, color: egui::Color32) -> bool {
        named_icon(name).inspect(|icon| crate::icons::paint(painter, rect, *icon, color)).is_some()
    }

    fn curve(&mut self, ui: &mut egui::Ui, key: &str, points: &[[f64; 2]]) -> Option<(egui::Response, Option<Vec<[f64; 2]>>)> {
        use super::fx_widgets::{CurveMode, CurvePoints, curves_graph};
        let side = ui.available_width().clamp(120.0, 256.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let shown = CurvePoints(points.iter().map(|point| [point[0] as f32, point[1] as f32]).collect());
        let mode = CurveMode { pencil: false, natural: false, max_points: craft_plugin::MAX_CURVE_POINTS };
        let (response, edit) = curves_graph(ui, rect.shrink(6.0), &shown, 0, mode, egui::Id::unique(("plugin-curve", key)), &self.tokens);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, ""));
        Some((response, edit.changed.map(|edited| edited.0.iter().map(|point| [round(point[0]), round(point[1])]).collect())))
    }

    fn color(&mut self, ui: &mut egui::Ui, _key: &str, value: [f32; 4]) -> Option<(egui::Response, Option<[f32; 4]>)> {
        let mut rgb = [value[0], value[1], value[2]];
        let response = crate::widgets::srgb_color_button(ui, &mut rgb);
        let picked = response.changed().then_some([rgb[0], rgb[1], rgb[2], value[3]]);
        Some((response, picked))
    }

    fn gradient(&mut self, ui: &mut egui::Ui, key: &str, value: &craft_plugin::Gradient) -> Option<(egui::Response, Option<craft_plugin::Gradient>)> {
        use super::gradient_editor::{Draft, Track, stop_track};
        let selected_id = egui::Id::unique(("plugin-gradient-stop", key));
        let mut draft = Draft::new(&app_gradient(value), None, Vec::new());
        draft.selected = ui.data(|data| data.get_temp::<(Track, u64)>(selected_id));
        let before = draft.gradient();
        let width = ui.available_width().clamp(160.0, 320.0);
        let response = stop_track(ui, &mut draft, width, &self.tokens, self.auto, key);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, ""));
        let selected = draft.selected.and_then(|(track, id)| draft.index_of(track, id).map(|index| (track, id, index)));
        if let Some((track, id, index)) = selected {
            ui.horizontal(|ui| {
                if let (Track::Color, Some(stop)) = (track, draft.colors.get(index)) {
                    let mut rgb = [stop.value[0], stop.value[1], stop.value[2]];
                    let swatch = crate::widgets::srgb_color_button(ui, &mut rgb);
                    self.auto.add_response(&format!("{key}.color"), &swatch, "Color");
                    if swatch.changed() {
                        draft.set_rgb(id, rgb);
                    }
                }
                let delete = ui.add_enabled(draft.len(track) > 1, egui::Button::new(tr("Delete stop")));
                self.auto.add_response(&format!("{key}.delete"), &delete, "Delete stop");
                if delete.clicked() {
                    draft.remove(track, id);
                }
            });
        }
        ui.data_mut(|data| match draft.selected {
            Some(selected) => {
                data.insert_temp(selected_id, selected);
            }
            None => data.remove::<(Track, u64)>(selected_id),
        });
        let after = draft.gradient();
        Some((response, (after != before).then(|| plugin_gradient(&after))))
    }

    fn frame(&mut self, ui: &mut egui::Ui, request: &craft_plugin::FrameRequest, size: egui::Vec2) -> Option<egui::Response> {
        let session = self.source.session?;
        let tokens = self.tokens;
        match self.source.images.frame(ui.ctx(), session, self.source.plugin, request) {
            Err(error) => Some(ui.label(RichText::new(error).font(Tokens::ui(12.0)).color(tokens.text_dim))),
            Ok(shown) => {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tr("Frame")));
                ui.painter().rect_filled(rect, 2.0, tokens.field_bg);
                match shown {
                    Some(texture) => {
                        let [width, height] = texture.size().map(|side| side as f32);
                        let scale = (rect.width() / width.max(1.0)).min(rect.height() / height.max(1.0));
                        let fitted = Rect::from_center_size(rect.center(), vec2(width * scale, height * scale));
                        ui.painter().image(texture.id(), fitted, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    None => {
                        let side = rect.height().min(rect.width()).min(16.0);
                        ui.put(Rect::from_center_size(rect.center(), vec2(side, side)), egui::Spinner::new().color(tokens.text_dim));
                    }
                }
                Some(response)
            }
        }
    }

    fn combo_box(&self, ui: &mut egui::Ui, id: egui::IdSalt, label: &str, selected: String, menu: &mut dyn FnMut(&mut egui::Ui)) -> egui::Response {
        craft_ui::ComboBox::new(id, label).selected_text(selected).show_ui(ui, menu).response
    }
}

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod controls_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
pub(crate) mod glide_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod list_tests;
#[cfg(test)]
mod panel_tests;
#[cfg(test)]
mod places_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use effectcraft_plugin::ui::{TextStyle, UiEventKind};
    use egui::Color32;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn harness(widgets: Vec<Widget>) -> Harness<'static, Vec<UiEvent>> {
        harness_for("test", widgets)
    }

    fn harness_for(plugin: &'static str, widgets: Vec<Widget>) -> Harness<'static, Vec<UiEvent>> {
        let tokens = Tokens::for_kind(Default::default());
        Harness::new_ui_state(
            move |ui, events: &mut Vec<UiEvent>| {
                let mut auto = Registry::default();
                let mut images = Images::default();
                events.extend(draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin, images: &mut images, session: None }));
            },
            Vec::new(),
        )
    }

    fn probe_bundle(id: &str) -> tempfile::TempDir {
        let probe = craft_plugin_host::testing::probe_dir();
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(probe.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(status.success(), "the probe builds");
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::copy(probe.join("target/wasm32-unknown-unknown/release/probe.wasm"), dir.path().join("probe.wasm")).expect("copy the probe");
        let manifest = json!({"id": id, "name": "Probe", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": []});
        std::fs::write(dir.path().join("plugin.json"), manifest.to_string()).expect("write the manifest");
        dir
    }

    #[test]
    fn a_file_dropped_on_a_plugin_drop_target_becomes_readable_by_that_plugin_without_files_read() {
        const PROBE: &str = "test.drop-probe";
        let outside = tempfile::tempdir().unwrap();
        let dropped = outside.path().join("footage.txt");
        std::fs::write(&dropped, "dropped footage").unwrap();
        let mut session = super::glide_tests::session_with(&[]);
        let bundle = probe_bundle(PROBE);
        let plugins = effectcraft_plugin::plugins().unwrap();
        plugins.load_for_session(bundle.path()).unwrap();
        std::fs::write(bundle.path().join("user-target.txt"), dropped.to_str().unwrap()).unwrap();
        let refused = effectcraft_plugin::run_command(&mut session, PROBE, "read-user").unwrap_err();
        assert!(refused.contains("files.read"), "{refused}");

        let target = Widget::DropTarget { id: 5, children: vec![Widget::Label { text: "Drop a file".into(), style: TextStyle::Body }] };
        let mut harness = harness_for(PROBE, vec![target]);
        harness.run();
        let at = harness.get_all_by_value("Drop a file").next().map(|node| node.rect().center()).unwrap_or_default();
        let path = dropped.to_string_lossy().into_owned();
        egui::DragAndDrop::set_payload(&harness.ctx, crate::panels::DragPayload::Files(vec![path.clone()]));
        harness.hover_at(at);
        harness.run();
        harness.drop_at(at);
        harness.run();
        assert_eq!(harness.state(), &vec![UiEvent { id: 5, kind: UiEventKind::Drop(Dropped::Files { paths: vec![path] }) }]);

        effectcraft_plugin::run_command(&mut session, PROBE, "read-user").unwrap();
        let data = plugins.get(PROBE).unwrap().data_dir.join("read.txt");
        assert_eq!(std::fs::read_to_string(data).unwrap(), "dropped footage");
    }

    #[test]
    fn a_concave_polygon_is_filled_inside_its_outline_and_not_across_its_notch() {
        let corner = |x: f32, y: f32| egui::pos2(x, y);
        let arrow = vec![corner(0.0, 0.0), corner(10.0, 0.0), corner(10.0, 4.0), corner(4.0, 4.0), corner(4.0, 10.0), corner(0.0, 10.0)];
        let egui::Shape::Mesh(mesh) = craft_plugin_ui::filled_polygon(arrow, Color32::WHITE, triangulate) else { panic!("a mesh was expected") };
        let triangles: Vec<[egui::Pos2; 3]> = mesh.indices.as_chunks::<3>().0.iter().map(|t| t.map(|i| mesh.vertices[i as usize].pos)).collect();
        let area = |[a, b, c]: &[egui::Pos2; 3]| ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() / 2.0;
        assert_eq!(triangles.iter().map(area).sum::<f32>(), 64.0);
        let covers = |point: egui::Pos2| {
            triangles.iter().any(|[a, b, c]| {
                let side = |p: egui::Pos2, q: egui::Pos2| (q.x - p.x) * (point.y - p.y) - (q.y - p.y) * (point.x - p.x);
                let signs = [side(*a, *b), side(*b, *c), side(*c, *a)];
                signs.iter().all(|s| *s >= 0.0) || signs.iter().all(|s| *s <= 0.0)
            })
        };
        assert!(covers(corner(2.0, 7.0)) && covers(corner(7.0, 2.0)) && covers(corner(1.0, 1.0)));
        assert!(!covers(corner(7.0, 7.0)), "the notch of the arrow stays empty");
    }

    #[test]
    fn a_crossing_outline_draws_nothing_and_repeated_points_and_flat_triangles_add_no_triangles() {
        let corners = |points: &[(f32, f32)]| points.iter().map(|&(x, y)| egui::pos2(x, y)).collect::<Vec<_>>();
        let bow_tie = craft_plugin_ui::filled_polygon(corners(&[(0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0)]), Color32::WHITE, triangulate);
        assert!(matches!(bow_tie, egui::Shape::Noop), "{bow_tie:?}");
        let egui::Shape::Mesh(line) = craft_plugin_ui::filled_polygon(corners(&[(0.0, 0.0), (5.0, 5.0), (10.0, 10.0)]), Color32::WHITE, triangulate) else {
            panic!("a mesh was expected")
        };
        assert_eq!(line.indices.len(), 0);
        let square = corners(&[(0.0, 0.0), (0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]);
        let egui::Shape::Mesh(mesh) = craft_plugin_ui::filled_polygon(square, Color32::WHITE, triangulate) else { panic!("a mesh was expected") };
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), (4, 6));
    }

    #[test]
    fn dropping_project_files_on_a_drop_target_sends_their_paths_with_its_id() {
        let target = Widget::DropTarget { id: 12, children: vec![Widget::Label { text: "Drop footage here".into(), style: TextStyle::Body }] };
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

    #[test]
    fn plugin_labels_get_automation_ids_in_draw_order_that_hold_their_text() {
        let tokens = Tokens::for_kind(Default::default());
        let widgets = vec![
            Widget::Label { text: "Ready".into(), style: TextStyle::Body },
            Widget::Row { children: vec![Widget::Label { text: "3 keys".into(), style: TextStyle::Weak }] },
        ];
        let mut harness = Harness::new_ui_state(
            move |ui, labels: &mut Vec<String>| {
                let mut auto = Registry::default();
                let mut images = Images::default();
                draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin: "test", images: &mut images, session: None });
                *labels = (0..3)
                    .filter_map(|n| auto.find(&format!("plugin.test.panel.label.{n}")))
                    .map(|element| format!("{} {}", element.id, element.label))
                    .collect();
            },
            Vec::new(),
        );
        harness.run();
        assert_eq!(harness.state(), &vec!["plugin.test.panel.label.0 Ready".to_string(), "plugin.test.panel.label.1 3 keys".to_string()]);
    }
}
