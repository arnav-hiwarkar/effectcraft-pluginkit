use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::panel_tests::until;
use crate::EffectcraftApp;
use crate::panels::plugin_tools::tests::{STAMP_TOOL, comp_point, element, harness, press_drag_release, probe_session, settle, tool_texts};

const PROBE: &str = "test.viewer-tool-ui";

fn pad_text(harness: &Harness<'_, EffectcraftApp>) -> String {
    harness.query_all(egui_kittest::kittest::by()).filter_map(|node| node.accesskit_node().value()).find(|text| text.starts_with("pad ")).unwrap_or_default()
}

fn open_pad(harness: &mut Harness<'_, EffectcraftApp>) -> egui::Rect {
    let scope = format!("plugin.{PROBE}.pad");
    until(harness, "the pad panel registering", |h| h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "pad"));
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "pad"})).expect("the pad opens");
    until(harness, "the pad drawn", |h| h.state().auto.find(&scope).is_some() && !h.state().plugin_ui.panels.busy(&scope) && !pad_text(h).is_empty());
    element(harness, &scope)
}

#[test]
fn a_right_click_and_a_double_click_on_the_probe_pad_reach_the_plugin_at_the_pointer() {
    let mut harness = harness(probe_session());
    let scope = open_pad(&mut harness);
    let pad = harness.get_all_by_label("Plug-in drawing").find(|node| scope.contains(node.rect().center())).expect("the pad shows its canvas");
    let (at, origin) = (pad.rect().center(), pad.rect().min);
    let expected = |what: &str| format!("pad {what} {:.0} {:.0}", at.x - origin.x, at.y - origin.y);
    pad.click_secondary();
    until(&mut harness, "the right-click reaching the plugin", |h| pad_text(h).starts_with(&expected("context")));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    harness.input_mut().events.extend([egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    until(&mut harness, "the double click reaching the plugin", |h| pad_text(h).starts_with(&expected("open")));
}

#[test]
fn a_plugin_tool_gets_context_on_a_right_click_and_open_after_a_double_click_in_the_viewer() {
    let _one_at_a_time = STAMP_TOOL.lock().unwrap_or_else(|e| e.into_inner());
    let mut harness = harness(probe_session());
    settle(&mut harness, |h| h.state().auto.find(&format!("tools.plugin.{PROBE}.stamp")).is_some());
    let stamp = element(&harness, &format!("tools.plugin.{PROBE}.stamp")).center();
    press_drag_release(&mut harness, &[stamp]);
    harness.state_mut().ui.viewer.zoom = Some(1.0);
    harness.run_steps(2);
    let at = comp_point(&harness, [40.0, 30.0]);
    let button = |button, pressed| egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE };
    let (primary, secondary) = (egui::PointerButton::Primary, egui::PointerButton::Secondary);
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        button(primary, true),
        button(primary, false),
        button(primary, true),
        button(primary, false),
    ]);
    settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
    assert_eq!(tool_texts(&mut harness), ["Down 40.0 30.0", "Up 40.0 30.0", "Open 40.0 30.0"]);
    harness.input_mut().events.extend([button(secondary, true), button(secondary, false)]);
    settle(&mut harness, |h| h.state().plugin_ui.panels.idle());
    assert_eq!(tool_texts(&mut harness).last().map(String::as_str), Some("Context 40.0 30.0"));
}

#[test]
fn the_eyedropper_of_a_plugin_colour_picks_a_red_viewer_pixel_into_the_panel() {
    let mut session = probe_session();
    session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff0000", "width": 320, "height": 180})).unwrap();
    let mut harness = harness(session);
    let scope = open_pad(&mut harness);
    settle(&mut harness, |h| h.state().frames.inflight() == 0 && h.state().viewer_image.is_some());
    let swatch = harness
        .get_all(egui_kittest::kittest::by().role(egui::accesskit::Role::ColorWell))
        .find(|node| scope.contains(node.rect().center()))
        .expect("the pad shows its Ink swatch");
    swatch.click();
    until(&mut harness, "the colour picker", |h| h.state().auto.find("colorPicker.eyedropper").is_some());
    let eyedropper = element(&harness, "colorPicker.eyedropper").center();
    press_drag_release(&mut harness, &[eyedropper]);
    assert!(crate::color_picker::eyedropper_armed(&harness.ctx), "the eyedropper is armed");
    let red = comp_point(&harness, [20.0, 20.0]);
    press_drag_release(&mut harness, &[red]);
    until(&mut harness, "the picked colour reaching the plugin", |h| pad_text(h).ends_with("ink [1.0, 0.0, 0.0, 1.0]"));
}
