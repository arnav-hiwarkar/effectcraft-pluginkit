use std::collections::HashMap;
use std::time::Duration;

use craft_plugin::{PanelPlace, PanelSize, PanelSpec};
use craft_plugin_ui::Theme;
use effectcraft_engine::PluginItem;
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

pub fn panel(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, kind: PanelKind) {
    let Some(item) = running_panel(app, kind).cloned() else { return not_running(app, ui, rect, kind) };
    let PanelKind::Plugin(key) = kind else { return };
    let instance = instance_of(app, kind).map(|open| open.instance.clone());
    let (scope, title) = match &instance {
        Some(instance) => (format!("plugin.{}.{}.{instance}", item.plugin, item.id), format!("{} {instance}", item.title)),
        None => (format!("plugin.{}.{}", item.plugin, item.id), item.title.clone()),
    };
    app.auto.add(&scope, rect, &title);
    let mut body = ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)));
    let surface = Surface::Panel { plugin: &item.plugin, panel: &item.id, instance: instance.as_deref() };
    app.plugin_ui.images.prune();
    let events = shown(app, &mut body, &scope, surface, rect, Some(key));
    let keys = typed(ui, rect);
    submit(app, ui, &scope, events, &keys);
}

fn shown(app: &mut EffectcraftApp, body: &mut egui::Ui, scope: &str, surface: Surface, rect: Rect, scroll: Option<u32>) -> Vec<UiEvent> {
    let tokens = app.tokens;
    let failed = |body: &mut egui::Ui, error: &str| body.label(RichText::new(tr_args("The plug-in panel failed: {}", &[&error])).color(tokens.danger));
    match app.plugin_ui.panels.show(&mut app.session, scope, surface, [rect.width(), rect.height()]) {
        Ok(shown) => {
            if let Some(error) = shown.error {
                failed(body, error);
            }
            let source = Source { plugin: shown.plugin, images: &mut app.plugin_ui.images };
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
    let mut host = AppHost { source: Source { plugin: &plugin, images: &mut app.plugin_ui.images }, auto: &mut app.auto };
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
            let mut host = AppHost { source: Source { plugin: &asked.plugin, images }, auto };
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
    let source = Source { plugin: "", images: &mut app.plugin_ui.images };
    craft_plugin_ui::param_button(ui, param, enabled, scope, &mut AppHost { source, auto: &mut app.auto })
}

pub(crate) fn triangulate(outline: &[[f64; 2]]) -> Vec<[usize; 3]> {
    effectcraft_model::triangulate::triangulate(outline, &[])
}

pub(crate) fn draw(ui: &mut egui::Ui, widgets: &[Widget], tokens: &Tokens, auto: &mut Registry, scope: &str, source: Source) -> Vec<UiEvent> {
    craft_plugin_ui::draw(ui, widgets, &theme(tokens), scope, &mut AppHost { source, auto })
}

struct AppHost<'a> {
    source: Source<'a>,
    auto: &'a mut Registry,
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

    fn combo_box(&self, ui: &mut egui::Ui, id: egui::IdSalt, label: &str, selected: String, menu: &mut dyn FnMut(&mut egui::Ui)) -> egui::Response {
        craft_ui::ComboBox::new(id, label).selected_text(selected).show_ui(ui, menu).response
    }
}

#[cfg(test)]
pub(crate) mod glide_tests;
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
                events.extend(draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin, images: &mut images }));
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
                draw(ui, &widgets, &tokens, &mut auto, "plugin.test.panel", Source { plugin: "test", images: &mut images });
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
