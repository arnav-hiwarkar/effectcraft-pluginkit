use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::STATUS_H;
use super::panel_tests::{shows, until};
use crate::EffectcraftApp;
use crate::dock::{PanelKind, WorkspaceExt};
use crate::panels::plugin_tools::tests::{STAMP_TOOL, element, harness, press_drag_release, probe_session, settle};

const PROBE: &str = "test.viewer-tool-ui";

fn open(harness: &mut Harness<'_, EffectcraftApp>, params: Value) -> Value {
    let ctx = harness.ctx.clone();
    crate::menus::invoke(harness.state_mut(), &ctx, "window.pluginPanel", params).expect("the panel opens")
}

fn type_note(harness: &mut Harness<'_, EffectcraftApp>, instance: &str, text: &str) {
    let scope = element(harness, &format!("plugin.{PROBE}.notes.{instance}"));
    let field = harness
        .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::TextInput))
        .find(|node| scope.contains(node.rect().center()))
        .expect("the notes panel shows its text field");
    field.click();
    harness.step();
    harness
        .get_all(egui_kittest::kittest::by().role(egui::accesskit::Role::TextInput))
        .find(|node| scope.contains(node.rect().center()))
        .unwrap()
        .type_text(text);
}

#[test]
fn the_status_item_sits_one_row_high_in_the_status_bar_and_its_button_reaches_the_plugin() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the status item", |h| shows(h, "0 pings"));
    let item = element(&harness, &format!("plugin.{PROBE}.status"));
    assert_eq!((item.height(), item.max.y), (STATUS_H, 800.0));
    harness.get_by_label("Ping").click();
    until(&mut harness, "the ping reaching the plugin", |h| shows(h, "1 pings"));
    let (entries, _) = effectcraft_engine::menus::dynamic(&harness.state().session, "pluginPanels", &Default::default());
    let listed: Vec<&str> = entries.iter().filter(|entry| entry.params["plugin"] == PROBE).map(|entry| entry.label.as_str()).collect();
    assert!(listed.contains(&"New Notes") && !listed.contains(&"Status") && !listed.contains(&"Stamp Options"), "Window ▸ Plug-ins lists {listed:?}");
}

#[test]
fn the_stamp_option_bar_shows_only_while_the_stamp_tool_is_active() {
    let _one_at_a_time = STAMP_TOOL.lock().unwrap_or_else(|e| e.into_inner());
    let mut harness = harness(probe_session());
    settle(&mut harness, |h| h.state().auto.find(&format!("tools.plugin.{PROBE}.stamp")).is_some());
    assert!(!shows(&harness, "hard stamp"), "no option bar before Stamp is picked");
    let stamp = element(&harness, &format!("tools.plugin.{PROBE}.stamp")).center();
    press_drag_release(&mut harness, &[stamp]);
    until(&mut harness, "the option bar", |h| shows(h, "hard stamp"));
    assert_eq!(element(&harness, &format!("plugin.{PROBE}.stamp-options")).height(), 26.0);
    harness.get_by_label("Soft").click();
    until(&mut harness, "the option reaching the plugin", |h| shows(h, "soft stamp"));
    let hand = element(&harness, &format!("tools.{:?}", crate::state::Tool::Hand)).center();
    press_drag_release(&mut harness, &[hand]);
    until(&mut harness, "the option bar going away", |h| !shows(h, "soft stamp"));
    assert_eq!(harness.state().auto.find(&format!("plugin.{PROBE}.stamp-options")).map(|e| e.id.clone()), None);
}

#[test]
fn new_notes_opens_numbered_instances_beside_the_timeline_that_keep_their_own_text() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the probe's notes panel registering", |h| {
        h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "notes")
    });
    let (one, two) = (PanelKind::plugin_instance(PROBE, "notes", "1"), PanelKind::plugin_instance(PROBE, "notes", "2"));
    assert_eq!(open(&mut harness, json!({"plugin": PROBE, "panel": "notes"}))["instance"], "1");
    assert!(harness.state().ui.dock.group_of(one).contains(&PanelKind::Timeline), "a never-placed Bottom panel opens beside the Timeline");
    until(&mut harness, "note 1", |h| {
        h.state().auto.find(&format!("plugin.{PROBE}.notes.1")).is_some() && !h.state().plugin_ui.panels.busy(&format!("plugin.{PROBE}.notes.1"))
    });
    type_note(&mut harness, "1", "milk");
    until(&mut harness, "note 1 holding its text", |h| shows(h, "note 1: milk"));
    assert_eq!(open(&mut harness, json!({"plugin": PROBE, "panel": "notes"}))["instance"], "2");
    until(&mut harness, "note 2", |h| {
        h.state().auto.find(&format!("plugin.{PROBE}.notes.2")).is_some() && !h.state().plugin_ui.panels.busy(&format!("plugin.{PROBE}.notes.2"))
    });
    type_note(&mut harness, "2", "eggs");
    until(&mut harness, "note 2 holding its text", |h| shows(h, "note 2: eggs"));
    let titles: Vec<String> =
        super::instance_titles(harness.state()).into_iter().filter(|(kind, _)| [one, two].contains(kind)).map(|(_, title)| title).collect();
    assert_eq!(titles, ["Notes 1", "Notes 2"]);
    let tab = harness.state().auto.find(&format!("panel.tab.{}", two.id())).map(|tab| tab.label.clone());
    assert_eq!(tab.as_deref(), Some("Notes 2"), "the second tab is titled with its key");
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes", "instance": "1"}));
    until(&mut harness, "note 1 raised with its own text", |h| shows(h, "note 1: milk"));
    assert!(harness.state().ui.dock.is_open(two));
}
