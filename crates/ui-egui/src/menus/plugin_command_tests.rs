use egui::{Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::EffectcraftApp;
use crate::panels::plugin_tools::tests::{harness, probe_session, settle};

const PROBE: &str = "test.viewer-tool-ui";

fn names(harness: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    harness.state().session.active_comp().unwrap().layers.iter().map(|layer| layer.name.clone()).collect()
}

fn made(harness: &Harness<'_, EffectcraftApp>) -> usize {
    names(harness).iter().filter(|name| *name == "From Plugin").count()
}

fn center(harness: &Harness<'_, EffectcraftApp>, id: &str) -> Pos2 {
    let found = harness.state().auto.find(id).unwrap_or_else(|| panic!("`{id}` is on screen"));
    pos2(found.rect[0] + found.rect[2] / 2.0, found.rect[1] + found.rect[3] / 2.0)
}

fn click(harness: &mut Harness<'_, EffectcraftApp>, pos: Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE });
    }
    harness.run_steps(2);
}

#[test]
fn a_plugin_command_with_a_menu_path_runs_from_layer_probe_tools() {
    let mut harness = harness(probe_session());
    settle(&mut harness, |h| h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "edit"));
    harness.state_mut().ui.show_menu_bar = true;
    settle(&mut harness, |h| h.state().auto.find("menu.Layer").is_some());
    let layer = center(&harness, "menu.Layer");
    click(&mut harness, layer, egui::PointerButton::Primary);
    assert_eq!(harness.query_all_by_label("Probe: Edit Document").count(), 0, "a command with a menu path leaves Window ▸ Plug-ins");
    harness.get_by_label("Probe Tools").hover();
    harness.run_steps(3);
    harness.get_by_label("Edit Document").click();
    settle(&mut harness, |h| made(h) == 1);
    assert_eq!(names(&harness), ["From Plugin"]);
}

#[test]
fn a_layers_command_runs_from_the_timeline_layer_context_menu_on_the_layer_under_the_pointer() {
    let mut session = probe_session();
    session.execute("layer.newNull", json!({"name": "Target"})).unwrap();
    let target = session.active_comp().unwrap().layers[0].id.0;
    let mut harness = harness(session);
    let row = format!("timeline.layer.{target}.row");
    settle(&mut harness, |h| h.state().auto.find(&row).is_some() && h.state().session.plugin_items.iter().any(|item| item.id == "edit"));
    let pos = center(&harness, &row);
    click(&mut harness, pos, egui::PointerButton::Secondary);
    let entry = format!("timeline.layerMenu.plugin.{PROBE}.edit");
    settle(&mut harness, |h| h.state().auto.find(&entry).is_some());
    assert!(harness.state().auto.find(&format!("viewer.context.plugin.{PROBE}.edit")).is_none(), "the command asks for Layers only");
    let pos = center(&harness, &entry);
    click(&mut harness, pos, egui::PointerButton::Primary);
    settle(&mut harness, |h| made(h) == 1);
    let mut after = names(&harness);
    after.sort();
    assert_eq!(after, ["From Plugin", "Target"]);
}

#[test]
fn a_toolbar_command_runs_by_pointer_and_by_keyboard_and_is_named_by_its_title() {
    let mut harness = harness(probe_session());
    let id = format!("tools.command.{PROBE}.edit");
    settle(&mut harness, |h| h.state().auto.find(&id).is_some());
    let tools = center(&harness, &format!("tools.plugin.{PROBE}.stamp"));
    let button = center(&harness, &id);
    assert!(button.x > tools.x, "the command button comes after the plug-in tools");
    click(&mut harness, button, egui::PointerButton::Primary);
    settle(&mut harness, |h| made(h) == 1);
    harness.event(egui::Event::PointerMoved(pos2(5.0, 790.0)));
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Edit Document").focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    settle(&mut harness, |h| made(h) == 2);
    assert_eq!(made(&harness), 2);
}

#[test]
fn the_app_draws_every_kit_icon_name_it_has_a_drawing_for() {
    let drawn = craft_plugin::ICONS.iter().filter(|name| crate::panels::plugin_view::named_icon(name).is_some()).count();
    assert_eq!(drawn, 33);
}
