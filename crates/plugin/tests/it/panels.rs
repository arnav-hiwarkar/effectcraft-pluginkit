use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_plugin_host::viewer::ToolEventKind;
use effectcraft_engine::project::{Layer, Property, Value as PropValue};
use effectcraft_engine::render::{EvalCtx, ExprHost};
use effectcraft_plugin::ui::{Key, Modifiers, Shape, UiEvent, UiEventKind, Widget};
use effectcraft_plugin::{Panels, Surface, Target};
use serde_json::{Value, json};

use crate::{App, app};

const SIZE: [f32; 2] = [300.0, 200.0];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub(crate) fn settled(app: &mut App, panels: &mut Panels, surface: Surface, frame: &mut u64) -> Vec<Widget> {
    for _ in 0..5000 {
        *frame += 1;
        panels.frame(&mut app.session, ms(16 * *frame));
        let shown = panels.show(&mut app.session, surface, SIZE).expect("the probe surface shows");
        assert_eq!(shown.error, None);
        let drawn = shown.widgets.to_vec();
        if !panels.busy(&surface.owner()) {
            return drawn;
        }
        std::thread::sleep(ms(2));
    }
    panic!("the probe plugin never answered")
}

fn show(app: &mut App, panels: &mut Panels, frame: &mut u64) -> Vec<Widget> {
    let id = app.id.clone();
    settled(app, panels, Surface::Panel { plugin: &id, panel: "watch", instance: None }, frame)
}

fn watch(app: &App) -> effectcraft_plugin::Owner {
    effectcraft_plugin::Owner::panel(&app.id, "watch", None)
}

fn relay_owner(app: &App) -> effectcraft_plugin::Owner {
    effectcraft_plugin::Owner::panel(&app.id, "relay", None)
}

