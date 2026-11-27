use egui::{Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::panel_tests::{shows, until};
use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::panels::plugin_tools::tests::{bundle, element, harness, menu_row, probe_session, session, wasm};

const PROBE: &str = "test.viewer-tool-ui";
const HELP: &str = "https://example.org/probe-app";

fn names(harness: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    harness.state().session.active_comp().unwrap().layers.iter().map(|layer| layer.name.clone()).collect()
}

fn click(harness: &mut Harness<'_, EffectcraftApp>, pos: Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE });
    }
    harness.run_steps(2);
}

fn with_app_panel() -> Harness<'static, EffectcraftApp> {
    let mut session = probe_session();
    session.execute("layer.newNull", json!({"name": "Keep"})).unwrap();
    let keep = session.active_comp().unwrap().layers[0].id;
    session.state.selected_layers = vec![keep];
    let mut harness = harness(session);
    let kind = PanelKind::plugin(PROBE, "app");
    until(&mut harness, "the app panel registering", |h| super::running_panel(h.state(), kind).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": PROBE, "panel": "app"})).unwrap();
    let owner = effectcraft_plugin::Owner::panel(PROBE, "app", None);
    until(&mut harness, "the app panel drawn", |h| shows(h, "Opacity") && !h.state().plugin_ui.panels.busy(&owner));
    harness
}

fn said(harness: &mut Harness<'_, EffectcraftApp>, text: &str) {
    until(harness, text, |h| shows(h, text));
}

fn delivered(harness: &Harness<'_, EffectcraftApp>, chords: &[&str]) {
    let owner = effectcraft_plugin::Owner::panel(PROBE, "app", None);
    let keys = &harness.state().plugin_ui.pressed_keys;
    assert_eq!(keys, &chords.iter().map(|chord| (owner.clone(), chord.to_string())).collect::<Vec<_>>());
}

#[test]
fn with_the_probe_panel_focused_delete_and_cmd_d_reach_the_plugin_and_backspace_still_clears_the_layer() {
    let mut harness = with_app_panel();
    let body = element(&harness, &format!("plugin.{PROBE}.app"));
    click(&mut harness, body.max - egui::vec2(12.0, 12.0), egui::PointerButton::Primary);
    assert_eq!(harness.state().ui.focused, PanelKind::plugin(PROBE, "app"));
    harness.key_press(egui::Key::Delete);
    said(&mut harness, "pressed Delete");
    harness.event(egui::Event::PointerMoved(pos2(5.0, 790.0)));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    said(&mut harness, "pressed Cmd+D");
    assert_eq!(names(&harness), ["Keep"], "neither key reached the app's Clear or Duplicate");
    delivered(&harness, &["Delete", "Cmd+D"]);
    harness.key_press(egui::Key::Backspace);
    until(&mut harness, "Backspace clearing the layer", |h| names(h).is_empty());
}

