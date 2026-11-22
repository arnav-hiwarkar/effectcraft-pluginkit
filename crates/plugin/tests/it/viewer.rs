use std::sync::Arc;

use craft_plugin_host::ui::{Paint, Shape};
use craft_plugin_host::viewer::{Bind, Mark, Space, ToolEvent, ToolEventKind};
use effectcraft_engine::render::EvalCtx;
use effectcraft_engine::time::Tick;
use effectcraft_plugin::viewer::{Shown, overlay, tool_event, tools};
use serde_json::{Value, json};

use crate::{App, app};

fn aim_layer(app: &mut App) {
    app.session.execute("layer.newSolid", json!({"name": "Aim", "color": "#204060", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Aim", "effect": format!("{}.aim", app.id)})).unwrap();
}

fn shown(app: &App) -> Arc<Shown> {
    let session = &app.session;
    let comp = session.active_comp().unwrap();
    let ctx = EvalCtx::new(&session.project, session.active_comp_id().unwrap(), comp, Tick::ZERO);
    let layer = comp.layers.iter().find(|layer| layer.name == "Aim").unwrap();
    overlay(&ctx, layer, layer.effects().unwrap().groups().next().unwrap()).expect("the effect draws an overlay")
}

fn drag(app: &mut App, shown: &Shown, handle: usize, delta: [f64; 2], merge: &str) {
    let comp = app.session.active_comp().unwrap();
    let layer = comp.layers.iter().find(|layer| layer.name == "Aim").unwrap();
    let commands = shown.drag_commands(handle, delta, layer, layer.effects().unwrap().groups().next().unwrap(), merge).unwrap();
    for (id, params) in commands {
        app.session.execute_checked(&id, params).unwrap();
    }
}

fn value(app: &mut App, param: &str) -> Value {
    app.session.execute_checked("prop.get", json!({"layer": "Aim", "path": format!("effects/#1/{param}")})).unwrap()["value"].clone()
}

#[test]
fn an_effect_overlay_arrives_with_its_shapes_and_handles_in_layer_space_and_is_kept_until_a_value_changes() {
    let mut app = app("test.viewer-overlay", &[]);
    aim_layer(&mut app);
    let first = shown(&app);
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
    assert!(Arc::ptr_eq(&first, &shown(&app)), "the same values give the retained overlay without a plugin call");
    app.session.execute_checked("prop.set", json!({"layer": "Aim", "path": "effects/#1/reach", "value": 20.0})).unwrap();
    let changed = shown(&app);
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.overlay.handles[2].at, [72.0, 18.0]);
}

#[test]
fn dragging_each_kind_of_handle_sets_its_parameter_in_one_undo_step() {
    let mut app = app("test.viewer-drag", &[]);
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
        let start = shown(&app);
        drag(&mut app, &start, handle, [delta[0] / 2.0, delta[1] / 2.0], &format!("gesture-{n}"));
        drag(&mut app, &start, handle, delta, &format!("gesture-{n}"));
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
    aim_layer(&mut app);
    for (time, at) in [(0.0, [10.0, 10.0]), (1.0, [20.0, 20.0])] {
        app.session.execute_checked("prop.addKey", json!({"layer": "Aim", "path": "effects/#1/centre", "time": time, "value": at})).unwrap();
    }
    let start = shown(&app);
    drag(&mut app, &start, 0, [5.0, 0.0], "keyed");
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

#[test]
fn a_tool_gesture_edits_the_project_through_commands_as_one_undo_step() {
    let mut app = app("test.viewer-tool", &["document.read", "document.write"]);
    assert!(tools().iter().any(|(plugin, spec)| *plugin == app.id && spec.id == "stamp" && spec.name == "Stamp"));
    let steps_before = app.session.history.undo.len();
    let mut last = None;
    for (kind, at) in [(ToolEventKind::Down, [4.0, 5.0]), (ToolEventKind::Move, [6.0, 7.0]), (ToolEventKind::Move, [8.0, 9.0]), (ToolEventKind::Up, [8.0, 9.0])]
    {
        last = Some(tool_event(&mut app.session, &app.id, "stamp", &event(kind, at)).unwrap());
    }
    let texts: Vec<String> = last
        .unwrap()
        .marks
        .into_iter()
        .filter_map(|mark| match mark.shape {
            Shape::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Down 4 5", "Move 6 7", "Move 8 9", "Up 8 9"]);
    assert_eq!(app.layer_names(), ["Stamp"]);
    let position = app.session.execute_checked("prop.get", json!({"layer": "Stamp", "path": "transform/position"})).unwrap()["value"].clone();
    assert_eq!(position, json!([8.0, 9.0, 0.0]));
    assert_eq!(app.session.history.undo.len(), steps_before + 1);
    assert_eq!(app.session.history.undo.last().map(|(label, _)| label.as_str()), Some("Stamp"));
    app.session.execute_checked("edit.undo", json!({})).unwrap();
    assert!(app.layer_names().is_empty(), "one undo removes the whole gesture");
}