fn lines(widgets: &[Widget]) -> Vec<String> {
    widgets
        .iter()
        .filter_map(|widget| match widget {
            Widget::Label { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn click(id: u64) -> UiEvent {
    UiEvent { id, kind: UiEventKind::Click }
}

fn button(widgets: &[Widget], at: usize) -> u64 {
    match &widgets[at] {
        Widget::Button { id, .. } => *id,
        other => panic!("a button expected, not {other:?}"),
    }
}

pub(crate) fn hold(app: &App) {
    hold_in(app.bundle.path());
}

fn hold_in(bundle: &std::path::Path) {
    std::fs::write(bundle.join("hold"), "").expect("write the hold file");
}

pub(crate) fn release(app: &App) {
    std::fs::remove_file(app.bundle.path().join("hold")).expect("remove the hold file");
}

pub(crate) fn relay(app: &mut App, panels: &mut Panels, frame: &mut u64, relayed: Value, held: bool) {
    let (id, bundle) = (app.id.clone(), app.bundle.path().to_path_buf());
    relay_in(app, panels, frame, (&id, &bundle), relayed, held);
}

fn relay_in(app: &mut App, panels: &mut Panels, frame: &mut u64, (plugin, bundle): (&str, &std::path::Path), relayed: Value, held: bool) {
    let surface = Surface::Panel { plugin, panel: "relay", instance: None };
    let relay = button(&settled(app, panels, surface, frame), 0);
    std::fs::write(bundle.join("relay.json"), relayed.to_string()).expect("write the relayed request");
    if held {
        hold_in(bundle);
    }
    let before = held_notices(app);
    panels.submit(&surface.owner(), vec![click(relay)], "");
    *frame += 1;
    panels.frame(&mut app.session, ms(16 * *frame));
    panels.show(&mut app.session, surface, SIZE).expect("the relay panel shows");
    for _ in 0..5000 {
        if !held || held_notices(app) > before {
            break;
        }
        *frame += 1;
        panels.frame(&mut app.session, ms(16 * *frame));
        std::thread::sleep(ms(2));
    }
    assert!(!held || held_notices(app) > before, "the relay call never started waiting on the hold");
    assert!(!held || panels.busy(&surface.owner()), "the relay call waits while the bundle holds");
}

fn held_notices(app: &App) -> usize {
    app.session.events.iter().filter(|event| matches!(event, effectcraft_engine::Event::Toast { message, .. } if message.ends_with(": held"))).count()
}

fn relay_said(app: &mut App, panels: &mut Panels, frame: &mut u64) -> String {
    let id = app.id.clone();
    lines(&settled(app, panels, Surface::Panel { plugin: &id, panel: "relay", instance: None }, frame)).join("")
}

#[test]
fn a_plugin_built_before_ui_levels_still_gets_its_panel_clicks_and_its_tool_events() {
    let mut app = app("test.panels-before-levels-host", &[]);
    let bundle = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/legacy/probe-before-regions.wasm");
    std::fs::copy(fixture, bundle.path().join("probe.wasm")).unwrap();
    let permissions = ["document.read", "document.write"];
    let manifest = json!({"id": "test.before-levels", "name": "Old Probe", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": permissions});
    std::fs::write(bundle.path().join("plugin.json"), manifest.to_string()).unwrap();
    app.plugins.load_for_session(bundle.path()).expect("a plugin built before UI levels loads");
    let (mut panels, mut frame) = (Panels::default(), 0);
    let surface = Surface::Panel { plugin: "test.before-levels", panel: "counter", instance: None };
    let count = button(&settled(&mut app, &mut panels, surface, &mut frame), 0);
    panels.submit(&surface.owner(), vec![click(count)], "");
    assert_eq!(lines(&settled(&mut app, &mut panels, surface, &mut frame)), ["1 clicks at speed 0"]);
    panels.tool(&app.session, "test.before-levels", "stamp", crate::viewer::event(ToolEventKind::Down, [4.0, 5.0])).unwrap();
    crate::viewer::settle(&mut app, &mut panels);
    assert_eq!(app.layer_names(), ["Stamp"]);
}

#[test]
fn each_instance_of_a_panel_each_is_called_with_its_instance_and_keeps_its_own_text() {
    let mut app = app("test.panels-notes", &[]);
    let (mut panels, mut frame, id) = (Panels::default(), 0, app.id.clone());
    let type_into = |app: &mut App, panels: &mut Panels, frame: &mut u64, instance: &str, text: &str| {
        let surface = Surface::Panel { plugin: &id, panel: "notes", instance: Some(instance) };
        let Widget::TextEdit { id: field, .. } = settled(app, panels, surface, frame)[0] else { panic!("the notes panel starts with its text field") };
        panels.submit(&surface.owner(), vec![UiEvent { id: field, kind: UiEventKind::Text(text.into()) }], "");
        lines(&settled(app, panels, surface, frame))
    };
    assert_eq!(type_into(&mut app, &mut panels, &mut frame, "1", "milk"), ["note 1: milk"]);
    assert_eq!(type_into(&mut app, &mut panels, &mut frame, "2", "eggs"), ["note 2: eggs"]);
    assert_eq!(type_into(&mut app, &mut panels, &mut frame, "1", "milk and tea"), ["note 1: milk and tea"]);
    let one = Surface::Panel { plugin: &id, panel: "notes", instance: Some("1") };
    panels.close(&app.session, one);
    assert_eq!(lines(&settled(&mut app, &mut panels, one, &mut frame)), ["note 1: "], "a closed instance starts again empty");
    assert_eq!(type_into(&mut app, &mut panels, &mut frame, "1", "bread"), ["note 1: bread"]);
    panels.close(&app.session, one);
    assert_eq!(lines(&settled(&mut app, &mut panels, one, &mut frame)), ["note 1: "], "an instance closed a second time starts again empty");
    assert_eq!(lines(&settled(&mut app, &mut panels, Surface::Panel { plugin: &id, panel: "notes", instance: Some("2") }, &mut frame)), ["note 2: eggs"]);
}

#[test]
fn a_panel_gets_each_app_event_it_subscribed_to_once_with_its_contents_and_key_events_on_its_canvas() {
    let mut app = app("test.panels-watch", &[]);
    let mut panels = Panels::default();
    let mut frame = 0;
    let first = show(&mut app, &mut panels, &mut frame);
    assert_eq!(lines(&first), Vec::<String>::new());
    let comp = app.session.active_comp_id().expect("a comp").0;

    app.session.execute("layer.newNull", json!({"name": "Null"})).expect("layer.newNull");
    let layer = app.session.active_comp().and_then(|comp| comp.layers.first().map(|layer| layer.id.0)).expect("a layer");
    show(&mut app, &mut panels, &mut frame);
    app.session.execute("time.set", json!({"time": 1.0})).expect("time.set");
    show(&mut app, &mut panels, &mut frame);
    app.session.execute("layer.select", json!({"layers": []})).expect("layer.select");
    show(&mut app, &mut panels, &mut frame);
    for _ in 0..5 {
        show(&mut app, &mut panels, &mut frame);
    }
    let Widget::Canvas { id, .. } = first[0] else { panic!("canvas expected, got {first:?}") };
    let shift = Modifiers { shift: true, alt: false, command: false };
    panels.submit(&watch(&app), vec![UiEvent { id, kind: UiEventKind::Key(Key { name: "Left".into(), modifiers: shift }) }], "");
    let last = show(&mut app, &mut panels, &mut frame);
    assert_eq!(
        lines(&last),
        vec![
            format!(r#"{{"topic":"selection","layers":[{layer}],"properties":[],"keyframes":0}}"#),
            format!(r#"{{"topic":"edit","layers":[{layer}]}}"#),
            format!(r#"{{"topic":"time","comp":{comp},"time":1.0}}"#),
            r#"{"topic":"selection","layers":[],"properties":[],"keyframes":0}"#.to_string(),
            "key Left shift true".to_string(),
        ]
    );
    assert_eq!(panels.calls(&watch(&app)), 5);

    app.session.execute("comp.new", json!({"name": "Second", "width": 32, "height": 18, "frameRate": 24, "duration": 1})).expect("comp.new");
    let second = app.session.active_comp_id().expect("the new comp").0;
    let after_comp = lines(&show(&mut app, &mut panels, &mut frame));
    app.session.execute("file.newProject", json!({})).expect("file.newProject");
    let after_project = lines(&show(&mut app, &mut panels, &mut frame));
    assert_eq!(
        after_comp[5..],
        [
            format!(r#"{{"topic":"time","comp":{second},"time":0.0}}"#),
            format!(r#"{{"topic":"comp","comp":{second}}}"#),
            r#"{"topic":"edit","layers":[]}"#.to_string()
        ]
    );
    assert_eq!(after_project[8..], [r#"{"topic":"comp","comp":null}"#.to_string(), r#"{"topic":"project","path":null}"#.to_string()]);
}

#[test]
fn a_panel_call_waiting_for_the_app_and_an_overlay_posted_meanwhile_both_finish_before_a_deadline() {
    let (finished, outcome) = channel();
    std::thread::spawn(move || {
        let mut app = app("test.panels-deadlock", &["document.read"]);
        crate::viewer::aim_layer(&mut app);
        let (mut panels, mut frame) = (Panels::default(), 0);
        relay(&mut app, &mut panels, &mut frame, json!({"command": "comp.info", "params": {}}), true);
        crate::viewer::try_shown(&mut app, &mut panels);
        release(&app);
        let said = relay_said(&mut app, &mut panels, &mut frame);
        crate::viewer::settle(&mut app, &mut panels);
        let handles = crate::viewer::try_shown(&mut app, &mut panels).map(|shown| shown.overlay.handles.len());
        let _ = finished.send((said, handles));
    });
    let outcome = outcome.recv_timeout(Duration::from_secs(60)).expect("the panel call and the overlay finish within 60 s");
    assert_eq!(outcome, ("relayed".to_string(), Some(7)));
}

#[test]
fn an_edit_a_panel_asks_for_after_its_project_closed_is_dropped() {
    let mut app = app("test.panels-closed", &["document.read", "document.write"]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"command": "layer.newNull", "params": {"name": "From Panel"}}), true);
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    release(&app);
    let said = relay_said(&mut app, &mut panels, &mut frame);
    assert_eq!(app.layer_names(), Vec::<String>::new());
    assert_eq!(said, "refused: the app dropped this request because its project closed");
}

#[test]
fn a_click_queued_behind_a_busy_panel_call_is_dropped_when_another_project_opens() {
    let mut app = app("test.panels-queued-closed", &["document.read", "document.write"]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"command": "layer.newNull", "params": {"name": "From Panel"}}), true);
    let id = app.id.clone();
    let surface = Surface::Panel { plugin: &id, panel: "relay", instance: None };
    let relay = button(panels.show(&mut app.session, surface, SIZE).unwrap().widgets, 0);
    panels.submit(&surface.owner(), vec![click(relay)], "");
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    release(&app);
    for _ in 0..20 {
        relay_said(&mut app, &mut panels, &mut frame);
    }
    assert_eq!(app.layer_names(), Vec::<String>::new());
}

#[test]
fn a_panel_that_asks_to_close_a_status_item_is_told_it_cannot() {
    let mut app = app("test.panels-close-status", &[]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"close_panel": "status"}), false);
    let said = relay_said(&mut app, &mut panels, &mut frame);
    assert!(said.contains("cannot be opened or closed"), "{said}");
    assert!(!app.session.events.iter().any(|event| matches!(event, effectcraft_engine::Event::Frontend { command, .. } if command == "window.pluginPanel")));
}

#[test]
fn a_reload_during_a_panel_call_drops_the_old_calls_edit_and_shows_the_new_load() {
    let mut app = app("test.panels-reloaded", &["document.read", "document.write"]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"command": "layer.newNull", "params": {"name": "From Panel"}}), true);
    assert_eq!(panels.calls(&relay_owner(&app)), 2);
    app.plugins.load_for_session(app.bundle.path()).unwrap();
    let id = app.id.clone();
    frame += 1;
    panels.frame(&mut app.session, ms(16 * frame));
    panels.show(&mut app.session, Surface::Panel { plugin: &id, panel: "relay", instance: None }, SIZE).unwrap();
    assert_eq!(panels.calls(&relay_owner(&app)), 1, "the reloaded plugin gets a slot of its own at once");
    release(&app);
    assert_eq!(relay_said(&mut app, &mut panels, &mut frame), "");
    app.run("greet").unwrap();
    assert_eq!(app.layer_names(), Vec::<String>::new());
}

#[test]
fn a_task_a_panel_starts_belongs_to_the_project_the_panel_was_called_for() {
    let mut app = app("test.panels-task", &["document.read", "document.write"]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"task": "edit"}), true);
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    release(&app);
    relay_said(&mut app, &mut panels, &mut frame);
    assert!(effectcraft_plugin::wait_for_tasks(&mut app.session, 20.0), "the task ends");
    assert_eq!(app.layer_names(), Vec::<String>::new());
}

#[derive(Default)]
struct Witness {
    threads: Mutex<Vec<std::thread::ThreadId>>,
}

impl ExprHost for Witness {
    fn eval(&self, _ctx: &EvalCtx, _layer: &Layer, _prop: &Property, value: &PropValue) -> Result<PropValue, String> {
        self.threads.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(std::thread::current().id());
        Ok(value.clone())
    }
}

#[test]
fn a_frame_a_panel_asks_for_renders_off_the_ui_thread() {
    let mut app = app("test.panels-frame", &["document.read"]);
    app.session.execute("layer.newSolid", json!({"name": "Lit", "color": "#ffffff"})).unwrap();
    app.session.execute("prop.setExpression", json!({"layer": "Lit", "path": "transform/opacity", "expression": "value"})).unwrap();
    let witness = Arc::new(Witness::default());
    app.session.expr = Some(witness.clone());
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"frame": {}}), false);
    assert_eq!(relay_said(&mut app, &mut panels, &mut frame), "64x36");
    let threads = witness.threads.lock().unwrap().clone();
    let here = std::thread::current().id();
    assert_eq!((threads.is_empty(), threads.iter().filter(|thread| **thread == here).count()), (false, 0));
}

