use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::panel_tests::{shows, until};
use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::panels::plugin_tools::tests::{harness, probe_session};

const PROBE: &str = "test.viewer-tool-ui";
const CONTROLS: &str = "plugin.test.viewer-tool-ui.controls";

fn open_controls() -> Harness<'static, EffectcraftApp> {
    let mut harness = harness(probe_session());
    let controls = PanelKind::plugin(PROBE, "controls");
    until(&mut harness, "the controls panel registering", |h| crate::panels::plugin_view::running_panel(h.state(), controls).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "controls"})).unwrap();
    until(&mut harness, "the controls showing", |h| shows(h, "Probe controls"));
    harness
}

fn focused(harness: &Harness<'_, EffectcraftApp>) -> Option<String> {
    harness.query_all(egui_kittest::kittest::by()).find(|node| node.accesskit_node().is_focused()).and_then(|node| node.accesskit_node().label())
}

fn shape(harness: &Harness<'_, EffectcraftApp>) -> egui::Rect {
    let found = harness
        .state()
        .auto
        .query(CONTROLS)
        .into_iter()
        .find(|element| element.label == "Shape")
        .map(|element| egui::Rect::from_min_size(egui::pos2(element.rect[0], element.rect[1]), egui::vec2(element.rect[2], element.rect[3])));
    found.expect("the Shape curve is registered under the panel's scope")
}

#[test]
fn the_controls_panel_is_toured_by_tab_from_its_focused_notes_and_counts_a_point_dragged_onto_its_curve() {
    let mut harness = open_controls();
    tab_from_the_focused_notes_reaches_every_plugin_control_in_the_dock(&mut harness);
    dragging_on_the_plugin_curve_adds_a_point_that_the_plugin_counts(&mut harness);
}

fn tab_from_the_focused_notes_reaches_every_plugin_control_in_the_dock(harness: &mut Harness<'static, EffectcraftApp>) {
    until(harness, "the notes taking the focus the panel asked for", |h| focused(h).as_deref() == Some("Notes"));
    let wanted = ["Live", "Plain", "Soft", "Hard", "Clear the notes", "More", "Divider"];
    let mut reached = Vec::new();
    for _ in 0..600 {
        harness.key_press(egui::Key::Tab);
        harness.step();
        if let Some(label) = focused(harness) {
            if label == "Notes" {
                break;
            }
            reached.push(label);
        }
    }
    let missing: Vec<&str> = wanted.iter().copied().filter(|label| !reached.iter().any(|seen| seen == label)).collect();
    assert!(missing.is_empty(), "Tab never reached {missing:?}; it reached {reached:?}");
}

fn dragging_on_the_plugin_curve_adds_a_point_that_the_plugin_counts(harness: &mut Harness<'static, EffectcraftApp>) {
    until(harness, "the curve before the edit", |h| shows(h, "2 points"));
    let graph = shape(harness);
    let start = graph.lerp_inside(egui::vec2(0.3, 0.2));
    let end = graph.lerp_inside(egui::vec2(0.35, 0.15));
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    for event in [egui::Event::PointerMoved(start), button(start, true), egui::Event::PointerMoved(end), button(end, false)] {
        harness.event(event);
        harness.step();
    }
    until(harness, "the plugin counting the new point", |h| shows(h, "3 points"));
}
