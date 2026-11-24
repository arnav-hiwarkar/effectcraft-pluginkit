use craft_plugin::{Drag, Dropped, Key, Modifiers, Paint, Pointer, PointerPhase, Shape, TextStyle, UiEvent, UiEventKind, Widget};
use egui::{Color32, FontFamily, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::{Host, Theme, Triangulate, draw};

struct Bare;

impl Host for Bare {
    type Payload = ();

    fn tr(&self, english: &str) -> String {
        english.to_string()
    }
    fn register(&mut self, _: &str, _: &egui::Response, _: &str) {}
    fn texture(&mut self, _: &egui::Context, _: &str) -> Option<egui::TextureId> {
        None
    }
    fn triangulate(&self) -> Triangulate {
        |_| Vec::new()
    }
    fn drag(&self, _: &Drag) -> Option<()> {
        None
    }
    fn dropped(&self, _: &()) -> Option<Dropped> {
        None
    }
    fn accept_files(&mut self, _: &[String]) {}
}

fn theme() -> Theme {
    Theme {
        text: Color32::WHITE,
        weak: Color32::GRAY,
        accent: Color32::LIGHT_BLUE,
        background: Color32::BLACK,
        surface: Color32::DARK_GRAY,
        border: Color32::GRAY,
        error: Color32::RED,
        heading: FontFamily::Proportional,
    }
}

fn harness(widgets: Vec<Widget>) -> Harness<'static, Vec<UiEvent>> {
    Harness::new_ui_state(move |ui, events: &mut Vec<UiEvent>| events.extend(draw(ui, &widgets, &theme(), "plugin.test.panel", &mut Bare)), Vec::new())
}

#[test]
fn an_open_section_inside_a_row_stays_open_when_a_sibling_appears_above_the_row() {
    let section =
        Widget::Collapsing { id: 2, title: "More".into(), open: false, children: vec![Widget::Button { id: 3, text: "Inside".into(), enabled: true }] };
    let mut harness = Harness::new_ui_state(
        move |ui, widgets: &mut Vec<Widget>| {
            draw(ui, widgets, &theme(), "plugin.test.panel", &mut Bare);
        },
        vec![Widget::Row { children: vec![section] }],
    );
    harness.get_by_label("More").click();
    harness.run();
    assert!(harness.query_by_label("Inside").is_some());
    harness.state_mut().insert(0, Widget::Label { text: "Name taken".into(), style: TextStyle::Error });
    harness.run();
    assert!(harness.query_by_label("Name taken").is_some());
    assert!(harness.query_by_label("Inside").is_some());
}

#[test]
fn clicking_a_button_sends_a_click_with_its_id() {
    let mut harness = harness(vec![Widget::Button { id: 42, text: "Count".into(), enabled: true }]);
    harness.get_by_label("Count").click();
    harness.run();
    assert_eq!(harness.state(), &vec![UiEvent { id: 42, kind: UiEventKind::Click }]);
}

#[test]
fn dragging_a_slider_sends_numbers_with_its_id_ending_at_the_dragged_value() {
    let mut harness = harness(vec![Widget::Slider { id: 7, text: "Speed".into(), value: 0.0, min: 0.0, max: 10.0 }]);
    harness.run();
    let rail = harness.get_by_role(egui::Role::Slider).rect();
    harness.drag_at(rail.left_center());
    harness.run();
    harness.hover_at(rail.right_center() + vec2(40.0, 0.0));
    harness.run();
    harness.drop_at(rail.right_center() + vec2(40.0, 0.0));
    harness.run();
    let events = harness.state();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.id == 7 && matches!(event.kind, UiEventKind::Number(_))), "{events:?}");
    assert_eq!(events.last().map(|event| &event.kind), Some(&UiEventKind::Number(10.0)));
}