#[test]
fn a_panel_shown_again_after_it_was_hidden_wakes_the_app_for_its_timer() {
    let mut app = app("test.panels-reopen", &[]);
    let mut panels = Panels::default();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = wakes.clone();
    panels.set_waker(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    let id = app.id.clone();
    let surface = Surface::Panel { plugin: &id, panel: "ticker", instance: None };
    let mut frame = 0;
    settled(&mut app, &mut panels, surface, &mut frame);
    frame += 2;
    panels.frame(&mut app.session, ms(16 * (frame - 1)));
    assert_eq!(panels.frame(&mut app.session, ms(16 * frame)), None, "a hidden panel's timer is not scheduled");
    wakes.store(0, Ordering::SeqCst);
    panels.show(&mut app.session, surface, SIZE).unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "showing the panel again wakes the app");
    frame += 1;
    assert_eq!(panels.frame(&mut app.session, ms(16 * frame)), Some(ms(84)));
}

fn scale_layer(app: &mut App) -> Target {
    app.session.execute("layer.newSolid", json!({"name": "Scaled", "color": "#ffffff", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Scaled", "effect": format!("{}.scale", app.id)})).unwrap();
    let found = app.session.execute("prop.get", json!({"layer": "Scaled", "path": "effects/#1/level"})).unwrap();
    let comp = app.session.active_comp_id().unwrap().0;
    Target { comp, layer: found["layer"].as_u64().unwrap(), prop: found["uid"].as_u64().unwrap(), time: 0.0, merge: "halve".into() }
}

fn level(app: &mut App) -> Value {
    app.session.execute("prop.get", json!({"layer": "Scaled", "path": "effects/#1/level"})).unwrap()
}

fn halve_held(app: &mut App, panels: &mut Panels, target: &Target, frame: &mut u64) {
    let effect = format!("{}.scale", app.id);
    let surface = Surface::Control { effect: &effect, param: "level", instance: target.prop, value: "1", edit: Some(target) };
    let halve = button(&settled(app, panels, surface, frame), 2);
    hold(app);
    panels.submit(&surface.owner(), vec![click(halve)], "");
    *frame += 1;
    panels.frame(&mut app.session, ms(16 * *frame));
    panels.show(&mut app.session, surface, SIZE).unwrap();
    assert!(panels.busy(&surface.owner()), "the control call waits while the bundle holds");
}

#[test]
fn a_control_edit_that_arrives_after_its_project_closed_is_not_applied_to_the_next_project() {
    let mut app = app("test.panels-control-closed", &[]);
    let target = scale_layer(&mut app);
    let (mut panels, mut frame) = (Panels::default(), 0);
    halve_held(&mut app, &mut panels, &target, &mut frame);
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    assert_eq!(scale_layer(&mut app), target, "the next project has the same comp, layer and property ids");
    release(&app);
    crate::viewer::settle(&mut app, &mut panels);
    assert_eq!(level(&mut app)["value"], json!("1"));
}

#[test]
fn a_control_edit_that_arrives_after_effect_controls_closed_is_not_applied() {
    let mut app = app("test.panels-control-hidden", &[]);
    let target = scale_layer(&mut app);
    let steps = app.session.history.undo.len();
    let (mut panels, mut frame) = (Panels::default(), 0);
    halve_held(&mut app, &mut panels, &target, &mut frame);
    for _ in 0..2 {
        frame += 1;
        panels.frame(&mut app.session, ms(16 * frame));
    }
    release(&app);
    crate::viewer::settle(&mut app, &mut panels);
    assert_eq!(level(&mut app)["value"], json!("1"));
    assert_eq!(app.session.history.undo.len(), steps, "no undo step");
}

#[test]
fn a_control_edit_keys_the_time_it_was_made_at_even_when_the_playhead_moved_meanwhile() {
    let mut app = app("test.panels-control-time", &[]);
    let target = scale_layer(&mut app);
    for time in [0.0, 1.0] {
        app.session.execute("prop.addKey", json!({"layer": "Scaled", "path": "effects/#1/level", "time": time, "value": "1"})).unwrap();
    }
    let (mut panels, mut frame) = (Panels::default(), 0);
    halve_held(&mut app, &mut panels, &target, &mut frame);
    app.session.execute("time.set", json!({"time": 1.0})).unwrap();
    release(&app);
    let effect = format!("{}.scale", app.id);
    let surface = Surface::Control { effect: &effect, param: "level", instance: target.prop, value: "1", edit: Some(&target) };
    settled(&mut app, &mut panels, surface, &mut frame);
    let keys: Vec<Value> = level(&mut app)["keys"].as_array().unwrap().iter().map(|key| key["value"].clone()).collect();
    assert_eq!(keys, vec![json!("0.5"), json!("1")]);
}

#[test]
fn a_menu_command_behind_a_panel_call_that_does_not_return_leaves_the_ui_at_once_and_runs_after_it() {
    let (finished, outcome) = channel();
    std::thread::spawn(move || {
        let mut app = app("test.panels-menu", &["document.read", "document.write"]);
        let (mut panels, mut frame) = (Panels::default(), 0);
        relay(&mut app, &mut panels, &mut frame, json!({"command": "comp.info", "params": {}}), true);
        let posted = panels.command(&app.session, &app.id, "edit");
        let before = app.layer_names();
        release(&app);
        crate::viewer::settle(&mut app, &mut panels);
        let _ = finished.send((posted, before, app.layer_names()));
    });
    let outcome = outcome.recv_timeout(Duration::from_secs(20)).expect("the menu returns while the panel call holds");
    assert_eq!(outcome, (Ok(()), Vec::<String>::new(), vec!["From Plugin".to_string()]));
}

#[test]
fn a_worker_waiting_for_the_app_is_released_when_its_panels_and_session_close() {
    let (finished, outcome) = channel();
    std::thread::spawn(move || {
        let mut app = app("test.panels-strand", &["document.read"]);
        let (mut panels, mut frame) = (Panels::default(), 0);
        relay(&mut app, &mut panels, &mut frame, json!({"command": "comp.info", "params": {}}), true);
        std::thread::sleep(ms(50));
        drop(panels);
        app.session = effectcraft_host::session();
        effectcraft_plugin::install(&mut app.session, None);
        release(&app);
        let _ = finished.send(app.run("greet"));
    });
    assert_eq!(outcome.recv_timeout(Duration::from_secs(20)).expect("the plugin is free again"), Ok(()));
}

#[test]
fn a_command_run_while_a_panel_call_waits_for_the_app_runs_after_it() {
    let (finished, outcome) = channel();
    std::thread::spawn(move || {
        let mut app = app("test.panels-run-during", &["document.read"]);
        let (mut panels, mut frame) = (Panels::default(), 0);
        relay(&mut app, &mut panels, &mut frame, json!({"command": "comp.info", "params": {}}), true);
        let hold = app.bundle.path().join("hold");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(ms(200));
            std::fs::remove_file(hold).expect("remove the hold file");
        });
        let ran = app.run("greet");
        let _ = releaser.join();
        let _ = finished.send((ran, relay_said(&mut app, &mut panels, &mut frame)));
    });
    assert_eq!(outcome.recv_timeout(Duration::from_secs(20)).expect("the command runs"), (Ok(()), "relayed".to_string()));
}

#[test]
fn two_clicks_on_a_control_answered_in_one_frame_each_start_from_the_value_the_other_left() {
    let mut app = app("test.panels-control-twice", &[]);
    let target = scale_layer(&mut app);
    let effect = format!("{}.scale", app.id);
    let (mut panels, mut frame) = (Panels::default(), 0);
    let surface = Surface::Control { effect: &effect, param: "level", instance: target.prop, value: "1", edit: Some(&target) };
    let halve = button(&settled(&mut app, &mut panels, surface, &mut frame), 2);
    panels.submit(&surface.owner(), vec![click(halve), click(halve)], "");
    for _ in 0..200 {
        frame += 1;
        panels.frame(&mut app.session, ms(16 * frame));
        let value = level(&mut app)["value"].as_str().unwrap_or_default().to_string();
        let surface = Surface::Control { effect: &effect, param: "level", instance: target.prop, value: &value, edit: Some(&target) };
        panels.show(&mut app.session, surface, SIZE).unwrap();
        std::thread::sleep(ms(2));
    }
    assert_eq!(level(&mut app)["value"], json!("0.25"));
}

fn toasted(app: &App, said: &str) -> bool {
    app.session.events.iter().any(|event| matches!(event, effectcraft_engine::Event::Toast { message, .. } if message.ends_with(said)))
}

fn stamped(panels: &mut Panels, app: &mut App, said: &str) -> bool {
    panels
        .tool_overlay(&mut app.session)
        .is_some_and(|overlay| overlay.marks.iter().any(|mark| matches!(&mark.shape, Shape::Text { text, .. } if text == said)))
}

#[test]
fn a_held_panel_call_lets_wasm_overlay_and_tool_calls_pass_but_holds_every_call_to_a_helper_process() {
    let mut app = super::app_exclusive("test.panels-lanes", &["document.read"]);
    let native = crate::render::native_probe(&app).to_string();
    let mut arrivals = Vec::new();
    for (plugin, sandboxed) in [(app.id.clone(), true), (native, false)] {
        let bundle = app.plugins.get(&plugin).expect("the probe runs").bundle.dir.clone();
        let (mut panels, mut frame) = (Panels::default(), 0);
        app.session.execute("comp.new", json!({"name": plugin, "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
        app.session.execute("layer.newSolid", json!({"name": "Aim", "color": "#204060", "width": 64, "height": 36})).unwrap();
        app.session.execute("effect.apply", json!({"layer": "Aim", "effect": format!("{plugin}.aim")})).unwrap();
        app.session.events.clear();
        relay_in(&mut app, &mut panels, &mut frame, (&plugin, &bundle), json!({"command": "comp.info", "params": {}}), true);
        let hold = bundle.join("hold");
        let start = Instant::now();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(ms(1000));
            std::fs::remove_file(hold).expect("remove the hold file");
        });
        panels.tool(&app.session, &plugin, "stamp", crate::viewer::event(ToolEventKind::Key, [1.0, 2.0])).unwrap();
        panels.command(&app.session, &plugin, "greet").unwrap();
        let (mut overlay, mut tool, mut command) = (None, None, None);
        while start.elapsed() < Duration::from_secs(10) && [overlay, tool, command].contains(&None) {
            frame += 1;
            panels.frame(&mut app.session, ms(16 * frame));
            if overlay.is_none() && crate::viewer::try_shown(&mut app, &mut panels).is_some() {
                overlay = Some(start.elapsed());
            }
            if tool.is_none() && stamped(&mut panels, &mut app, "Key 1.0 2.0") {
                tool = Some(start.elapsed());
            }
            if command.is_none() && toasted(&app, "3 layers renamed") {
                command = Some(start.elapsed());
            }
            std::thread::sleep(ms(1));
        }
        releaser.join().unwrap();
        crate::viewer::settle(&mut app, &mut panels);
        eprintln!("lanes {plugin}: overlay {overlay:?} tool {tool:?} command {command:?}");
        arrivals.push((plugin, sandboxed, [overlay, tool, command].map(|arrived| arrived.expect("every call answers within 10 s"))));
    }
    for (plugin, sandboxed, [overlay, tool, command]) in arrivals {
        let quick = |arrived: Duration| arrived < ms(200);
        assert_eq!(
            (quick(overlay), quick(tool)),
            (sandboxed, sandboxed),
            "{plugin}: a wasm plugin's lanes pass a held panel call and a helper serves one call at a time; overlay {overlay:?} tool {tool:?}"
        );
        assert!(command >= ms(1000), "{plugin}: a command waits for the held panel call; {command:?}");
    }
}

fn opacity(app: &mut App) -> Option<f64> {
    app.session.execute("prop.get", json!({"layer": "#1", "path": "transform/opacity"})).unwrap()["value"].as_f64()
}

fn widget_id(widgets: &[Widget], text: &str) -> u64 {
    widgets
        .iter()
        .find_map(|widget| match widget {
            Widget::Slider { id, text: shown, .. } | Widget::Checkbox { id, text: shown, .. } if shown == text => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the app panel shows no `{text}`"))
}

#[test]
fn a_slider_dragged_over_five_panel_calls_that_each_set_a_property_is_one_undo_step_and_one_undo_restores_the_start() {
    let mut app = app("test.panels-drag", &["document.read", "document.write"]);
    app.session.execute("layer.newSolid", json!({"name": "Dragged", "color": "#ffffff", "width": 64, "height": 36})).unwrap();
    let (mut panels, mut frame, id) = (Panels::default(), 0, app.id.clone());
    let surface = Surface::Panel { plugin: &id, panel: "app", instance: None };
    let slider = widget_id(&settled(&mut app, &mut panels, surface, &mut frame), "Opacity");
    let (start, steps) = (opacity(&mut app), app.session.history.undo.len());
    let mut drag = |app: &mut App, panels: &mut Panels, value: f64| {
        panels.submit(&surface.owner(), vec![UiEvent { id: slider, kind: UiEventKind::Number(value) }], "");
        settled(app, panels, surface, &mut frame);
    };
    panels.hold(&surface.owner(), true);
    for value in [10.0, 20.0, 30.0, 40.0] {
        drag(&mut app, &mut panels, value);
    }
    panels.hold(&surface.owner(), false);
    drag(&mut app, &mut panels, 50.0);
    assert_eq!((app.session.history.undo.len() - steps, opacity(&mut app)), (1, Some(50.0)));
    drag(&mut app, &mut panels, 70.0);
    assert_eq!((app.session.history.undo.len() - steps, opacity(&mut app)), (2, Some(70.0)));
    for _ in 0..2 {
        app.session.execute("edit.undo", json!({})).unwrap();
    }
    assert_eq!((opacity(&mut app), start), (Some(100.0), Some(100.0)));
}

#[test]
fn two_drags_queued_behind_a_slow_panel_call_stay_two_undo_steps() {
    let mut app = app("test.panels-two-drags", &["document.read", "document.write"]);
    app.session.execute("layer.newSolid", json!({"name": "Dragged", "color": "#ffffff", "width": 64, "height": 36})).unwrap();
    let (mut panels, mut frame, id) = (Panels::default(), 0, app.id.clone());
    let surface = Surface::Panel { plugin: &id, panel: "app", instance: None };
    let slider = widget_id(&settled(&mut app, &mut panels, surface, &mut frame), "Opacity");
    let owner = surface.owner();
    let value = |value: f64| vec![UiEvent { id: slider, kind: UiEventKind::Number(value) }];
    hold(&app);
    let before = held_notices(&app);
    panels.hold(&owner, true);
    panels.submit(&owner, value(10.0), "");
    for _ in 0..5000 {
        frame += 1;
        panels.frame(&mut app.session, ms(16 * frame));
        panels.show(&mut app.session, surface, SIZE).unwrap();
        if held_notices(&app) > before {
            break;
        }
        std::thread::sleep(ms(2));
    }
    assert!(panels.busy(&owner), "the first drag's call waits while the bundle holds");
    panels.submit(&owner, value(20.0), "");
    panels.hold(&owner, false);
    panels.hold(&owner, true);
    panels.submit(&owner, value(60.0), "");
    panels.submit(&owner, value(70.0), "");
    panels.hold(&owner, false);
    release(&app);
    for _ in 0..3 {
        settled(&mut app, &mut panels, surface, &mut frame);
    }
    assert_eq!(opacity(&mut app), Some(70.0));
    app.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(opacity(&mut app), Some(20.0), "the first undo restores the end of the first drag");
    app.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(opacity(&mut app), Some(100.0), "the second undo restores the start");
}

#[test]
fn a_command_its_plugin_turned_off_is_marked_off_and_refused_by_name_until_turned_on_again() {
    let mut app = app("test.panels-state", &[]);
    let (mut panels, mut frame, id) = (Panels::default(), 0, app.id.clone());
    let surface = Surface::Panel { plugin: &id, panel: "app", instance: None };
    let disable = widget_id(&settled(&mut app, &mut panels, surface, &mut frame), "Disable Probe Action");
    let mut set = |app: &mut App, panels: &mut Panels, off: bool| {
        panels.submit(&surface.owner(), vec![UiEvent { id: disable, kind: UiEventKind::Bool(off) }], "");
        settled(app, panels, surface, &mut frame);
        let item = app.session.plugin_items.iter().find(|item| item.plugin == id && item.id == "app-act").cloned().unwrap_or_default();
        (item.disabled, item.checked)
    };
    assert_eq!(set(&mut app, &mut panels, true), (true, Some(false)));
    let listed = app.session.execute("plugin.list", json!({})).unwrap();
    let plugin = listed.as_array().and_then(|all| all.iter().find(|plugin| plugin["id"] == id.as_str())).cloned().unwrap_or_default();
    let command = plugin["commands"].as_array().and_then(|all| all.iter().find(|command| command["id"] == "app-act")).cloned();
    assert_eq!(
        command,
        Some(json!({"id": "app-act", "title": "Probe Action", "enabled": false, "checked": false})),
        "automation sees the state the menus show"
    );
    assert_eq!(app.run("app-act"), Err("Probe turned its command `Probe Action` off for now, so it did not run".into()));
    assert_eq!(panels.command(&app.session, &id, "app-act"), Err("Probe turned its command `Probe Action` off for now, so it did not run".into()));
    assert_eq!(set(&mut app, &mut panels, false), (false, Some(true)));
    assert_eq!(app.run("app-act"), Ok(()));
}

#[test]
fn showing_a_panel_in_a_session_with_no_window_says_there_is_no_window() {
    let mut app = app("test.panels-headless-show", &[]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    let id = app.id.clone();
    let surface = Surface::Panel { plugin: &id, panel: "files", instance: None };
    let widgets = settled(&mut app, &mut panels, surface, &mut frame);
    let Some(Widget::List { id: list, .. }) = widgets.first() else { panic!("the files panel starts with a list, not {widgets:?}") };
    let activate = UiEvent { id: *list, kind: UiEventKind::List(effectcraft_plugin::ui::ListEvent::Activate(3)) };
    panels.submit(&surface.owner(), vec![activate], "");
    let said = lines(&settled(&mut app, &mut panels, surface, &mut frame));
    assert!(said.iter().any(|line| line.contains("there is no app window to show panel `inspector` in")), "{said:?}");
    assert!(!app.session.events.iter().any(|event| matches!(event, effectcraft_engine::Event::Frontend { command, .. } if command == "window.pluginPanel")));
}
