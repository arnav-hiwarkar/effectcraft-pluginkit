use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::EffectcraftApp;
use crate::dock::{PanelKind, WorkspaceExt};
use crate::panels::plugin_tools::tests::{harness, probe_session};

const PROBE: &str = "test.viewer-tool-ui";
const FILES: &str = "plugin.test.viewer-tool-ui.files";

fn rows(harness: &Harness<'_, EffectcraftApp>) -> Vec<u32> {
    harness
        .query_all(egui_kittest::kittest::by())
        .filter_map(|node| node.accesskit_node().label().and_then(|label| label.strip_prefix("file ").and_then(|n| n.parse().ok())))
        .collect()
}

fn until(harness: &mut Harness<'_, EffectcraftApp>, what: &str, mut done: impl FnMut(&mut Harness<'_, EffectcraftApp>) -> bool) {
    for _ in 0..2_000 {
        harness.step();
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let items: Vec<String> = harness.state().session.plugin_items.iter().map(|item| format!("{}/{}", item.plugin, item.id)).collect();
    let inspected: Vec<String> =
        harness.query_all(egui_kittest::kittest::by()).filter_map(|n| n.accesskit_node().value()).filter(|t| t.starts_with("inspecting")).collect();
    panic!("{what} never happened; {} plugin items, rows {:?}, inspected {inspected:?}, status {:?}", items.len(), rows(harness), harness.state().ui.status);
}

fn shows(harness: &Harness<'_, EffectcraftApp>, text: &str) -> bool {
    harness.query_all(egui_kittest::kittest::by()).any(|node| node.accesskit_node().label().or(node.accesskit_node().value()).as_deref() == Some(text))
}

#[test]
fn a_hundred_thousand_row_plugin_list_scrolls_in_a_few_calls_and_enter_on_a_row_opens_the_floating_inspector() {
    let mut harness = harness(probe_session());
    let files = PanelKind::plugin(PROBE, "files");
    until(&mut harness, "the files panel registering", |h| crate::panels::plugin_view::running_panel(h.state(), files).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "files"})).unwrap();
    until(&mut harness, "the list showing", |h| shows(h, "file 3"));
    let before = harness.state().plugin_ui.panels.calls(FILES);
    harness.get_by_label("file 3").hover();
    harness.step();
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -40_000.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    };
    harness.event(wheel);
    until(&mut harness, "rows far down the list", |h| rows(h).iter().any(|row| *row > 1_000) && !h.state().plugin_ui.panels.busy(FILES));
    let mut last = Vec::new();
    until(&mut harness, "the scroll coming to rest", |h| {
        let now = rows(h);
        std::mem::replace(&mut last, now.clone()) == now
    });
    let calls = harness.state().plugin_ui.panels.calls(FILES) - before;
    assert!((1..=4).contains(&calls), "scrolling 40 000 points took {calls} plugin calls");
    let row = rows(&harness).into_iter().filter(|row| *row > 1_000).nth(2).expect("rows far down the list are on screen");
    harness.get_by_label(&format!("file {row}")).click();
    harness.step();
    harness.key_press(egui::Key::Enter);
    let inspecting = format!("inspecting file {row}");
    until(&mut harness, "the inspector", |h| shows(h, &inspecting));
    let inspector = PanelKind::plugin(PROBE, "inspector");
    assert!(harness.state().ui.dock.is_floating(inspector), "the inspector opens floating, as its PanelPlace::Float asks");
    let plugin_close = harness
        .query_all_by_label("Close")
        .find(|node| node.accesskit_node().raw_bounds().is_some_and(|bounds| bounds.width() > 30.0))
        .expect("the inspector's own Close button, wider than a tab's close mark");
    plugin_close.click();
    until(&mut harness, "the inspector closing", |h| !h.state().ui.dock.is_open(inspector));
}
