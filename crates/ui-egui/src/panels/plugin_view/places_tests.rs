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
    let scope = element(harness, &format!("plugin.{PROBE}.notes#{instance}"));
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
        h.state().auto.find(&format!("plugin.{PROBE}.notes#1")).is_some()
            && !h.state().plugin_ui.panels.busy(&effectcraft_plugin::Owner::panel(PROBE, "notes", Some("1")))
    });
    type_note(&mut harness, "1", "milk");
    until(&mut harness, "note 1 holding its text", |h| shows(h, "note 1: milk"));
    assert_eq!(open(&mut harness, json!({"plugin": PROBE, "panel": "notes"}))["instance"], "2");
    until(&mut harness, "note 2", |h| {
        h.state().auto.find(&format!("plugin.{PROBE}.notes#2")).is_some()
            && !h.state().plugin_ui.panels.busy(&effectcraft_plugin::Owner::panel(PROBE, "notes", Some("2")))
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

fn note_shown(harness: &mut Harness<'_, EffectcraftApp>, instance: &str) {
    let scope = format!("plugin.{PROBE}.notes#{instance}");
    let owner = effectcraft_plugin::Owner::panel(PROBE, "notes", Some(instance));
    until(harness, &format!("note {instance}"), |h| h.state().auto.find(&scope).is_some() && !h.state().plugin_ui.panels.busy(&owner));
}

#[test]
fn a_closed_notes_instance_is_forgotten_so_the_next_new_notes_starts_empty() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the probe's notes panel registering", |h| {
        h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "notes")
    });
    assert_eq!(open(&mut harness, json!({"plugin": PROBE, "panel": "notes"}))["instance"], "1");
    note_shown(&mut harness, "1");
    type_note(&mut harness, "1", "milk");
    until(&mut harness, "note 1 holding its text", |h| shows(h, "note 1: milk"));
    let ctx = harness.ctx.clone();
    let one = PanelKind::plugin_instance(PROBE, "notes", "1");
    harness.state_mut().dock_command(&ctx, craft_ui::docking::Action::Close { panel: one }).unwrap();
    until(&mut harness, "the instance forgotten", |h| !h.state().ui.plugin_instances.iter().any(|open| open.plugin == PROBE && open.panel == "notes"));
    assert_eq!(open(&mut harness, json!({"plugin": PROBE, "panel": "notes"}))["instance"], "1");
    note_shown(&mut harness, "1");
    until(&mut harness, "the new note 1, empty", |h| shows(h, "note 1: "));
    assert!(!shows(&harness, "note 1: milk"));
}

#[test]
fn two_instance_keys_whose_dock_keys_would_fold_together_open_two_tabs_with_their_own_text() {
    assert_eq!(PanelKind::plugin_instance(PROBE, "notes", "k384"), PanelKind::plugin_instance(PROBE, "notes", "k608"), "the keys fold to one u16");
    let mut harness = harness(probe_session());
    until(&mut harness, "the probe's notes panel registering", |h| {
        h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "notes")
    });
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes", "instance": "k384"}));
    note_shown(&mut harness, "k384");
    type_note(&mut harness, "k384", "milk");
    until(&mut harness, "the first note holding its text", |h| shows(h, "note k384: milk"));
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes", "instance": "k608"}));
    note_shown(&mut harness, "k608");
    until(&mut harness, "the second note, empty", |h| shows(h, "note k608: "));
    let kinds: Vec<PanelKind> = harness.state().ui.plugin_instances.iter().filter(|open| open.plugin == PROBE).map(|open| open.kind()).collect();
    assert_eq!(kinds.len(), 2);
    assert_ne!(kinds[0], kinds[1]);
    assert!(kinds.iter().all(|kind| harness.state().ui.dock.is_open(*kind)), "both tabs are open");
}

#[test]
fn a_status_item_cannot_be_closed_and_says_why() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the status item", |h| shows(h, "0 pings"));
    let ctx = harness.ctx.clone();
    let refused = super::open(harness.state_mut(), &ctx, PROBE, "status", None, true).unwrap_err();
    assert!(refused.contains("cannot be opened or closed"), "{refused}");
}

#[test]
fn a_notes_instance_named_1_and_a_panel_named_notes_dot_1_keep_their_own_text_and_close_apart() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the probe's dotted notes panel registering", |h| {
        h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "notes.1")
    });
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes", "instance": "1"}));
    note_shown(&mut harness, "1");
    type_note(&mut harness, "1", "milk");
    until(&mut harness, "note 1 holding its text", |h| shows(h, "note 1: milk"));
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes.1"}));
    let (dotted, owner) = (format!("plugin.{PROBE}.notes%2E1"), effectcraft_plugin::Owner::panel(PROBE, "notes.1", None));
    until(&mut harness, "the dotted panel, empty", |h| {
        h.state().auto.find(&dotted).is_some() && !h.state().plugin_ui.panels.busy(&owner) && shows(h, "note -: ")
    });
    assert!(shows(&harness, "note 1: milk"), "the instance keeps its own text beside the dotted panel");
    let ctx = harness.ctx.clone();
    let one = PanelKind::plugin_instance(PROBE, "notes", "1");
    harness.state_mut().dock_command(&ctx, craft_ui::docking::Action::Close { panel: one }).unwrap();
    until(&mut harness, "the instance forgotten", |h| !h.state().ui.plugin_instances.iter().any(|open| open.plugin == PROBE && open.panel == "notes"));
    let field = element(&harness, &dotted);
    let input = harness
        .get_all(egui_kittest::kittest::by().role(egui::accesskit::Role::TextInput))
        .find(|node| field.contains(node.rect().center()))
        .expect("the dotted panel shows its text field");
    input.click();
    harness.step();
    harness
        .get_all(egui_kittest::kittest::by().role(egui::accesskit::Role::TextInput))
        .find(|node| field.contains(node.rect().center()))
        .unwrap()
        .type_text("tea");
    until(&mut harness, "the dotted panel taking its own text after the instance closed", |h| shows(h, "note -: tea"));
}

#[test]
fn a_notes_instance_in_a_saved_workspace_keeps_its_title_and_text_after_switching_away_and_back() {
    let mut harness = harness(probe_session());
    until(&mut harness, "the probe's notes panel registering", |h| {
        h.state().session.plugin_items.iter().any(|item| item.plugin == PROBE && item.id == "notes")
    });
    open(&mut harness, json!({"plugin": PROBE, "panel": "notes", "instance": "w1"}));
    note_shown(&mut harness, "w1");
    type_note(&mut harness, "w1", "milk");
    until(&mut harness, "the note holding its text", |h| shows(h, "note w1: milk"));
    let saved = harness.state().ui.dock.clone();
    harness.state_mut().ui.saved_workspaces.insert("Notes Desk".into(), saved);
    harness.state_mut().set_workspace("Default");
    harness.run_steps(4);
    harness.state_mut().set_workspace("Notes Desk");
    let kind = PanelKind::plugin_instance(PROBE, "notes", "w1");
    let tab = format!("panel.tab.{}", kind.id());
    until(&mut harness, "the note's tab back with its title", |h| h.state().auto.find(&tab).is_some_and(|tab| tab.label == "Notes w1"));
    note_shown(&mut harness, "w1");
    type_note(&mut harness, "w1", " and tea");
    until(&mut harness, "the note taking more text", |h| shows(h, "note w1: milk and tea"));
}