#[test]
fn a_panel_key_waits_while_a_file_dialog_is_up() {
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    const ID: &str = "test.modal-shortcut";
    let mut s = session();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modal-key-probe");
    let probe =
        bundle(&wasm(&source, "modal-key-probe"), json!({"id": ID, "name": "Modal Shortcut Probe", "version": "1.0.0", "entry": {"wasm": "plugin.wasm"}}));
    let plugins = effectcraft_plugin::plugins().unwrap();
    plugins.load_for_session(probe.path()).unwrap();
    effectcraft_plugin::install(&mut s, None);
    let mut harness = harness(s);
    let kind = PanelKind::plugin(ID, "keys");
    until(&mut harness, "the shortcut probe registering", |h| super::running_panel(h.state(), kind).is_some());
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", json!({"plugin": ID, "panel": "keys"})).unwrap();
    said(&mut harness, "Waiting for Delete");

    let body = element(&harness, &format!("plugin.{ID}.keys"));
    click(&mut harness, body.max - egui::vec2(12.0, 12.0), egui::PointerButton::Primary);
    assert_eq!(harness.state().ui.focused, kind);
    let pending: Arc<Mutex<Vec<crate::DialogDone>>> = Arc::default();
    let queue = pending.clone();
    harness.state_mut().hooks.file_dialog_async = Some(Box::new(move |_dialog, done| queue.lock().unwrap().push(done)));
    crate::menus::invoke(harness.state_mut(), &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(pending.lock().unwrap().len(), 1, "Save As shows its file dialog");
    harness.key_press(egui::Key::Delete);
    harness.run_steps(3);
    assert!(harness.state().plugin_ui.pressed_keys.is_empty());
    assert!(shows(&harness, "Waiting for Delete"));
    assert!(!shows(&harness, "pressed Delete"));

    pending.lock().unwrap().pop().unwrap()(Ok(Vec::new()));
    harness.run_steps(2);
    harness.key_press(egui::Key::Delete);
    said(&mut harness, "pressed Delete");
    assert_eq!(harness.state().plugin_ui.pressed_keys, [(effectcraft_plugin::Owner::panel(ID, "keys", None), "Delete".into())]);
    plugins.disable(ID).unwrap();
}

#[test]
fn the_panel_menu_lists_the_plugins_items_and_help_and_a_pick_reaches_the_plugin() {
    let mut harness = with_app_panel();
    let menu = format!("panel.menu.{}", PanelKind::plugin(PROBE, "app").id());
    let button = element(&harness, &menu).center();
    click(&mut harness, button, egui::PointerButton::Primary);
    let help = format!("plugin.{PROBE}.app.help");
    until(&mut harness, "the panel menu", |h| shows(h, "Zoom to Fit") && shows(h, "Reset View") && h.state().auto.find(&help).is_some());
    assert!(shows(&harness, "Close Panel"), "the app's own items stay");
    harness.get_by_label("Reset View").click();
    said(&mut harness, "view reset");
    assert!(!shows(&harness, "Zoom to Fit"), "a pick closes the menu");
    click(&mut harness, button, egui::PointerButton::Primary);
    until(&mut harness, "the panel menu again", |h| h.state().auto.find(&help).is_some());
    let help_row = element(&harness, &help);
    harness.query_all_by_label("Help").find(|node| help_row.contains(node.rect().center())).expect("the panel menu's Help").click();
    until(&mut harness, "the help page opening", |h| {
        h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::OpenUrl(open) if open.url == HELP))
    });
}

#[test]
fn a_command_the_probe_turned_off_shows_disabled_in_the_menu_toolbar_and_layer_menu_and_does_not_run() {
    let mut harness = with_app_panel();
    let params = effectcraft_engine::shortcuts::plugin_command_params(PROBE, "app-act");
    harness.get_by_label("Disable Probe Action").click();
    until(&mut harness, "the command turned off", |h| h.state().session.plugin_command(&params).is_some_and(|item| item.disabled));
    let toolbar = format!("tools.command.{PROBE}.app-act");
    let button = element(&harness, &toolbar).center();
    click(&mut harness, button, egui::PointerButton::Primary);
    assert!(harness.get_by_label("Probe Action").accesskit_node().is_disabled(), "the toolbar button is disabled");
    harness.state_mut().ui.show_menu_bar = true;
    until(&mut harness, "the menu bar", |h| h.state().auto.find("menu.Layer").is_some());
    let layer_menu = element(&harness, "menu.Layer").center();
    click(&mut harness, layer_menu, egui::PointerButton::Primary);
    harness.get(menu_row("Probe Tools")).hover();
    harness.run_steps(3);
    let entries: Vec<bool> = harness.query_all(menu_row("Probe Action")).map(|node| node.accesskit_node().is_disabled()).collect();
    assert_eq!(entries, [true, true], "the menu entry and the toolbar button are both disabled");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    let keep = harness.state().session.active_comp().unwrap().layers[0].id.0;
    let row = format!("timeline.layer.{keep}.row");
    let at = element(&harness, &row).center();
    click(&mut harness, at, egui::PointerButton::Secondary);
    let entry = format!("timeline.layerMenu.plugin.{PROBE}.app-act");
    until(&mut harness, "the layer context menu", |h| h.state().auto.find(&entry).is_some());
    let context_entry = element(&harness, &entry);
    let in_context_menu: Vec<bool> = harness
        .query_all(menu_row("Probe Action"))
        .filter(|node| context_entry.contains(node.rect().center()))
        .map(|node| node.accesskit_node().is_disabled())
        .collect();
    assert_eq!(in_context_menu, [true], "the context menu entry is disabled");
    let ctx = harness.ctx.clone();
    let refused = crate::menus::invoke(harness.state_mut(), &ctx, "plugin.run", params.clone());
    assert_eq!(refused.map(drop), Err("Probe turned its command `Probe Action` off for now, so it did not run".to_string()));
    let palette = effectcraft_engine::menus::plugin_commands(&harness.state().session, Some("Layer"));
    let entry = palette.iter().find(|(_, entry)| entry.params == params).map(|(_, entry)| crate::menus::entry_enabled(harness.state(), entry));
    assert_eq!(entry, Some(false), "the palette leaves out what the menus show disabled");
}

