use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::panels::plugin_tools::tests::{harness, probe_session};

const PROBE: &str = "test.viewer-tool-ui";

fn until(harness: &mut Harness<'_, EffectcraftApp>, what: &str, mut done: impl FnMut(&mut Harness<'_, EffectcraftApp>) -> bool) {
    for _ in 0..2_000 {
        harness.step();
        if done(harness) && !harness.state().plugin_ui.panels.busy(&effectcraft_plugin::Owner::panel(PROBE, "tree", None)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let focused: Vec<String> =
        harness.query_all(egui_kittest::kittest::by()).filter(|n| n.accesskit_node().is_focused()).filter_map(|n| n.accesskit_node().label()).collect();
    panic!("{what} never happened; focused {focused:?}, status {:?}", harness.state().ui.status);
}

fn shows(harness: &Harness<'_, EffectcraftApp>, text: &str) -> bool {
    harness.query_all(egui_kittest::kittest::by()).any(|node| node.accesskit_node().label().or(node.accesskit_node().value()).as_deref() == Some(text))
}

fn focused(harness: &Harness<'_, EffectcraftApp>, label: &str) -> bool {
    harness.query_all_by_label(label).any(|node| node.accesskit_node().is_focused())
}

#[test]
fn the_probe_tree_is_reached_with_tab_and_its_folders_open_from_the_keyboard_alone() {
    let mut harness = harness(probe_session());
    let tree = PanelKind::plugin(PROBE, "tree");
    until(&mut harness, "the tree panel registering", |h| crate::panels::plugin_view::running_panel(h.state(), tree).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "tree"})).unwrap();
    until(&mut harness, "the tree showing", |h| shows(h, "Folder 0") && shows(h, "take 3  3.3 s  ****"));
    let mut tabs = 0;
    while !focused(&harness, "Folder 0") {
        assert!(tabs < 400, "Tab never reached the tree's first row");
        harness.key_press(egui::Key::Tab);
        harness.step();
        tabs += 1;
    }
    harness.step();
    harness.key_press(egui::Key::ArrowRight);
    until(&mut harness, "Folder 0 opening", |h| shows(h, "Shot 0.1") && shows(h, "opened Folder 0"));
    harness.key_press(egui::Key::ArrowDown);
    until(&mut harness, "the focus moving to Shot 0.0", |h| focused(h, "Shot 0.0"));
    harness.step();
    harness.key_press(egui::Key::ArrowRight);
    until(&mut harness, "Shot 0.0 opening", |h| shows(h, "Take 0.0.1") && shows(h, "opened Shot 0.0"));
    assert!(focused(&harness, "Shot 0.0"), "the focus stays on the row that opened");
    harness.key_press(egui::Key::ArrowLeft);
    until(&mut harness, "Shot 0.0 closing", |h| !shows(h, "Take 0.0.1") && shows(h, "closed Shot 0.0"));
}
