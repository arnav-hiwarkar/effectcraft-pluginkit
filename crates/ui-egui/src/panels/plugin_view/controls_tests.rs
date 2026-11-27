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
    the_menu_key_on_the_focused_live_toggle_opens_its_context_menu(&mut harness);
    dragging_on_the_plugin_curve_adds_a_point_that_the_plugin_counts(&mut harness);
}

fn the_menu_key_on_the_focused_live_toggle_opens_its_context_menu(harness: &mut Harness<'static, EffectcraftApp>) {
    for _ in 0..8 {
        if focused(harness).as_deref() == Some("Live") {
            break;
        }
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.step();
    }
    assert_eq!(focused(harness).as_deref(), Some("Live"));
    harness.key_press(egui::Key::Menu);
    until(harness, "the context menu opening", |h| shows(h, "Say hello"));
    harness.key_press(egui::Key::Escape);
    harness.step();
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

type Edited = (craft_plugin::Gradient, Vec<(String, egui::Rect)>);

fn gradient_harness(gradient: craft_plugin::Gradient) -> Harness<'static, Edited> {
    use effectcraft_plugin::ui::{UiEventKind, Widget};
    let tokens = crate::theme::Tokens::for_kind(Default::default());
    Harness::new_ui_state(
        move |ui, (gradient, placed): &mut Edited| {
            let mut auto = crate::automation::Registry::default();
            let mut images = super::Images::default();
            let tree = vec![Widget::Gradient { id: 4, text: "Ramp".into(), value: gradient.clone() }];
            let source = super::Source { plugin: "test", images: &mut images, session: None };
            for event in super::draw(ui, &tree, &tokens, &mut auto, "plugin.test.panel", source) {
                if let UiEventKind::Gradient(edited) = event.kind {
                    *gradient = edited;
                }
            }
            let rect = |found: &crate::automation::Element| {
                egui::Rect::from_min_size(egui::pos2(found.rect[0], found.rect[1]), egui::vec2(found.rect[2], found.rect[3]))
            };
            auto.begin_frame();
            *placed = auto.query("plugin.test.panel.4").into_iter().map(|found| (found.id.clone(), rect(found))).collect();
        },
        (gradient, Vec::new()),
    )
}

fn placed(harness: &Harness<'_, Edited>, id: &str) -> egui::Rect {
    harness.state().1.iter().find(|(found, _)| found == id).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("`{id}` is drawn"))
}

#[test]
fn dragging_a_plugin_gradient_stop_past_its_neighbour_over_many_passes_keeps_hold_of_the_same_stop() {
    let red_and_blue = craft_plugin::Gradient { colors: vec![(0.2, [1.0, 0.0, 0.0, 1.0]), (0.5, [0.0, 0.0, 1.0, 1.0])], opacities: vec![(0.0, 1.0)] };
    let mut harness = gradient_harness(red_and_blue);
    harness.run();
    let (red, bar) = (placed(&harness, "plugin.test.panel.4.colorStop.0").center(), placed(&harness, "plugin.test.panel.4.bar"));
    let to = bar.min.x + bar.width() * 0.8;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    harness.event(egui::Event::PointerMoved(red));
    harness.step();
    harness.event(button(red, true));
    harness.step();
    let mut x = red.x;
    while x < to {
        x = (x + 8.0).min(to);
        harness.event(egui::Event::PointerMoved(egui::pos2(x, red.y)));
        harness.step();
    }
    harness.event(button(egui::pos2(x, red.y), false));
    harness.step();
    let colors = &harness.state().0.colors;
    let red_at = colors.iter().find(|(_, color)| color[0] > 0.9).map(|(at, _)| *at);
    let blue_at = colors.iter().find(|(_, color)| color[2] > 0.9).map(|(at, _)| *at);
    assert!(red_at.is_some_and(|at| (at - 0.8).abs() < 0.03), "red ends where it was dropped: {colors:?}");
    assert_eq!(blue_at, Some(0.5), "blue stays where it was: {colors:?}");
}

#[test]
fn a_plugin_gradient_track_at_the_kits_stop_limit_takes_no_more_stops() {
    let crowded = craft_plugin::Gradient {
        colors: vec![(0.0, [1.0, 1.0, 1.0, 1.0])],
        opacities: (0..craft_plugin::MAX_GRADIENT_STOPS - 1).map(|i| (i as f64 * 0.004, 1.0)).collect(),
    };
    let mut harness = gradient_harness(crowded);
    harness.run();
    let row = placed(&harness, "plugin.test.panel.4.addOpacity");
    for at in [0.9, 0.95] {
        let spot = row.lerp_inside(egui::vec2(at, 0.5));
        harness.event(egui::Event::PointerMoved(spot));
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: spot, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            harness.step();
        }
        harness.run();
    }
    assert_eq!(harness.state().0.opacities.len(), craft_plugin::MAX_GRADIENT_STOPS, "the 64th stop was added and a 65th was not");
}
