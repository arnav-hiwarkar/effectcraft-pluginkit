use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_plugin_host::ui::{Paint, Shape};
use craft_plugin_host::viewer::{Bind, Mark, Space, ToolEvent, ToolEventKind};
use effectcraft_engine::render::EvalCtx;
use effectcraft_engine::time::Tick;
use effectcraft_plugin::Panels;
use effectcraft_plugin::viewer::{Shown, tools};
use serde_json::{Value, json};

use crate::{App, app};

pub(crate) fn aim_layer(app: &mut App) {
    app.session.execute("layer.newSolid", json!({"name": "Aim", "color": "#204060", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Aim", "effect": format!("{}.aim", app.id)})).unwrap();
}

pub(crate) fn settle(app: &mut App, panels: &mut Panels) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !panels.idle() {
        assert!(Instant::now() < deadline, "the plugin did not answer within 20 s");
        panels.frame(&mut app.session, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn try_shown(app: &mut App, panels: &mut Panels) -> Option<Arc<Shown>> {
    let project = app.session.project.clone();
    let comp = project.comp(app.session.active_comp_id()?)?;
    let ctx = EvalCtx::new(&project, app.session.active_comp_id()?, comp, app.session.time_of(app.session.active_comp_id()?));
    let layer = comp.layers.iter().find(|layer| layer.name == "Aim")?;
    panels.overlay(&mut app.session, &ctx, layer, layer.effects()?.groups().next()?)
}

fn shown(app: &mut App, panels: &mut Panels) -> Arc<Shown> {
    try_shown(app, panels);
    settle(app, panels);
    try_shown(app, panels).expect("the effect draws an overlay")
}

fn post_drag(app: &mut App, panels: &mut Panels, shown: &Shown, handle: usize, delta: [f64; 2], merge: &str) {
    let project = app.session.project.clone();
    let layer = project.comp(app.session.active_comp_id().unwrap()).unwrap().layers.iter().find(|layer| layer.name == "Aim").unwrap();
    let commands = panels.drag(shown, handle, delta, layer.effects().unwrap().groups().next().unwrap(), merge).unwrap();
    for (id, params) in commands {
        app.session.execute_checked(&id, params).unwrap();
    }
}

fn drag(app: &mut App, panels: &mut Panels, shown: &Shown, handle: usize, delta: [f64; 2], merge: &str) {
    post_drag(app, panels, shown, handle, delta, merge);
    settle(app, panels);
}

fn value(app: &mut App, param: &str) -> Value {
    app.session.execute_checked("prop.get", json!({"layer": "Aim", "path": format!("effects/#1/{param}")})).unwrap()["value"].clone()
}

#[test]
fn an_effect_overlay_arrives_with_its_shapes_and_handles_in_layer_space_and_is_kept_until_a_value_changes() {
    let mut app = app("test.viewer-overlay", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let first = shown(&mut app, &mut panels);
    let handles: Vec<([f64; 2], &Bind)> = first.overlay.handles.iter().map(|handle| (handle.at, &handle.bind)).collect();
    let point = |param: &str| Bind::Point { param: param.into() };
    assert_eq!(
        handles,
        vec![
            ([32.0, 18.0], &point("centre")),
            ([40.0, 18.0], &Bind::Angle { param: "turn".into(), centre: [32.0, 18.0] }),
            ([52.0, 18.0], &Bind::Radius { param: "reach".into(), centre: [32.0, 18.0], unit: 2.0 }),
            ([24.0, 13.5], &Bind::Rect { min: "Box/min".into(), max: "Box/max".into() }),
            ([16.0, 9.0], &point("Box/min")),
            ([32.0, 18.0], &point("Box/max")),
            ([1.0, 2.0], &Bind::Custom { param: "pin".into(), key: "pin".into() }),
        ]
    );
    assert_eq!(first.overlay.handles[6].name, "Pin");
    let stroke = Some((Paint::Accent, 1.5));
    assert_eq!(
        first.overlay.marks,
        vec![
            Mark { shape: Shape::Circle { center: [32.0, 18.0], radius: 20.0, fill: None, stroke }, space: Space::Layer },
            Mark { shape: Shape::Rect { min: [16.0, 9.0], max: [32.0, 18.0], fill: None, stroke, radius: 0.0 }, space: Space::Layer },
            Mark { shape: Shape::Line { points: vec![[0.0, 0.0], [4.0, 4.0]], color: Paint::Accent, width: 1.5 }, space: Space::Comp },
        ]
    );
    assert!(Arc::ptr_eq(&first, &try_shown(&mut app, &mut panels).unwrap()), "the same values give the retained overlay without a plugin call");
    assert!(panels.idle());
    app.session.execute_checked("prop.set", json!({"layer": "Aim", "path": "effects/#1/reach", "value": 20.0})).unwrap();
    let changed = shown(&mut app, &mut panels);
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.overlay.handles[2].at, [72.0, 18.0]);
}

#[test]
fn an_overlay_of_a_hundred_thousand_shapes_reaches_the_viewer_as_its_first_four_thousand_and_ninety_six() {
    let mut app = app("test.viewer-crowd", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    app.session.execute_checked("prop.set", json!({"layer": "Aim", "path": "effects/#1/crowd", "value": 100000.0})).unwrap();
    let crowded = shown(&mut app, &mut panels);
    assert_eq!(crowded.overlay.marks.len(), 4096);
    let Shape::Circle { center: first, .. } = crowded.overlay.marks[3].shape else { panic!("the fourth shape is the first of the crowd") };
    let Shape::Circle { center: last, .. } = crowded.overlay.marks[4095].shape else { panic!("the last kept shape is a crowd circle") };
    assert_eq!((first, last), ([0.0, 0.0], [4092.0, 0.0]));
    assert_eq!(crowded.overlay.handles.len(), 7);
}

#[test]
fn dragging_each_kind_of_handle_sets_its_parameter_in_one_undo_step() {
    let mut app = app("test.viewer-drag", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let cases: [(usize, [f64; 2], &[(&str, Value)]); 5] = [
        (0, [3.0, -2.0], &[("centre", json!([35.0, 16.0]))]),
        (1, [-8.0, 8.0], &[("turn", json!(90.0))]),
        (2, [-10.0, 0.0], &[("reach", json!(5.0))]),
        (3, [2.0, 1.0], &[("Box/min", json!([18.0, 10.0])), ("Box/max", json!([34.0, 19.0]))]),
        (6, [4.0, 5.0], &[("pin", json!("5,7"))]),
    ];
    for (n, (handle, delta, expected)) in cases.into_iter().enumerate() {
        let before: Vec<Value> = expected.iter().map(|(param, _)| value(&mut app, param)).collect();
        let start = shown(&mut app, &mut panels);
        drag(&mut app, &mut panels, &start, handle, [delta[0] / 2.0, delta[1] / 2.0], &format!("gesture-{n}"));
        drag(&mut app, &mut panels, &start, handle, delta, &format!("gesture-{n}"));
        let after: Vec<(&str, Value)> = expected.iter().map(|(param, _)| (*param, value(&mut app, param))).collect();
        assert_eq!(after, expected.to_vec(), "handle {handle}");
        app.session.execute_checked("edit.undo", json!({})).unwrap();
        let undone: Vec<Value> = expected.iter().map(|(param, _)| value(&mut app, param)).collect();
        assert_eq!(undone, before, "one undo restores handle {handle}");
    }
}

#[test]
fn a_handle_drag_on_a_keyframed_parameter_moves_the_key_at_the_current_time() {
    let mut app = app("test.viewer-keys", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    for (time, at) in [(0.0, [10.0, 10.0]), (1.0, [20.0, 20.0])] {
        app.session.execute_checked("prop.addKey", json!({"layer": "Aim", "path": "effects/#1/centre", "time": time, "value": at})).unwrap();
    }
    let start = shown(&mut app, &mut panels);
    drag(&mut app, &mut panels, &start, 0, [5.0, 0.0], "keyed");
    let keys = app.session.execute_checked("prop.get", json!({"layer": "Aim", "path": "effects/#1/centre"})).unwrap()["keys"].clone();
    let values: Vec<Value> = keys.as_array().unwrap().iter().map(|key| key["value"].clone()).collect();
    assert_eq!(values, vec![json!([15.0, 10.0]), json!([20.0, 20.0])]);
}

fn event(kind: ToolEventKind, comp: [f64; 2]) -> ToolEvent {
    ToolEvent {
        kind,
        comp,
        layer: None,
        layer_id: None,
        modifiers: Default::default(),
        pressure: 1.0,
        scroll: [0.0; 2],
        key: String::new(),
        zoom: 1.0,
        time: 0.0,
    }
}

fn gesture(app: &mut App, panels: &mut Panels, events: &[(ToolEventKind, [f64; 2])]) -> Vec<String> {
    for (kind, at) in events {
        panels.tool(&app.session, &app.id, "stamp", event(*kind, *at)).unwrap();
        settle(app, panels);
    }
    let overlay = panels.tool_overlay(&mut app.session).cloned().unwrap_or_default();
    overlay
        .marks
        .into_iter()
        .filter_map(|mark| match mark.shape {
            Shape::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn undo_labels(app: &App, steps: usize) -> Vec<String> {
    let undo = &app.session.history.undo;
    undo[undo.len().saturating_sub(steps)..].iter().map(|(label, _)| label.clone()).collect()
}

#[test]
fn a_tool_gesture_edits_the_project_through_commands_as_one_undo_step() {
    let mut app = app("test.viewer-tool", &["document.read", "document.write"]);
    let mut panels = Panels::default();
    assert!(tools().iter().any(|(plugin, spec)| *plugin == app.id && spec.id == "stamp" && spec.name == "Stamp"));
    let steps_before = app.session.history.undo.len();
    let events = [(ToolEventKind::Down, [4.0, 5.0]), (ToolEventKind::Move, [6.0, 7.0]), (ToolEventKind::Move, [8.0, 9.0]), (ToolEventKind::Up, [8.0, 9.0])];
    let texts = gesture(&mut app, &mut panels, &events);
    assert_eq!(texts, ["Down 4.0 5.0", "Move 6.0 7.0", "Move 8.0 9.0", "Up 8.0 9.0"]);
    assert_eq!(app.layer_names(), ["Stamp"]);
    let position = app.session.execute_checked("prop.get", json!({"layer": "Stamp", "path": "transform/position"})).unwrap()["value"].clone();
    assert_eq!(position, json!([8.0, 9.0, 0.0]));
    assert_eq!(app.session.history.undo.len(), steps_before + 1);
    assert_eq!(app.session.history.undo.last().map(|(label, _)| label.as_str()), Some("Stamp"));
    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert!(app.layer_names().is_empty(), "one undo removes the whole gesture");
}

#[test]
fn an_edit_made_between_a_tools_press_and_release_stays_its_own_undo_step() {
    let mut app = app("test.viewer-tool-between", &["document.read", "document.write"]);
    let mut panels = Panels::default();
    gesture(&mut app, &mut panels, &[(ToolEventKind::Down, [4.0, 5.0])]);
    app.session.execute_checked("layer.newSolid", json!({"name": "Between", "color": "#ffffff"})).unwrap();
    gesture(&mut app, &mut panels, &[(ToolEventKind::Move, [6.0, 7.0]), (ToolEventKind::Up, [8.0, 9.0])]);
    assert_eq!(undo_labels(&app, 3), ["Stamp", "New Solid", "Stamp"]);
    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(app.layer_names(), ["Between", "Stamp"], "the first undo takes back only the moves after the solid");
    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(app.layer_names(), ["Stamp"]);
}

#[test]
fn a_tool_put_away_in_the_middle_of_a_gesture_folds_nothing_more_into_it() {
    let mut app = app("test.viewer-tool-away", &["document.read", "document.write"]);
    let mut panels = Panels::default();
    gesture(&mut app, &mut panels, &[(ToolEventKind::Down, [4.0, 5.0])]);
    panels.put_tool_away();
    gesture(&mut app, &mut panels, &[(ToolEventKind::Move, [6.0, 7.0]), (ToolEventKind::Up, [8.0, 9.0])]);
    assert_eq!(undo_labels(&app, 3), ["Stamp", "Change Property", "Change Property"]);
}

#[test]
fn a_grab_does_not_continue_on_the_overlay_of_a_reloaded_plugin() {
    let mut app = app("test.viewer-grab", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let grabbed = shown(&mut app, &mut panels);
    assert!(shown(&mut app, &mut panels).continues(&grabbed, 0));
    assert!(!shown(&mut app, &mut panels).continues(&grabbed, 9), "a handle the overlay does not have");
    app.plugins.load_for_session(app.bundle.path()).unwrap();
    let reloaded = shown(&mut app, &mut panels);
    assert_eq!(reloaded.overlay.handles.len(), grabbed.overlay.handles.len());
    assert!(!reloaded.continues(&grabbed, 0));
}

#[test]
fn a_plugin_built_before_viewer_controls_loads_renders_and_keeps_the_built_in_crosshair() {
    let mut app = app("test.viewer-before", &[]);
    let bundle = tempfile::tempdir().unwrap();
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/legacy/legacy.wasm"), bundle.path().join("legacy.wasm")).unwrap();
    let manifest = json!({"id": "test.viewer-legacy", "name": "Legacy", "version": "1.0.0", "entry": {"wasm": "legacy.wasm"}, "permissions": []});
    std::fs::write(bundle.path().join("plugin.json"), manifest.to_string()).unwrap();
    app.plugins.load_for_session(bundle.path()).expect("a plugin built before viewer controls loads");
    assert!(!tools().iter().any(|(plugin, _)| plugin == "test.viewer-legacy"), "it has no tools");

    app.session.execute("layer.newSolid", json!({"name": "Aim", "color": "#000000", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Aim", "effect": "test.viewer-legacy.mark"})).unwrap();
    let session = &app.session;
    let comp = session.active_comp_id().unwrap();
    let renderer = effectcraft_engine::render::Renderer::new(&session.project, session.footage.as_ref(), Default::default());
    let image = renderer.comp_frame(comp, Tick::ZERO);
    let centre = image.data[(image.height / 2 * image.width + image.width / 2) as usize];
    assert!((centre[0] - 0.5).abs() < 0.01 && (centre[1] - 0.5).abs() < 0.01, "it renders its point parameter: {centre:?}");

    let shown = shown(&mut app, &mut Panels::default());
    assert!(shown.overlay.handles.is_empty() && shown.overlay.marks.is_empty(), "it draws no overlay of its own");
    assert!(!shown.binds("spot"), "so the viewer keeps the built-in crosshair for its point");
}

fn hold_the_worker(app: &mut App, panels: &mut Panels) {
    crate::panels::relay(app, panels, &mut 0, json!({"command": "comp.info", "params": {}}), true);
}

#[test]
fn a_tool_event_queued_behind_a_slow_call_is_dropped_when_its_project_closes() {
    let mut app = app("test.viewer-tool-closed", &["document.read", "document.write"]);
    let mut panels = Panels::default();
    hold_the_worker(&mut app, &mut panels);
    panels.tool(&app.session, &app.id, "stamp", event(ToolEventKind::Key, [6.0, 7.0])).unwrap();
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Next", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    crate::panels::release(&app);
    settle(&mut app, &mut panels);
    assert_eq!(gesture(&mut app, &mut panels, &[(ToolEventKind::Key, [8.0, 9.0])]), ["Key 8.0 9.0"]);
}

#[test]
fn a_tool_event_of_an_earlier_activation_is_dropped_when_the_tool_is_picked_again() {
    let mut app = app("test.viewer-tool-again", &["document.read", "document.write"]);
    let mut panels = Panels::default();
    hold_the_worker(&mut app, &mut panels);
    panels.tool(&app.session, &app.id, "stamp", event(ToolEventKind::Key, [2.0, 2.0])).unwrap();
    panels.put_tool_away();
    panels.tool(&app.session, &app.id, "stamp", event(ToolEventKind::Key, [3.0, 3.0])).unwrap();
    crate::panels::release(&app);
    settle(&mut app, &mut panels);
    assert_eq!(gesture(&mut app, &mut panels, &[]), ["Key 3.0 3.0"]);
}

#[test]
fn an_overlay_and_its_grab_do_not_carry_over_to_a_reopened_project_with_the_same_ids_and_values() {
    let mut app = app("test.viewer-reopened", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let first = shown(&mut app, &mut panels);
    app.session.execute("file.newProject", json!({})).unwrap();
    app.session.execute("comp.new", json!({"name": "Test", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).unwrap();
    aim_layer(&mut app);
    let second = shown(&mut app, &mut panels);
    assert_eq!((Arc::ptr_eq(&first, &second), second.continues(&first, 0)), (false, false));
}

#[test]
fn a_custom_handle_dragged_back_to_where_its_value_started_sets_that_value() {
    let mut app = app("test.viewer-back", &[]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let start = shown(&mut app, &mut panels);
    drag(&mut app, &mut panels, &start, 6, [4.0, 5.0], "back");
    drag(&mut app, &mut panels, &start, 6, [-1.0, -2.0], "back");
    assert_eq!(value(&mut app, "pin"), json!("0,0"));
}

#[test]
fn drags_of_three_gestures_queued_behind_a_slow_call_keep_three_undo_steps() {
    let mut app = app("test.viewer-gestures", &["document.read"]);
    let mut panels = Panels::default();
    aim_layer(&mut app);
    let start = shown(&mut app, &mut panels);
    hold_the_worker(&mut app, &mut panels);
    for (delta, gesture) in [([4.0, 5.0], "one"), ([6.0, 7.0], "two"), ([8.0, 9.0], "three")] {
        post_drag(&mut app, &mut panels, &start, 6, delta, gesture);
    }
    crate::panels::release(&app);
    settle(&mut app, &mut panels);
    let last = value(&mut app, "pin");
    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!((last, value(&mut app, "pin")), (json!("9,11"), json!("7,9")));
}