#[test]
fn dragging_a_number_with_a_range_stops_at_the_range_and_keeps_its_decimals() {
    let number = Widget::Number { id: 3, text: "Count".into(), value: 2.0, speed: 1.0, range: Some([0.0, 5.0]), decimals: Some(1) };
    let mut harness = harness(vec![number]);
    harness.run();
    assert_eq!(harness.get_by_role(egui::Role::SpinButton).value().as_deref(), Some("2.0"));
    let at = harness.get_by_role(egui::Role::SpinButton).rect().center();
    harness.drag_at(at);
    harness.run();
    harness.hover_at(at + vec2(200.0, 0.0));
    harness.run();
    harness.drop_at(at + vec2(200.0, 0.0));
    harness.run();
    assert_eq!(harness.state().last().map(|event| &event.kind), Some(&UiEventKind::Number(5.0)));
}

#[test]
fn enter_in_a_number_field_commits_the_value_and_tab_then_reaches_the_next_control() {
    let number = Widget::Number { id: 3, text: "Count".into(), value: 2.0, speed: 1.0, range: None, decimals: None };
    let next = Widget::Button { id: 10, text: "Next".into(), enabled: true };
    let mut harness = harness(vec![number, next]);
    harness.run();
    harness.get_by_role(egui::Role::SpinButton).click();
    harness.run();
    harness.get_by_role(egui::Role::SpinButton).type_text("4");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(harness.state().last().map(|event| &event.kind), Some(&UiEventKind::Number(4.0)));
    assert!(harness.get_by_role(egui::Role::SpinButton).is_focused(), "the field keeps the keyboard after Enter");
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.get_by_label("Next").is_focused(), "Tab goes to the control after the field");
}

#[test]
fn a_canvas_with_extreme_or_broken_coordinates_still_draws_in_bounded_time() {
    let far = f32::MAX;
    let shapes = vec![
        Shape::Bezier { points: [[0.0, 0.0], [1e30, 1e30], [-1e30, 0.0], [10.0, 10.0]], color: Paint::Accent, width: 1e30 },
        Shape::Bezier { points: [[0.0, 0.0], [f32::NAN, 1.0], [2.0, f32::INFINITY], [10.0, 10.0]], color: Paint::Accent, width: 1.0 },
        Shape::Line { points: vec![[0.0, 0.0], [far, -far], [f32::NAN, 0.0]], color: Paint::Text, width: f32::NAN },
        Shape::Circle { center: [5.0, 5.0], radius: f32::INFINITY, fill: Some(Paint::Error), stroke: None },
        Shape::Rect { min: [-far, -far], max: [far, far], fill: Some(Paint::Surface), stroke: Some((Paint::Border, far)), radius: f32::NAN },
        Shape::Text { pos: [f32::NAN, 0.0], text: "x".into(), color: Paint::Text, size: f32::INFINITY },
    ];
    let mut harness = harness(vec![Widget::Canvas { id: 7, size: [f32::INFINITY, f32::NAN], shapes }]);
    harness.run();
    assert!(harness.state().iter().all(|event| event.id == 7));
}

#[test]
fn clicking_an_image_with_no_upload_yet_still_sends_a_click_with_its_id() {
    let image = Widget::Image { id: 9, key: "thumb/0".into(), size: [40.0, 30.0] };
    let tip = Widget::Tooltip { text: "Linear".into() };
    let cell = Widget::Column { children: vec![image, tip] };
    let grid = Widget::Grid { id: 8, columns: 3, children: vec![cell] };
    let mut harness = harness(vec![Widget::Scroll { id: 7, height: 100.0, children: vec![grid] }]);
    harness.get_by_label("thumb/0").click();
    harness.run();
    assert_eq!(harness.state(), &vec![UiEvent { id: 9, kind: UiEventKind::Click }]);
}

#[test]
fn clicking_a_tab_inside_a_scroll_area_and_a_grid_sends_its_index() {
    let tabs = Widget::Tabs { id: 5, options: vec!["Red".into(), "Green".into()], selected: 0 };
    let grid = Widget::Grid { id: 6, columns: 2, children: vec![tabs, Widget::Progress { fraction: 0.5, text: "Half".into() }] };
    let mut harness = harness(vec![Widget::Scroll { id: 4, height: 80.0, children: vec![grid] }]);
    harness.get_by_label("Green").click();
    harness.run();
    assert_eq!(harness.state(), &vec![UiEvent { id: 5, kind: UiEventKind::Number(1.0) }]);
}

