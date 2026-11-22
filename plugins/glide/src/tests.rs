use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use craft_plugin::{Panel, Paint, Pointer, PointerPhase, Shape, Ui, UiEvent, UiEventKind, UiInput, Widget};
use serde_json::{Value, json};

use crate::apply::{self, arc_length};
use crate::bridge::Host;
use crate::curve::{Curve, Handle};
use crate::panel::GlidePanel;
use crate::presets::{self, BUILTIN, PRESETS_FILE, Preset};

#[derive(Default)]
struct Recorded {
    calls: Vec<(String, Value)>,
    files: HashMap<String, Vec<u8>>,
}

#[derive(Clone, Default)]
struct FakeHost {
    replies: Arc<HashMap<&'static str, Value>>,
    recorded: Arc<Mutex<Recorded>>,
}

impl FakeHost {
    fn replying(replies: &[(&'static str, Value)]) -> FakeHost {
        FakeHost { replies: Arc::new(replies.iter().cloned().collect()), ..FakeHost::default() }
    }

    fn calls(&self, id: &str) -> Vec<Value> {
        self.recorded.lock().map(|r| r.calls.iter().filter(|(c, _)| c == id).map(|(_, p)| p.clone()).collect()).unwrap_or_default()
    }

    fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.recorded.lock().ok().and_then(|r| r.files.get(path).cloned())
    }
}

impl Host for FakeHost {
    fn command(&self, id: &str, params: Value) -> Result<Value, String> {
        if let Ok(mut recorded) = self.recorded.lock() {
            recorded.calls.push((id.to_string(), params));
        }
        self.replies.get(id).cloned().ok_or_else(|| format!("{id} is not available"))
    }

    fn read_data(&self, path: &str) -> Result<Vec<u8>, String> {
        self.file(path).ok_or_else(|| format!("{path} does not exist"))
    }

