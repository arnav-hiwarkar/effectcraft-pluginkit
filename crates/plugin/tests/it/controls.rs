use std::time::Duration;

use effectcraft_plugin::Panels;
use serde_json::{Value, json};

use crate::{App, app};

fn switched(id: &str) -> (App, u64, [u64; 2]) {
    let mut app = app(id, &["document.read", "document.write"]);
    app.session.execute("layer.newSolid", json!({"name": "S", "color": "#000000", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "S", "effect": format!("{id}.switches")})).unwrap();
    let layer = app.session.active_comp().unwrap().layers[0].clone();
    let effect = layer.effects().unwrap().groups().next().unwrap();
    let buttons = ["reset", "ask"].map(|param| effect.get(param).unwrap().uid);
    (app, layer.id.0, buttons)
}

fn get(app: &mut App, param: &str) -> Value {
    app.session.execute("prop.get", json!({"layer": "S", "path": format!("effects/#1/{param}")})).unwrap()["value"].clone()
}

fn values(app: &mut App) -> [Value; 2] {
    [get(app, "softness"), get(app, "amount")]
}

fn set(app: &mut App, param: &str, value: Value) -> Result<Value, String> {
    app.session.execute_checked("prop.set", json!({"layer": "S", "path": format!("effects/#1/{param}"), "value": value})).map_err(|e| e.to_string())
}

fn undo(app: &mut App) {
    app.session.execute("edit.undo", json!({})).unwrap();
}

fn pump(app: &mut App, panels: &mut Panels, done: impl Fn(&App, &Panels) -> bool) {
    for frame in 1..5000 {
        panels.frame(&mut app.session, Duration::from_millis(16 * frame));
        if done(app, panels) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the press never settled")
}

fn log(app: &mut App) -> String {
    app.session.execute("plugin.log", json!({"id": app.id})).unwrap().to_string()
}

#[test]
fn a_button_takes_no_value_and_a_turned_off_parameter_takes_no_edit_while_a_hidden_one_does() {
    let (mut app, ..) = switched("test.controls-edits");
    let button = set(&mut app, "reset", json!(true)).unwrap_err();
    assert!(button.contains("is a button"), "{button}");
    let key = app.session.execute_checked("prop.addKey", json!({"layer": "S", "path": "effects/#1/reset", "time": 0.5})).unwrap_err().to_string();
    assert!(key.contains("is a button"), "{key}");
    let off = set(&mut app, "amount", json!(3.0)).unwrap_err();
    assert!(off.contains("turned off by another control"), "{off}");
    set(&mut app, "softness", json!(6.0)).unwrap();
    app.session.execute("effect.applyPreset", json!({"layer": "S", "effect": 1, "preset": "Calm"})).unwrap();
    assert_eq!(values(&mut app), [json!(2.0), json!(1.0)], "a preset sets hidden Softness and skips turned-off Amount");
    set(&mut app, "manual", json!(true)).unwrap();
    app.session.execute("effect.applyPreset", json!({"layer": "S", "effect": 1, "preset": "Calm"})).unwrap();
    assert_eq!(values(&mut app), [json!(2.0), json!(5.0)]);
}

#[test]
fn plugin_press_runs_without_the_app_window_in_one_undo_step() {
    let (mut app, layer, [reset, _]) = switched("test.controls-press");
    app.session.execute("plugin.press", json!({"layer": layer, "prop": reset})).unwrap();
    assert_eq!(values(&mut app), [json!(0.0), json!(0.0)]);
    undo(&mut app);
    assert_eq!(values(&mut app), [json!(4.0), json!(1.0)]);
}

#[test]
fn a_queued_press_stops_when_its_effect_is_removed_or_another_edit_lands_while_it_runs() {
    let (mut app, layer, [reset, ask]) = switched("test.controls-queue");
    let mut panels = Panels::default();
    panels.press(&app.session, layer, reset).unwrap();
    app.session.execute("effect.remove", json!({"layer": "S", "effect": 1})).unwrap();
    pump(&mut app, &mut panels, |_, panels| panels.idle());
    let said = log(&mut app);
    assert!(said.contains("the press of `Reset` stopped: its effect was removed"), "{said}");
    undo(&mut app);
    assert_eq!(values(&mut app), [json!(4.0), json!(1.0)], "the removed effect's press changed nothing");

    panels.press(&app.session, layer, ask).unwrap();
    pump(&mut app, &mut panels, |app, _| effectcraft_plugin::asked_form(&app.session, |_| ()).is_some());
    set(&mut app, "manual", json!(true)).unwrap();
    let request = effectcraft_plugin::asked_form(&app.session, |asked| asked.id).unwrap();
    effectcraft_plugin::answer_form(&app.session, request, Some(Default::default())).unwrap();
    pump(&mut app, &mut panels, |_, panels| panels.idle());
    assert_eq!(values(&mut app), [json!(0.0), json!(1.0)], "the press stops before its write after the other edit");
    assert!(log(&mut app).contains("another edit changed the project while it ran"));
    undo(&mut app);
    assert_eq!(get(&mut app, "manual"), json!(false));
    undo(&mut app);
    assert_eq!(values(&mut app), [json!(4.0), json!(1.0)], "the press's own edits are one undo step");
}