#[test]
fn a_collapsing_section_closed_by_default_hides_its_children_until_opened() {
    let button = Widget::Button { id: 3, text: "Inside".into(), enabled: true };
    let mut harness = harness(vec![Widget::Collapsing { id: 2, title: "More".into(), open: false, children: vec![button] }]);
    harness.run();
    assert!(harness.query_by_label("Inside").is_none());
    harness.get_by_label("More").click();
    harness.run();
    harness.run();
    assert!(harness.query_by_label("Inside").is_some());
}

#[test]
fn dragging_on_a_canvas_sends_down_then_drags_then_up_with_the_held_modifiers() {
    let mut harness = harness(vec![Widget::Canvas { id: 11, size: [100.0, 100.0], shapes: vec![] }]);
    harness.run();
    let rect = harness.get_by_label("Plug-in drawing").rect();
    harness.drag_at(rect.center());
    harness.run();
    harness.hover_at(rect.center() + vec2(10.0, 0.0));
    harness.run();
    harness.hover_at(rect.center() + vec2(20.0, 0.0));
    harness.run();
    harness.drop_at(rect.center() + vec2(20.0, 0.0));
    harness.run();
    let phases: Vec<PointerPhase> = harness
        .state()
        .iter()
        .filter_map(|event| match event.kind {
            UiEventKind::Pointer(pointer) if event.id == 11 => Some(pointer.phase),
            _ => None,
        })
        .collect();
    assert_eq!(phases.first(), Some(&PointerPhase::Down), "{phases:?}");
    assert!(phases.contains(&PointerPhase::Drag), "{phases:?}");
    assert_eq!(phases.last(), Some(&PointerPhase::Up), "{phases:?}");
}

fn canvas_events(harness: &Harness<'static, Vec<UiEvent>>) -> Vec<UiEventKind> {
    harness.state().iter().filter(|event| event.id == 11).map(|event| event.kind.clone()).collect()
}

#[test]
fn a_canvas_reached_with_tab_gets_arrow_keys_with_their_modifiers_and_keeps_the_focus() {
    let button = Widget::Button { id: 10, text: "Before".into(), enabled: true };
    let mut harness = harness(vec![button, Widget::Canvas { id: 11, size: [100.0, 60.0], shapes: vec![] }]);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.get_by_label("Plug-in drawing").is_focused());
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowLeft);
    harness.run();
    harness.key_press(egui::Key::ArrowDown);
    harness.run();
    let shift = Modifiers { shift: true, alt: false, command: false };
    assert_eq!(
        canvas_events(&harness),
        vec![UiEventKind::Key(Key { name: "Left".into(), modifiers: shift }), UiEventKind::Key(Key { name: "Down".into(), modifiers: Modifiers::default() })]
    );
    assert!(harness.get_by_label("Plug-in drawing").is_focused());
}

#[test]
fn every_pointer_move_of_a_frame_reaches_the_canvas_in_order() {
    let mut harness = harness(vec![Widget::Canvas { id: 11, size: [100.0, 100.0], shapes: vec![] }]);
    harness.run();
    let origin = harness.get_by_label("Plug-in drawing").rect().min;
    harness.drag_at(origin + vec2(10.0, 10.0));
    harness.run();
    harness.state_mut().clear();
    for x in [20.0, 30.0, 40.0] {
        harness.event(egui::Event::PointerMoved(origin + vec2(x, 10.0)));
    }
    harness.run();
    let drags: Vec<[f32; 2]> = canvas_events(&harness)
        .into_iter()
        .filter_map(|kind| match kind {
            UiEventKind::Pointer(Pointer { phase: PointerPhase::Drag, pos, .. }) => Some(pos),
            _ => None,
        })
        .collect();
    assert_eq!(drags, vec![[20.0, 10.0], [30.0, 10.0], [40.0, 10.0]]);
}
