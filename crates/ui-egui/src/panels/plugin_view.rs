use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use craft_plugin::{PanelPlace, PanelSize, PanelSpec};
use craft_plugin_ui::Theme;
use effectcraft_engine::{PluginItem, Session};
use effectcraft_plugin::ui::{Drag, Dropped, UiEvent, Widget};
use effectcraft_plugin::{Owner, Surface};
use egui::{Rect, RichText, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::EffectcraftApp;
use crate::automation::Registry;
use crate::dock::{PanelId, PanelKind};
use crate::i18n::{tr, tr_args};
use crate::theme::Tokens;

#[derive(Default)]
pub(crate) struct PluginUi {
    pub panels: effectcraft_plugin::Panels,
    pub listed: Option<(std::time::Instant, Vec<super::plugin_store::Listed>)>,
    pub images: Images,
    pub control_heights: HashMap<u64, f32>,
    #[cfg(test)]
    pub pressed_keys: Vec<(Owner, String)>,
}

pub(crate) fn tick(app: &mut EffectcraftApp, ctx: &egui::Context) {
    forget_closed_instances(app);
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

pub(crate) fn submit(app: &mut EffectcraftApp, ui: &egui::Ui, key: &Owner, events: Vec<UiEvent>, keys: &str) {
    if !events.is_empty() || !keys.is_empty() {
        app.plugin_ui.panels.submit(key, events, keys);
        ui.ctx().request_repaint();
    }
}

#[derive(Default)]
pub(crate) struct Images {
    textures: HashMap<(String, String), (u64, egui::TextureHandle)>,
    frames: HashMap<(String, egui::Id), Preview>,
    running: Arc<AtomicUsize>,
    tickets: u64,
}

type Rendered = Arc<Mutex<Option<egui::ColorImage>>>;

type Drawer = (Weak<AtomicBool>, Weak<effectcraft_plugin::Loaded>);

struct Preview {
    owner: Drawer,
    asked: Option<(u64, String)>,
    shown: Result<Option<egui::TextureHandle>, String>,
    pending: Option<Rendered>,
    waiting: Option<u64>,
    drawn: f64,
}

const PREVIEW_KEPT_SECONDS: f64 = 5.0;
pub(crate) const RUNNING_PREVIEWS: usize = 2;

fn project_state(session: &Session) -> u64 {
    let mut state = DefaultHasher::new();
    let reloads = session.footage_reloads.load(std::sync::atomic::Ordering::Relaxed);
    (session.revision, session.prefs_revision, reloads, session.active_comp_id().map(|comp| comp.0)).hash(&mut state);
    state.finish()
}

fn owner_of(session: &Session, plugin: &str) -> Option<(Drawer, Arc<effectcraft_plugin::Loaded>)> {
    let loaded = effectcraft_plugin::plugins()?.get(plugin)?;
    Some(((Arc::downgrade(&session.project_closed), Arc::downgrade(&loaded)), loaded))
}

fn same_owner(a: &Drawer, b: &Drawer) -> bool {
    a.0.ptr_eq(&b.0) && a.1.ptr_eq(&b.1)
}

fn rendered(job: effectcraft_plugin::Job<effectcraft_plugin::Pixels>, fit: [usize; 2], max_side: usize, running: &Arc<AtomicUsize>) -> Option<Rendered> {
    let slot = Rendered::default();
    let (fill, counted) = (slot.clone(), running.clone());
    let run = move || {
        let pixels = job();
        let image = effectcraft_engine::raster::Image { width: pixels.width, height: pixels.height, data: pixels.data };
        let size = [image.width as usize, image.height as usize];
        let colour = egui::ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8());
        let shrink = (size[0] / fit[0].max(1)).min(size[1] / fit[1].max(1)).max(1);
        if let Ok(mut fill) = fill.lock() {
            *fill = Some(crate::frames::fit_texture_by(colour, max_side, shrink));
        }
        counted.fetch_sub(1, Ordering::SeqCst);
    };
    running.fetch_add(1, Ordering::SeqCst);
    if cfg!(target_arch = "wasm32") {
        run();
    } else if std::thread::Builder::new().name("plugin-frame".into()).spawn(run).is_err() {
        running.fetch_sub(1, Ordering::SeqCst);
        return None;
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
        widget: egui::Id,
        request: &craft_plugin::FrameRequest,
        size: egui::Vec2,
    ) -> &Result<Option<egui::TextureHandle>, String> {
        let now = ctx.input(|i| i.time);
        self.frames.retain(|_, preview| now - preview.drawn < PREVIEW_KEPT_SECONDS);
        let Some((owner, loaded)) = owner_of(session, plugin) else {
            let gone = Preview {
                owner: Default::default(),
                asked: None,
                shown: Err(format!("the plug-in `{plugin}` is not running")),
                pending: None,
                waiting: None,
                drawn: now,
            };
            return &self.frames.entry((plugin.to_string(), widget)).insert_entry(gone).into_mut().shown;
        };
        let key = (plugin.to_string(), widget);
        let first_other = self.frames.iter().filter(|(other, _)| **other != key).filter_map(|(_, other)| other.waiting).min();
        let fresh = || Preview { owner: owner.clone(), asked: None, shown: Ok(None), pending: None, waiting: None, drawn: now };
        let preview = self.frames.entry(key).or_insert_with(fresh);
        if !same_owner(&preview.owner, &owner) {
            *preview = fresh();
        }
        preview.drawn = now;
        if let Some(pending) = &preview.pending {
            match pending.lock().ok().and_then(|mut done| done.take()) {
                Some(image) => {
                    preview.pending = None;
                    preview.shown = match loaded.plugin.granted(craft_plugin::Permission::DocumentRead) {
                        true => {
                            let fitted = crate::frames::fit_texture(image, crate::frames::max_texture_side(ctx));
                            Ok(Some(ctx.load_texture(format!("plugin.{plugin}.frame"), fitted, egui::TextureOptions::LINEAR)))
                        }
                        false => Err("a frame needs the plug-in's \"document.read\" permission; add it to `permissions` in plugin.json".into()),
                    };
                }
                None => ctx.request_repaint_after(Duration::from_millis(30)),
            }
        }
        let wanted = (project_state(session), serde_json::to_string(request).unwrap_or_default());
        if preview.pending.is_none() && preview.asked.as_ref() != Some(&wanted) && preview.waiting.is_none() {
            self.tickets += 1;
            preview.waiting = Some(self.tickets);
        }
        let turn = preview.waiting.is_some_and(|mine| first_other.is_none_or(|other| mine < other));
        if turn && self.running.load(Ordering::SeqCst) < RUNNING_PREVIEWS {
            preview.waiting = None;
            preview.asked = Some(wanted);
            match effectcraft_plugin::panel_frame(session, plugin, request) {
                Ok(job) => {
                    let ppp = ctx.pixels_per_point();
                    let fit = [(size.x * ppp).ceil() as usize, (size.y * ppp).ceil() as usize];
                    preview.pending = rendered(job, fit, crate::frames::max_texture_side(ctx), &self.running);
                    ctx.request_repaint();
                }
                Err(error) => preview.shown = Err(error),
            }
        } else if preview.waiting.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
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

    fn named(&self, plugin: &str, panel: &str, instance: &str) -> bool {
        self.plugin == plugin && self.panel == panel && self.instance == instance
    }

    fn surface(&self) -> Surface<'_> {
        Surface::Panel { plugin: &self.plugin, panel: &self.panel, instance: Some(&self.instance) }
    }

    fn of(&self, item: &PluginItem) -> bool {
        item.multiple && item.plugin == self.plugin && item.id == self.panel
    }
}

fn instance_of(app: &EffectcraftApp, kind: PanelKind) -> Option<&PanelInstance> {
    app.ui.plugin_instances.iter().find(|open| open.kind() == kind)
}

fn forget_closed_instances(app: &mut EffectcraftApp) {
    use crate::dock::WorkspaceExt;
    let (dock, saved) = (&app.ui.dock, &app.ui.saved_workspaces);
    let kept = |kind: PanelKind| dock.is_open(kind) || saved.values().any(|workspace| workspace.is_open(kind));
    let closed: Vec<PanelInstance> = app.ui.plugin_instances.extract_if(.., |open| !kept(open.kind())).collect();
    for open in &closed {
        app.plugin_ui.panels.close(&app.session, open.surface());
    }
}

pub(crate) fn instance_titles(app: &EffectcraftApp) -> Vec<(PanelKind, String)> {
    let title = |open: &PanelInstance| {
        app.session.plugin_items.iter().find(|item| open.of(item)).map(|item| (open.kind(), format!("{} {}", item.title, open.instance)))
    };
    app.ui.plugin_instances.iter().filter_map(title).collect()
}

pub fn running_panel(app: &EffectcraftApp, kind: PanelKind) -> Option<&PluginItem> {
    if !matches!(kind, PanelKind::Plugin(_)) {
        return None;
    }
    let instance = instance_of(app, kind);
    app.session.plugin_items.iter().find(|item| {
        item.panel
            && match instance {
                Some(open) => open.of(item),
                None => !item.multiple && PanelKind::plugin(&item.plugin, &item.id) == kind,
            }
    })
}

struct Scoped {
    item: PluginItem,
    instance: Option<String>,
    owner: Owner,
    scope: String,
    title: String,
}

pub(crate) use crate::dock::plugin_panel_name as panel_scope;

fn scoped(app: &EffectcraftApp, kind: PanelKind) -> Option<Scoped> {
    let item = running_panel(app, kind)?.clone();
    let instance = instance_of(app, kind).map(|open| open.instance.clone());
    let scope = panel_scope(&item.plugin, &item.id, instance.as_deref());
    let title = instance.as_ref().map_or_else(|| item.title.clone(), |instance| format!("{} {instance}", item.title));
    let owner = Owner::panel(&item.plugin, &item.id, instance.as_deref());
    Some(Scoped { item, instance, owner, scope, title })
}

pub fn panel(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let Some(Scoped { item, instance, owner, scope, title }) = scoped(app, kind) else { return not_running(app, ui, rect, kind) };
    let PanelKind::Plugin(key) = kind else { return };
    app.auto.add(&scope, rect, &title);
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    let surface = Surface::Panel { plugin: &item.plugin, panel: &item.id, instance: instance.as_deref() };
    app.plugin_ui.images.prune();
    let first_widget = app.auto.elements.len();
    let events = shown(app, &mut body, &scope, surface, rect, Some(key));
    for widget in &mut app.auto.elements[first_widget..] {
        widget.panel = Some(kind);
    }
    let keys = if app.ui.focused == kind && !app.floats_blocked(ui.ctx()) { typed(ui) } else { String::new() };
    hold(app, ui, &owner, rect);
    submit(app, ui, &owner, events, &keys);
}

fn hold(app: &mut EffectcraftApp, ui: &egui::Ui, owner: &Owner, rect: Rect) {
    let held = ui.ctx().dragged_id().is_some() && ui.input(|i| i.pointer.press_origin()).is_some_and(|at| rect.contains(at));
    app.plugin_ui.panels.hold(owner, held);
}

pub(crate) fn key_target(app: &EffectcraftApp, ctx: &egui::Context) -> Option<(Owner, Vec<String>)> {
    use crate::dock::WorkspaceExt;
    let kind = app.ui.focused;
    let PanelKind::Plugin(_) = kind else { return None };
    if !app.ui.dock.is_visible(kind) || app.ui.start_screen || app.ui.maximized.is_some_and(|max| max != kind) && !app.ui.dock.is_floating(kind) {
        return None;
    }
    let Scoped { item, owner, .. } = scoped(app, kind)?;
    if crate::framework::focused_widget(ctx)
        .is_some_and(|id| !app.auto.previous.iter().chain(&app.auto.elements).any(|element| element.response_id == Some(id) && element.panel == Some(kind)))
    {
        return None;
    }
    let keys = effectcraft_plugin::panel_spec(&item.plugin, &item.id)?.keys;
    Some((owner, keys))
}

pub(crate) fn panel_menu(app: &mut EffectcraftApp, ui: &mut egui::Ui, kind: PanelKind) -> bool {
    let Some(Scoped { item, owner, scope, .. }) = scoped(app, kind) else { return false };
    let items = app.plugin_ui.panels.menu(&owner).to_vec();
    let help = effectcraft_plugin::panel_spec(&item.plugin, &item.id).and_then(|spec| spec.help);
    let tokens = app.tokens;
    let source = Source { plugin: &item.plugin, images: &mut app.plugin_ui.images, session: None };
    let mut host = AppHost { source, auto: &mut app.auto, tokens };
    let events = craft_plugin_ui::draw_panel_menu(ui, &items, &theme(&tokens), &format!("{scope}.menu"), &mut host);
    let mut picked = !events.is_empty();
    submit(app, ui, &owner, events, "");
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

fn shown(app: &mut EffectcraftApp, body: &mut egui::Ui, scope: &str, surface: Surface, rect: Rect, scroll: Option<PanelId>) -> Vec<UiEvent> {
    let tokens = app.tokens;
    let failed = |body: &mut egui::Ui, error: &str| body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
    match app.plugin_ui.panels.show(&mut app.session, surface, [rect.width(), rect.height()]) {
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
    let scope = panel_scope(plugin, &spec.id, None);
    let owner = Owner::panel(plugin, &spec.id, None);
    app.auto.add(&scope, rect, &spec.title);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout).id_salt(("plugin-row", &scope)));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    let events = shown(app, &mut body, &scope, Surface::Panel { plugin, panel: &spec.id, instance: None }, rect, None);
    hold(app, ui, &owner, rect);
    submit(app, ui, &owner, events, "");
}

pub(crate) fn status_items() -> Vec<(String, PanelSpec)> {
    effectcraft_plugin::placed_panels(PanelPlace::Status)
}

pub(crate) fn status_bar(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, items: &[(String, PanelSpec)]) {
    ui.painter().rect_filled(rect, 0.0, app.tokens.header_bg);
    let mut right = rect.max.x - ROW_GAP;
    for (plugin, spec) in items {
        if right - rect.min.x < ROW_GAP {
            break;
        }
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    Show,
    Float,
    Close,
}

pub(crate) fn open(
    app: &mut EffectcraftApp,
    ctx: &egui::Context,
    plugin: &str,
    panel: &str,
    instance: Option<&str>,
    request: Request,
) -> Result<Value, String> {
    use crate::dock::WorkspaceExt;
    use craft_behavior::dock::Action;
    let close = request == Request::Close;
    let spec = effectcraft_plugin::panel_request(plugin, panel, instance, close)?;
    let instance = match (spec.multiple, instance) {
        (true, Some(key)) => Some(key.to_string()),
        (true, None) => (1..).map(|n: u32| n.to_string()).find(|key| !app.ui.plugin_instances.iter().any(|open| open.named(plugin, panel, key))),
        (false, _) => None,
    };
    let kind = instance.as_ref().map_or_else(|| PanelKind::plugin(plugin, panel), |key| PanelKind::plugin_instance(plugin, panel, key));
    let was_open = app.ui.dock.is_open(kind);
    if close {
        if instance.as_ref().is_some_and(|key| !app.ui.plugin_instances.iter().any(|open| open.named(plugin, panel, key))) {
            return Err(format!("no `{panel}` instance is open"));
        }
        if was_open {
            app.dock_command(ctx, Action::close(kind))?;
        }
        return Ok(json!({"panel": kind.id(), "open": false}));
    }
    if !was_open {
        let anchor = match spec.place {
            PanelPlace::Bottom => PanelKind::Timeline,
            _ => PanelKind::EffectsPresets,
        };
        let placed_before = app.ui.dock.hidden().iter().any(|(hidden, _)| *hidden == kind);
        let mut actions = vec![Action::open(kind, app.ui.dock.is_open(anchor).then_some(anchor))];
        if (spec.place == PanelPlace::Float || request == Request::Float) && !placed_before {
            let screen = ctx.input_for(egui::ViewportId::ROOT, |input| input.content_rect());
            let rect = Rect::from_center_size(screen.center(), float_size(spec.size).min(screen.size()));
            actions.push(Action::float(kind, rect));
        }
        app.apply_dock_all(actions)?;
    }
    if let Some(key) = &instance
        && instance_of(app, kind).is_none()
    {
        app.ui.plugin_instances.push(PanelInstance { plugin: plugin.into(), panel: panel.into(), instance: key.clone() });
    }
    app.show_panel(kind);
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

fn typed(ui: &egui::Ui) -> String {
    if crate::framework::widget_has_keyboard(ui.ctx()) {
        return String::new();
    }
    ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(text) = e { Some(text.as_str()) } else { None }).collect())
}

fn not_running(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let owner = match kind {
        PanelKind::Plugin(id) => id.owner(),
        _ => None,
    };
    let installed = owner.and_then(|id| super::plugin_store::listed(app).into_iter().find(|plugin| plugin.id == id));
    let message = match installed {
        Some(plugin) => tr_args("{} is not running. Turn it on in Plugins ▸ Manage plugins.", &[&plugin.name]),
        None => tr("The plug-in that provides this panel is not installed. Install it in Plugins ▸ Manage plugins.").to_string(),
    };
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(12.0)));
    body.label(RichText::new(message).color(app.tokens.text_dim));
    let manage = body.button(tr("Manage plugins…"));
    app.auto.add_response(&format!("{}.manage", kind.id()), &manage, "Manage plugins…");
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
        "shapes" => Icon::ShapeLayer,
        "wand" => Icon::Wand,
        "sliders" => Icon::Sliders,
        "palette" => Icon::Palette,
        "droplet" => Icon::Droplet,
        "heart" => Icon::Heart,
        "layers" => Icon::Layers,
        "file" => Icon::File,
        "info" => Icon::Info,
        "warning" => Icon::Warning,
        "help" => Icon::Help,
        "stop" => Icon::Stop,
        "copy" => Icon::Copy,
        "crop" => Icon::Crop,
        "move" => Icon::Move,
        "magnet" => Icon::Magnet,
        "flag" => Icon::Flag,
        "tag" => Icon::Tag,
        "bookmark" => Icon::Bookmark,
        "bell" => Icon::Bell,
        "terminal" => Icon::Terminal,
        "puzzle" => Icon::Puzzle,
        "download" => Icon::Download,
        "upload" => Icon::Upload,
        "check" => Icon::Check,
        _ => return None,
    })
}

