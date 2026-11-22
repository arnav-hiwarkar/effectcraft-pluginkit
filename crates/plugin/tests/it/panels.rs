use std::time::Duration;

use effectcraft_plugin::ui::{Key, Modifiers, UiEvent, UiEventKind, Widget};
use effectcraft_plugin::{Panels, Surface};
use serde_json::json;

use crate::{App, app};

const KEY: &str = "plugin.test.panels-watch.watch";

pub(crate) fn settled(app: &mut App, panels: &mut Panels, key: &str, surface: Surface, frame: &mut u64) -> (Vec<Widget>, Option<String>) {
    for _ in 0..1000 {
        *frame += 1;
        panels.frame(&mut app.session, Duration::from_millis(16 * *frame));
        let shown = panels.show(&mut app.session, key, surface, [300.0, 200.0]).expect("the probe surface shows");
        assert_eq!(shown.error, None);
        let drawn = (shown.widgets.to_vec(), shown.edited);
        if !panels.busy(key) {
            return drawn;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the probe plugin never answered")
}

fn show(app: &mut App, panels: &mut Panels, frame: &mut u64) -> Vec<Widget> {
    let id = app.id.clone();
    settled(app, panels, KEY, Surface::Panel { plugin: &id, panel: "watch" }, frame).0
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
