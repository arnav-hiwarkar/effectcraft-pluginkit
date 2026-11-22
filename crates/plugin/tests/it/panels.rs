use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use effectcraft_engine::project::{Layer, Property, Value as PropValue};
use effectcraft_engine::render::{EvalCtx, ExprHost};
use effectcraft_plugin::ui::{Key, Modifiers, UiEvent, UiEventKind, Widget};
use effectcraft_plugin::{Panels, Surface, Target};
use serde_json::{Value, json};

use crate::{App, app};

const KEY: &str = "plugin.test.panels-watch.watch";
const RELAY: &str = "plugin.test.relay";
const SIZE: [f32; 2] = [300.0, 200.0];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub(crate) fn settled(app: &mut App, panels: &mut Panels, key: &str, surface: Surface, frame: &mut u64) -> Vec<Widget> {
    for _ in 0..5000 {
        *frame += 1;
        panels.frame(&mut app.session, ms(16 * *frame));
        let shown = panels.show(&mut app.session, key, surface, SIZE).expect("the probe surface shows");
        assert_eq!(shown.error, None);
        let drawn = shown.widgets.to_vec();
        if !panels.busy(key) {
            return drawn;
        }
        std::thread::sleep(ms(2));
    }
    panic!("the probe plugin never answered")
}

fn show(app: &mut App, panels: &mut Panels, frame: &mut u64) -> Vec<Widget> {
    let id = app.id.clone();
    settled(app, panels, KEY, Surface::Panel { plugin: &id, panel: "watch" }, frame)
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

fn hold(app: &App) {
    std::fs::write(app.bundle.path().join("hold"), "").expect("write the hold file");
}

pub(crate) fn release(app: &App) {
    std::fs::remove_file(app.bundle.path().join("hold")).expect("remove the hold file");
}

pub(crate) fn relay(app: &mut App, panels: &mut Panels, frame: &mut u64, relayed: Value, held: bool) {
    let id = app.id.clone();
    let surface = Surface::Panel { plugin: &id, panel: "relay" };
    let relay = button(&settled(app, panels, RELAY, surface, frame), 0);
    std::fs::write(app.bundle.path().join("relay.json"), relayed.to_string()).expect("write the relayed request");
    if held {
        hold(app);
    }
    panels.submit(RELAY, vec![click(relay)], "");
    *frame += 1;
    panels.frame(&mut app.session, ms(16 * *frame));
    panels.show(&mut app.session, RELAY, surface, SIZE).expect("the relay panel shows");
    assert!(!held || panels.busy(RELAY), "the relay call waits while the bundle holds");
}

fn relay_said(app: &mut App, panels: &mut Panels, frame: &mut u64) -> String {
    let id = app.id.clone();
    lines(&settled(app, panels, RELAY, Surface::Panel { plugin: &id, panel: "relay" }, frame)).join("")
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
    panels.submit(KEY, vec![UiEvent { id, kind: UiEventKind::Key(Key { name: "Left".into(), modifiers: shift }) }], "");
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
    assert_eq!(panels.calls(KEY), 5);

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
        let early = crate::viewer::try_shown(&mut app, &mut panels);
        release(&app);
        let said = relay_said(&mut app, &mut panels, &mut frame);
        crate::viewer::settle(&mut app, &mut panels);
        let handles = crate::viewer::try_shown(&mut app, &mut panels).map(|shown| shown.overlay.handles.len());
        let _ = finished.send((early.is_none(), said, handles));
    });
    let outcome = outcome.recv_timeout(Duration::from_secs(60)).expect("the panel call and the overlay finish within 60 s");
    assert_eq!(outcome, (true, "relayed".to_string(), Some(7)));
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
fn a_reload_during_a_panel_call_drops_the_old_calls_edit_and_shows_the_new_load() {
    let mut app = app("test.panels-reloaded", &["document.read", "document.write"]);
    let (mut panels, mut frame) = (Panels::default(), 0);
    relay(&mut app, &mut panels, &mut frame, json!({"command": "layer.newNull", "params": {"name": "From Panel"}}), true);
    assert_eq!(panels.calls(RELAY), 2);
    app.plugins.load_for_session(app.bundle.path()).unwrap();
    let id = app.id.clone();
    frame += 1;
    panels.frame(&mut app.session, ms(16 * frame));
    panels.show(&mut app.session, RELAY, Surface::Panel { plugin: &id, panel: "relay" }, SIZE).unwrap();
    assert_eq!(panels.calls(RELAY), 1, "the reloaded plugin gets a slot of its own at once");
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
    let surface = Surface::Panel { plugin: &id, panel: "ticker" };
    let mut frame = 0;
    settled(&mut app, &mut panels, "ticker", surface, &mut frame);
    frame += 2;
    panels.frame(&mut app.session, ms(16 * (frame - 1)));
    assert_eq!(panels.frame(&mut app.session, ms(16 * frame)), None, "a hidden panel's timer is not scheduled");
    wakes.store(0, Ordering::SeqCst);
    panels.show(&mut app.session, "ticker", surface, SIZE).unwrap();
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
    let halve = button(&settled(app, panels, "control", surface, frame), 2);
    hold(app);
    panels.submit("control", vec![click(halve)], "");
    *frame += 1;
    panels.frame(&mut app.session, ms(16 * *frame));
    panels.show(&mut app.session, "control", surface, SIZE).unwrap();
    assert!(panels.busy("control"), "the control call waits while the bundle holds");
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
    crate::viewer::settle(&mut app, &mut panels);
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