#[test]
fn a_panel_key_reaches_the_plugin_while_one_of_its_widgets_has_the_keyboard_and_the_app_keeps_undo() {
    let mut harness = with_app_panel();
    let body = element(&harness, &format!("plugin.{PROBE}.app"));
    click(&mut harness, body.max - egui::vec2(12.0, 12.0), egui::PointerButton::Primary);
    harness.get_by_label("Disable Probe Action").focus();
    harness.run_steps(2);
    assert!(harness.ctx.egui_wants_keyboard_input(), "a widget of the panel holds the keyboard");
    harness.key_press(egui::Key::Delete);
    said(&mut harness, "pressed Delete");
    harness.state_mut().session.execute("layer.newNull", json!({"name": "Undone"})).unwrap();
    harness.run_steps(2);
    assert_eq!(names(&harness), ["Undone", "Keep"]);
    assert!(harness.ctx.egui_wants_keyboard_input(), "the panel's widget still holds the keyboard");
    harness.event(egui::Event::PointerMoved(body.center()));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    until(&mut harness, "Cmd+Z undoing the new layer", |h| names(h) == ["Keep"]);
    delivered(&harness, &["Delete"]);
}

#[test]
fn tab_moves_focus_past_the_last_plugin_control() {
    let mut harness = with_app_panel();
    harness.get_by_label("Disable Probe Action").focus();
    harness.run_steps(2);
    let last = crate::framework::focused_widget(&harness.ctx).expect("the checkbox has focus");
    harness.key_press(egui::Key::Tab);
    harness.run_steps(2);
    assert_ne!(crate::framework::focused_widget(&harness.ctx), Some(last));
    delivered(&harness, &[]);
}

#[test]
fn delete_with_the_timeline_focused_goes_to_the_timeline_even_with_the_pointer_over_a_panel_that_declares_delete() {
    let mut harness = with_app_panel();
    let body = element(&harness, &format!("plugin.{PROBE}.app"));
    harness.state_mut().ui.focused = PanelKind::Timeline;
    harness.event(egui::Event::PointerMoved(body.max - egui::vec2(12.0, 12.0)));
    harness.run_steps(2);
    assert_eq!(harness.state().ui.focused, PanelKind::Timeline);
    harness.key_press(egui::Key::Delete);
    until(&mut harness, "Delete clearing the layer", |h| names(h).is_empty());
    delivered(&harness, &[]);
}

#[test]
fn a_click_on_a_widget_of_the_panel_focuses_it_and_delete_then_reaches_the_plugin_with_the_pointer_elsewhere() {
    let mut harness = with_app_panel();
    harness.state_mut().ui.focused = PanelKind::Timeline;
    let body = element(&harness, &format!("plugin.{PROBE}.app"));
    let widget = harness
        .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::Slider))
        .find(|control| body.contains(control.rect().center()))
        .expect("the Opacity slider is in the plugin panel")
        .rect()
        .center();
    click(&mut harness, widget, egui::PointerButton::Primary);
    assert_eq!(harness.state().ui.focused, PanelKind::plugin(PROBE, "app"));
    harness.event(egui::Event::PointerMoved(pos2(5.0, 790.0)));
    harness.key_press(egui::Key::Delete);
    said(&mut harness, "pressed Delete");
    assert_eq!(names(&harness), ["Keep"], "the key did not reach the app's Clear");
    delivered(&harness, &["Delete"]);
}