    fn write_data(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.recorded.lock().map(|mut r| r.files.insert(path.to_string(), bytes.to_vec())).map(|_| ()).map_err(|_| "poisoned".to_string())
    }
}

const PANEL_SIZE: [f32; 2] = [300.0, 900.0];

fn frame(panel: &mut GlidePanel<FakeHost>, events: Vec<UiEvent>) -> Vec<Widget> {
    let input = UiInput { events, size: PANEL_SIZE, ..Default::default() };
    let mut ui = Ui::new(&input);
    panel.ui(&mut ui);
    ui.finish()
}

fn flatten(widgets: &[Widget]) -> Vec<&Widget> {
    widgets
        .iter()
        .flat_map(|w| match w {
            Widget::Row { children } | Widget::Column { children } | Widget::Group { children, .. } => {
                std::iter::once(w).chain(flatten(children)).collect::<Vec<_>>()
            }
            other => vec![other],
        })
        .collect()
}

fn canvases(widgets: &[Widget]) -> Vec<(u64, Vec<Shape>)> {
    flatten(widgets).into_iter().filter_map(|w| if let Widget::Canvas { id, shapes, .. } = w { Some((*id, shapes.clone())) } else { None }).collect()
}

fn button(widgets: &[Widget], text: &str) -> Option<u64> {
    flatten(widgets).into_iter().find_map(|w| match w {
        Widget::Button { id, text: t, .. } if t == text => Some(*id),
        _ => None,
    })
}

fn labels(widgets: &[Widget]) -> Vec<String> {
    flatten(widgets).into_iter().filter_map(|w| if let Widget::Label { text, .. } = w { Some(text.clone()) } else { None }).collect()
}

fn pointer(id: u64, pos: [f32; 2], phase: PointerPhase) -> Vec<UiEvent> {
    vec![UiEvent { id, kind: UiEventKind::Pointer(Pointer::new(pos, phase)) }]
}

fn click(id: u64) -> Vec<UiEvent> {
    vec![UiEvent { id, kind: UiEventKind::Click }]
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn opacity_selection() -> Value {
    json!([
        {"layer": 4, "prop": 15, "name": "Opacity", "time": 0.0, "compTime": 0.0, "value": 0.0, "spatial": null,
         "inEase": [null], "outEase": [{"speed": 0.0, "influence": 42.0}]},
        {"layer": 4, "prop": 15, "name": "Opacity", "time": 2.0, "compTime": 2.0, "value": 100.0, "spatial": null,
         "inEase": [{"speed": 0.0, "influence": 42.0}], "outEase": [null]}
    ])
}

fn two_key_property() -> Value {
    json!({"keys": [{"time": 0.0}, {"time": 2.0}]})
}

fn ease_steps(batch: &Value) -> Vec<(String, u64, f64, f64)> {
    batch["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .filter(|s| s["command"] == "keys.setEase")
                .map(|s| {
                    let p = &s["params"];
                    (p["side"].as_str().unwrap_or("").to_string(), p["dim"].as_u64().unwrap_or(99), p["speed"].as_f64().unwrap_or(f64::NAN), p["influence"].as_f64().unwrap_or(f64::NAN))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn dragging_the_start_handle_follows_the_pointer_and_clamps_overshoot() {
    let mut panel = GlidePanel::new(FakeHost::default());
    let editor = canvases(&frame(&mut panel, vec![]))[0].0;
    frame(&mut panel, pointer(editor, [116.0, 191.0], PointerPhase::Down));
    assert_eq!(panel.drag.grabbed, Some(Handle::Start));
    frame(&mut panel, pointer(editor, [75.0, 136.0], PointerPhase::Drag));
    assert!(close(panel.curve.x1, 0.25) && close(panel.curve.y1, 0.5));
    frame(&mut panel, pointer(editor, [-40.0, -10.0], PointerPhase::Up));
    assert_eq!(panel.curve, Curve::new(0.0, 1.6, 0.58, 1.0));
    assert_eq!(panel.drag.grabbed, None);
}

#[test]
fn hovering_highlights_the_nearer_handle() {
    let mut panel = GlidePanel::new(FakeHost::default());
    let editor = canvases(&frame(&mut panel, vec![]))[0].0;
    let widgets = frame(&mut panel, pointer(editor, [160.0, 75.0], PointerPhase::Hover));
    assert_eq!(panel.drag.hovered, Some(Handle::End));
    let highlighted: Vec<[f32; 2]> = canvases(&widgets)[0]
        .1
        .iter()
        .filter_map(|s| match s {
            Shape::Circle { center, fill: Some(Paint::Accent), radius, .. } if *radius > 7.0 => Some(*center),
            _ => None,
        })
        .collect();
    assert_eq!(highlighted.len(), 1);
    assert!((highlighted[0][0] - 155.52).abs() < 1e-3 && (highlighted[0][1] - 80.545_45).abs() < 1e-3);
}

#[test]
fn clicking_a_preset_thumbnail_loads_its_curve_without_applying() {
    let host = FakeHost::default();
    let mut panel = GlidePanel::new(host.clone());
    let all = canvases(&frame(&mut panel, vec![]));
    let back_out = BUILTIN.iter().position(|p| p.name == "Back Out").map_or(0, |i| i + 1);
    frame(&mut panel, pointer(all[back_out].0, [20.0, 20.0], PointerPhase::Up));
    assert_eq!(panel.curve, Curve::new(0.34, 1.56, 0.64, 1.0));
    assert!(host.calls("engine.batch").is_empty());
    let widgets = frame(&mut panel, vec![]);
    assert!(labels(&widgets).contains(&"cubic-bezier(0.34, 1.56, 0.64, 1)".to_string()));
}

#[test]
fn applying_without_a_selection_says_what_to_select() {
    let mut panel = GlidePanel::new(FakeHost::replying(&[("keys.info", json!([]))]));
    let apply = button(&frame(&mut panel, vec![]), "Apply").unwrap_or_default();
    let widgets = frame(&mut panel, click(apply));
    assert!(labels(&widgets).contains(&"Select two or more neighbouring keyframes on a property.".to_string()));
}

#[test]
fn applying_with_one_selected_key_says_what_to_select() {
    let one = json!([{"layer": 4, "prop": 15, "name": "Opacity", "time": 0.0, "compTime": 0.0, "value": 0.0}]);
    let mut panel = GlidePanel::new(FakeHost::replying(&[("keys.info", one)]));
    let apply = button(&frame(&mut panel, vec![]), "Apply").unwrap_or_default();
    frame(&mut panel, click(apply));
    assert_eq!(panel.status, "Each property has only one selected keyframe: select two or more neighbouring keyframes.");
}

#[test]
fn auto_apply_sends_the_clicked_preset_as_one_batch() {
    let host = FakeHost::replying(&[("keys.info", opacity_selection()), ("prop.get", two_key_property()), ("engine.batch", json!({}))]);
    let mut panel = GlidePanel::new(host.clone());
    panel.auto_apply = true;
    let all = canvases(&frame(&mut panel, vec![]));
    let back_out = BUILTIN.iter().position(|p| p.name == "Back Out").map_or(0, |i| i + 1);
    frame(&mut panel, pointer(all[back_out].0, [20.0, 20.0], PointerPhase::Up));
    let batches = host.calls("engine.batch");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0]["label"], "Glide: Apply Curve");
    let steps = ease_steps(&batches[0]);
    assert_eq!(steps.len(), 2);
    assert_eq!((steps[0].0.as_str(), steps[0].1), ("out", 0));
    assert!(close(steps[0].2, 229.41176470588235) && close(steps[0].3, 34.0));
    assert_eq!((steps[1].0.as_str(), steps[1].1, steps[1].2), ("in", 0, 0.0));
    assert!(close(steps[1].3, 36.0));
    assert_eq!(panel.status, "Applied to 2 keys on 1 property");
}

#[test]
fn reading_the_selection_loads_the_curve_of_the_first_pair() {
    let host = FakeHost::replying(&[("keys.info", opacity_selection()), ("prop.get", two_key_property())]);
    let mut panel = GlidePanel::new(host);
    let read = button(&frame(&mut panel, vec![]), "Read Selection").unwrap_or_default();
    panel.curve = Curve::new(0.1, 0.2, 0.3, 0.4);
    frame(&mut panel, click(read));
    assert!(close(panel.curve.x1, 0.42) && close(panel.curve.y1, 0.0) && close(panel.curve.x2, 0.58) && close(panel.curve.y2, 1.0));
    assert_eq!(panel.status, "Read cubic-bezier(0.42, 0, 0.58, 1) from Opacity");
}

#[test]
fn number_fields_edit_the_curve_and_clamp_it() {
    let mut panel = GlidePanel::new(FakeHost::default());
    let widgets = frame(&mut panel, vec![]);
    let field = |name: &str| {
        flatten(&widgets).into_iter().find_map(|w| match w {
            Widget::Number { id, text, .. } if text == name => Some(*id),
            _ => None,
        })
    };
    let (x2, y1) = (field("x2").unwrap_or_default(), field("y1").unwrap_or_default());
    frame(&mut panel, vec![UiEvent { id: x2, kind: UiEventKind::Number(1.7) }, UiEvent { id: y1, kind: UiEventKind::Number(-0.3) }]);
    assert_eq!(panel.curve, Curve::new(0.42, -0.3, 1.0, 1.0));
}

#[test]
fn saved_presets_are_written_and_load_in_a_new_panel() {
    let host = FakeHost::default();
    let mut panel = GlidePanel::new(host.clone());
    panel.curve = Curve::new(0.9, 0.0, 0.1, 1.0);
    let widgets = frame(&mut panel, vec![]);
    let name = flatten(&widgets).into_iter().find_map(|w| if let Widget::TextEdit { id, .. } = w { Some(*id) } else { None }).unwrap_or_default();
    let save = button(&widgets, "Save").unwrap_or_default();
    frame(&mut panel, vec![UiEvent { id: name, kind: UiEventKind::Text("Snap".into()) }, UiEvent { id: save, kind: UiEventKind::Click }]);
    let snap = vec![Preset { name: "Snap".into(), curve: Curve::new(0.9, 0.0, 0.1, 1.0) }];
    assert_eq!(presets::decode(&host.file(PRESETS_FILE).unwrap_or_default()), Ok(snap.clone()));
    let mut reopened = GlidePanel::new(host.clone());
    let widgets = frame(&mut reopened, vec![]);
    assert_eq!(reopened.user_presets, snap);
    let delete = button(&widgets, "Delete").unwrap_or_default();
    frame(&mut reopened, click(delete));
    assert_eq!(presets::decode(&host.file(PRESETS_FILE).unwrap_or_default()), Ok(vec![]));
    assert_eq!(reopened.status, "Deleted preset Snap");
}

#[test]
fn scale_gets_one_ease_per_dimension_from_its_own_value_change() {
    let selection = json!([
        {"layer": 4, "prop": 10, "name": "Scale", "time": 0.0, "compTime": 0.0, "value": [100.0, 50.0, 100.0], "spatial": null},
        {"layer": 4, "prop": 10, "name": "Scale", "time": 4.0, "compTime": 4.0, "value": [160.0, 250.0, 100.0], "spatial": null}
    ]);
    let host = FakeHost::replying(&[("keys.info", selection), ("prop.get", json!({"keys": [{"time": 0.0}, {"time": 4.0}]})), ("engine.batch", json!({}))]);
    let curve = Curve::new(0.5, 0.25, 0.75, 1.0);
    assert_eq!(apply::apply(&host, curve, "Test"), Ok("Applied to 2 keys on 1 property".to_string()));
    let steps = ease_steps(&host.calls("engine.batch")[0]);
    let outgoing: Vec<(u64, f64)> = steps.iter().filter(|s| s.0 == "out").map(|s| (s.1, s.2)).collect();
    assert_eq!(outgoing, vec![(0, 7.5), (1, 25.0), (2, 0.0)]);
}

#[test]
fn a_spatial_property_eases_along_its_measured_path_length() {
    let selection = json!([
        {"layer": 4, "prop": 9, "name": "Position", "time": 0.0, "compTime": 0.0, "value": [0.0, 0.0, 0.0], "spatial": "autoBezier"},
        {"layer": 4, "prop": 9, "name": "Position", "time": 2.0, "compTime": 2.0, "value": [300.0, 400.0, 0.0], "spatial": "autoBezier"}
    ]);
    let line: Vec<Value> = (0..=128).map(|i| json!({"value": [300.0 * f64::from(i) / 128.0, 400.0 * f64::from(i) / 128.0, 0.0]})).collect();
    let host = FakeHost::replying(&[("keys.info", selection), ("prop.get", two_key_property()), ("engine.batch", json!({"results": line}))]);
    apply::apply(&host, Curve::new(0.25, 0.5, 0.75, 1.0), "Test").unwrap_or_default();
    let batches = host.calls("engine.batch");
    assert_eq!(batches[0]["steps"].as_array().map(Vec::len), Some(129));
    let steps = ease_steps(&batches[1]);
    assert_eq!(steps.len(), 2);
    assert!(close(steps[0].2, 500.0) && close(steps[0].3, 25.0));
}

#[test]
fn an_auto_bezier_key_keeps_its_outer_side_and_continuous_keys_are_released() {
    let selection = json!([
        {"layer": 4, "prop": 12, "name": "Rotation", "time": 1.0, "compTime": 1.0, "value": 40.0, "spatial": null, "autoBezier": true,
         "inEase": [{"speed": 50.0, "influence": 16.666666666666664}], "outEase": [{"speed": 50.0, "influence": 16.666666666666664}]},
        {"layer": 4, "prop": 12, "name": "Rotation", "time": 2.0, "compTime": 2.0, "value": 100.0, "spatial": null, "continuous": true,
         "inEase": [{"speed": 60.0, "influence": 30.0}], "outEase": [{"speed": 60.0, "influence": 30.0}]}
    ]);
    let property = json!({"keys": [{"time": 0.0}, {"time": 1.0}, {"time": 2.0}, {"time": 3.0}]});
    let host = FakeHost::replying(&[("keys.info", selection), ("prop.get", property), ("engine.batch", json!({}))]);
    apply::apply(&host, Curve::DEFAULT, "Test").unwrap_or_default();
    let batch = &host.calls("engine.batch")[0];
    assert_eq!(batch["steps"][0], json!({"command": "keys.interpolation", "params": {"continuous": false}}));
    let steps = ease_steps(batch);
    assert_eq!(steps[0].0, "in");
    assert!(close(steps[0].2, 50.0) && close(steps[0].3, 16.666666666666664));
    assert_eq!(steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), vec!["in", "out", "in"]);
}

#[test]
fn a_quarter_circle_measures_its_arc_length() {
    let points: Vec<Vec<f64>> = (0..=128)
        .map(|i| {
            let angle = std::f64::consts::FRAC_PI_2 * f64::from(i) / 128.0;
            vec![100.0 * angle.cos(), 100.0 * angle.sin()]
        })
        .collect();
    assert!((arc_length(&points) - 157.07963267948966).abs() < 1e-7);
}

#[test]
fn auto_apply_runs_once_when_a_handle_drag_ends() {
    let host = FakeHost::replying(&[("keys.info", opacity_selection()), ("prop.get", two_key_property()), ("engine.batch", json!({}))]);
    let mut panel = GlidePanel::new(host.clone());
    panel.auto_apply = true;
    let editor = canvases(&frame(&mut panel, vec![]))[0].0;
    frame(&mut panel, pointer(editor, [116.0, 191.0], PointerPhase::Down));
    frame(&mut panel, pointer(editor, [75.0, 136.0], PointerPhase::Drag));
    assert!(host.calls("engine.batch").is_empty());
    frame(&mut panel, pointer(editor, [75.0, 136.0], PointerPhase::Up));
    let steps = ease_steps(&host.calls("engine.batch")[0]);
    assert!(close(steps[0].2, 100.0) && close(steps[0].3, 25.0));
}

#[test]
fn the_result_names_properties_that_were_skipped() {
    let mut selection = opacity_selection();
    if let Some(keys) = selection.as_array_mut() {
        keys.push(json!({"layer": 4, "prop": 12, "name": "Rotation", "time": 1.0, "compTime": 1.0, "value": 10.0}));
    }
    let host = FakeHost::replying(&[("keys.info", selection), ("prop.get", two_key_property()), ("engine.batch", json!({}))]);
    assert_eq!(apply::apply(&host, Curve::DEFAULT, "Test"), Ok("Applied to 2 keys on 1 property; skipped 1 property with one selected key".to_string()));
}