pub(crate) fn float_size(size: PanelSize) -> egui::Vec2 {
    match size {
        PanelSize::Small => vec2(260.0, 220.0),
        PanelSize::Medium => vec2(360.0, 320.0),
        PanelSize::Large => vec2(520.0, 440.0),
    }
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

pub(crate) fn form_open(app: &EffectcraftApp) -> bool {
    app.framework_dialogs.is_open::<PluginForm>()
}

pub(crate) fn form(app: &mut EffectcraftApp, ctx: &egui::Context) -> Option<u64> {
    use craft_ui::dialog::StackDialog;
    let shown = app.framework_dialogs.get::<PluginForm>();
    let request =
        effectcraft_plugin::asked_form(&app.session, |asked| (asked.id, shown.is_none_or(|dialog| dialog.asked.id != asked.id).then(|| asked.clone())));
    if let Some(dialog) = shown
        && request.as_ref().is_none_or(|(id, _)| *id != dialog.asked.id)
    {
        app.framework_dialogs.cancel_by_id(ctx, dialog.id());
        app.framework_dialogs.take::<PluginForm>();
    }
    let (id, asked) = request?;
    if let Some(asked) = asked {
        let dialog = PluginForm { asked, tokens: app.tokens, auto: Registry::default() };
        let _ = app.framework_dialogs.open_unless_open_in(ctx, dialog);
    }
    Some(id)
}

pub(crate) fn finish_form(app: &mut EffectcraftApp, request: Option<u64>) {
    if let Some(dialog) = app.framework_dialogs.get_mut::<PluginForm>() {
        app.auto.elements.append(&mut dialog.auto.elements);
        effectcraft_plugin::asked_form(&app.session, |asked| {
            if asked.id == dialog.asked.id && asked.values != dialog.asked.values {
                asked.values.clone_from(&dialog.asked.values);
            }
        });
    }
    if let Some(request) = request
        && let Some(result) = app.framework_dialogs.take::<PluginForm>()
        && let Err(error) = effectcraft_plugin::answer_form(&app.session, request, result.ok())
    {
        app.ui.status = error;
    }
}

struct PluginForm {
    asked: effectcraft_plugin::AskedForm,
    tokens: Tokens,
    auto: Registry,
}

impl craft_ui::dialog::StackDialog for PluginForm {
    type Result = effectcraft_plugin::Values;

    fn id(&self) -> craft_ui::dialog::DialogId {
        craft_ui::dialog::dialog_id("plugin.form")
    }

    fn show(&mut self, ui: &mut egui::Ui, stack: &craft_ui::dialog::DialogStack) -> Option<craft_ui::dialog::DialogResult<Self::Result>> {
        use craft_ui::contract::ShowContainer as _;
        use craft_ui::dialog::{Dialog, DialogResult};
        let mut answer = None;
        let mut images = Images::default();
        let source = Source { plugin: &self.asked.plugin, images: &mut images, session: None };
        let mut host = AppHost { source, auto: &mut self.auto, tokens: self.tokens };
        let shown = Dialog::new(self.asked.form.title.clone()).close_on_enter(true).escape_keeps_typing(false).show(ui, |ui, _| {
            answer = craft_plugin_ui::draw_form(ui, &self.asked.form, &mut self.asked.values, "plugin.form", &mut host);
        });
        shown.result(|| self.asked.values.clone()).or_else(|| {
            if !stack.is_top(self.id()) {
                return None;
            }
            answer.map(|answer| match answer {
                craft_plugin_ui::Answer::Ok => DialogResult::Ok(self.asked.values.clone()),
                craft_plugin_ui::Answer::Cancel => DialogResult::Cancel,
            })
        })
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

    fn color(&mut self, ui: &mut egui::Ui, key: &str, value: [f32; 4]) -> Option<(egui::Response, Option<[f32; 4]>)> {
        let mut rgb = [value[0], value[1], value[2]];
        let response = crate::widgets::srgb_color_button(ui, &mut rgb);
        let mut percent = f64::from(value[3].clamp(0.0, 1.0)) * 100.0;
        let alpha = ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).max_decimals(0).suffix("%"));
        ui.ctx().accesskit_node_builder(alpha.id, |node| node.set_label(tr("Opacity").to_string()));
        self.auto.add_response(&format!("{key}.alpha"), &alpha, "Opacity");
        let opacity = if alpha.changed() { (percent / 100.0) as f32 } else { value[3] };
        let picked = (response.changed() || alpha.changed()).then_some([rgb[0], rgb[1], rgb[2], opacity]);
        Some((response, picked))
    }

    fn gradient(&mut self, ui: &mut egui::Ui, key: &str, value: &craft_plugin::Gradient) -> Option<(egui::Response, Option<craft_plugin::Gradient>)> {
        use super::gradient_editor::{Draft, Track, stop_track};
        let selected_id = egui::Id::unique(("plugin-gradient-stop", key));
        let kept_id = egui::Id::unique(("plugin-gradient-draft", key));
        let kept = ui.data(|data| data.get_temp::<Draft>(kept_id)).filter(|draft| plugin_gradient(&draft.gradient()) == *value);
        let mut draft = kept.unwrap_or_else(|| {
            let mut draft = Draft::new(&app_gradient(value), None, Vec::new());
            draft.selected = ui.data(|data| data.get_temp::<(Track, u64)>(selected_id));
            draft
        });
        let before = draft.gradient();
        let width = ui.available_width().clamp(160.0, 320.0);
        let response = stop_track(ui, &mut draft, width, &self.tokens, self.auto, key, craft_plugin::MAX_GRADIENT_STOPS);
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
        ui.data_mut(|data| data.insert_temp(kept_id, draft));
        Some((response, (after != before).then(|| plugin_gradient(&after))))
    }

    fn frame(&mut self, ui: &mut egui::Ui, request: &craft_plugin::FrameRequest, size: egui::Vec2) -> Option<egui::Response> {
        let session = self.source.session?;
        let tokens = self.tokens;
        match self.source.images.frame(ui.ctx(), session, self.source.plugin, ui.scope_id(), request, size) {
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
        egui::ComboBox::new(id, label).selected_text(selected).show_ui(ui, menu).response
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

    #[test]
    fn a_file_dropped_on_a_plugin_drop_target_becomes_readable_by_that_plugin_without_files_read() {
        use crate::panels::plugin_tools::tests::{PROBE, load_probe};
        let outside = tempfile::tempdir().unwrap();
        let dropped = outside.path().join("footage.txt");
        std::fs::write(&dropped, "dropped footage").unwrap();
        let mut session = super::glide_tests::session_with(&[]);
        let bundle = load_probe();
        let plugins = effectcraft_plugin::plugins().unwrap();
        std::fs::write(bundle.join("user-target.txt"), dropped.to_str().unwrap()).unwrap();
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
    fn dragging_a_plugin_colours_opacity_field_lowers_its_alpha_and_keeps_its_colour() {
        let mut harness = harness(vec![Widget::Color { id: 3, text: "Ink".into(), value: [0.25, 0.5, 0.75, 1.0] }]);
        harness.run();
        let field = harness.get_by_label("Opacity").rect().center();
        harness.drag_at(field);
        harness.run();
        harness.hover_at(field - egui::vec2(50.0, 0.0));
        harness.run();
        harness.drop_at(field - egui::vec2(50.0, 0.0));
        harness.run();
        let last = harness.state().last().cloned();
        assert!(
            matches!(last, Some(UiEvent { id: 3, kind: UiEventKind::Color([r, g, b, a]) }) if [r, g, b] == [0.25, 0.5, 0.75] && a < 0.9 && a > 0.0),
            "{last:?}"
        );
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
