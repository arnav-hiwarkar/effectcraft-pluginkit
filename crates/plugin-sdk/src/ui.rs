use std::collections::HashMap;
use std::ops::RangeInclusive;

use crate::wire::{AppEvent, Drag, Dropped, Key, MIN_TIMER_MS, Paint, PanelReply, Pointer, Shape, TextStyle, Timer, Topic, UiEventKind, UiInput, Widget};

pub trait Panel: Send + 'static {
    fn ui(&mut self, ui: &mut Ui);
}

pub struct Ui<'a> {
    input: &'a UiInput,
    widgets: Vec<Widget>,
    seen: HashMap<u64, u32>,
    scope: u64,
    topics: Vec<Topic>,
    timers: Vec<Timer>,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn hash(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

impl<'a> Ui<'a> {
    pub fn new(input: &'a UiInput) -> Self {
        Ui { input, widgets: Vec::new(), seen: HashMap::new(), scope: FNV_OFFSET, topics: Vec::new(), timers: Vec::new() }
    }

    pub fn finish(self) -> Vec<Widget> {
        self.widgets
    }

    pub fn reply(self) -> PanelReply {
        PanelReply { widgets: self.widgets, topics: self.topics, timers: self.timers, legacy: false }
    }

    pub fn available(&self) -> [f32; 2] {
        self.input.size
    }

    pub fn keys(&self) -> &'a str {
        &self.input.keys
    }

    pub fn on(&mut self, topic: Topic) -> Option<&'a AppEvent> {
        if !self.topics.contains(&topic) {
            self.topics.push(topic);
        }
        self.input.app.iter().find(|event| event.topic() == topic)
    }

    pub fn every(&mut self, name: &str, ms: u32) -> bool {
        self.timer(name, ms, true)
    }

    pub fn after(&mut self, name: &str, ms: u32) -> bool {
        self.timer(name, ms, false)
    }

    fn timer(&mut self, name: &str, ms: u32, repeat: bool) -> bool {
        self.timers.retain(|timer| timer.name != name);
        self.timers.push(Timer { name: name.into(), ms: ms.max(MIN_TIMER_MS), repeat });
        self.input.timers.iter().any(|fired| fired == name)
    }

    fn id(&mut self, key: &str) -> u64 {
        let base = hash(self.scope, key.as_bytes());
        let nth = self.seen.entry(base).or_insert(0);
        *nth += 1;
        hash(base, &nth.to_le_bytes())
    }

    fn event(&self, id: u64) -> Option<&'a UiEventKind> {
        self.events_of(id).last()
    }

    fn events_of(&self, id: u64) -> impl DoubleEndedIterator<Item = &'a UiEventKind> + use<'a> {
        self.input.events.iter().filter(move |e| e.id == id).map(|e| &e.kind)
    }

    fn text(&mut self, text: &str, style: TextStyle) {
        self.widgets.push(Widget::Label { text: text.into(), style });
    }

    pub fn label(&mut self, text: &str) {
        self.text(text, TextStyle::Body)
    }

    pub fn heading(&mut self, text: &str) {
        self.text(text, TextStyle::Heading)
    }

    pub fn weak(&mut self, text: &str) {
        self.text(text, TextStyle::Weak)
    }

    pub fn mono(&mut self, text: &str) {
        self.text(text, TextStyle::Mono)
    }

    pub fn error(&mut self, text: &str) {
        self.text(text, TextStyle::Error)
    }

    pub fn button(&mut self, text: &str) -> bool {
        self.button_enabled(text, true)
    }

    pub fn button_enabled(&mut self, text: &str, enabled: bool) -> bool {
        let id = self.id(text);
        self.widgets.push(Widget::Button { id, text: text.into(), enabled });
        enabled && matches!(self.event(id), Some(UiEventKind::Click))
    }

    pub fn checkbox(&mut self, text: &str, value: &mut bool) -> bool {
        let id = self.id(text);
        let changed = match self.event(id) {
            Some(UiEventKind::Bool(new)) => std::mem::replace(value, *new) != *new,
            _ => false,
        };
        self.widgets.push(Widget::Checkbox { id, text: text.into(), value: *value });
        changed
    }

    fn number_event(&self, id: u64, value: &mut f64) -> bool {
        match self.event(id) {
            Some(UiEventKind::Number(new)) => std::mem::replace(value, *new) != *new,
            _ => false,
        }
    }

    pub fn slider(&mut self, text: &str, value: &mut f64, range: RangeInclusive<f64>) -> bool {
        let id = self.id(text);
        let changed = self.number_event(id, value);
        self.widgets.push(Widget::Slider { id, text: text.into(), value: *value, min: *range.start(), max: *range.end() });
        changed
    }

    pub fn number(&mut self, text: &str, value: &mut f64, speed: f64) -> bool {
        let id = self.id(text);
        let changed = self.number_event(id, value);
        self.widgets.push(Widget::Number { id, text: text.into(), value: *value, speed, range: None, decimals: None });
        changed
    }

    pub fn number_in(&mut self, text: &str, value: &mut f64, speed: f64, range: RangeInclusive<f64>, decimals: u8) -> bool {
        let id = self.id(text);
        let before = *value;
        let edited = self.number_event(id, value);
        *value = value.max(*range.start()).min(*range.end());
        self.widgets.push(Widget::Number {
            id,
            text: text.into(),
            value: *value,
            speed,
            range: Some([*range.start(), *range.end()]),
            decimals: Some(decimals),
        });
        edited && *value != before
    }

    pub fn text_edit(&mut self, key: &str, value: &mut String, hint: &str) -> bool {
        let id = self.id(key);
        let changed = match self.event(id) {
            Some(UiEventKind::Text(new)) if new != value => {
                value.clone_from(new);
                true
            }
            _ => false,
        };
        self.widgets.push(Widget::TextEdit { id, value: value.clone(), hint: hint.into() });
        changed
    }

    pub fn popup(&mut self, text: &str, selected: &mut usize, options: &[&str]) -> bool {
        let id = self.id(text);
        let changed = match self.event(id) {
            Some(UiEventKind::Number(new)) => std::mem::replace(selected, *new as usize) != *new as usize,
            _ => false,
        };
        let options = options.iter().map(|o| o.to_string()).collect();
        self.widgets.push(Widget::Popup { id, text: text.into(), options, selected: *selected as u32 });
        changed
    }

    pub fn color(&mut self, text: &str, value: &mut [f32; 4]) -> bool {
        let id = self.id(text);
        let changed = match self.event(id) {
            Some(UiEventKind::Color(new)) => std::mem::replace(value, *new) != *new,
            _ => false,
        };
        self.widgets.push(Widget::Color { id, text: text.into(), value: *value });
        changed
    }

    pub fn separator(&mut self) {
        self.widgets.push(Widget::Separator);
    }

    pub fn space(&mut self, size: f32) {
        self.widgets.push(Widget::Space { size });
    }

    pub fn keyed(&mut self, key: &str, build: impl FnOnce(&mut Ui<'a>)) {
        let scope = hash(hash(self.scope, b"keyed\x1f"), key.as_bytes());
        let outer_scope = std::mem::replace(&mut self.scope, scope);
        build(self);
        self.scope = outer_scope;
    }

    fn nested(&mut self, key: &str, build: impl FnOnce(&mut Ui<'a>)) -> Vec<Widget> {
        let scope = self.id(key);
        self.nested_in(scope, build)
    }

    pub fn row(&mut self, build: impl FnOnce(&mut Ui<'a>)) {
        let children = self.nested("row", build);
        self.widgets.push(Widget::Row { children });
    }

    pub fn column(&mut self, build: impl FnOnce(&mut Ui<'a>)) {
        let children = self.nested("column", build);
        self.widgets.push(Widget::Column { children });
    }

    pub fn group(&mut self, title: &str, build: impl FnOnce(&mut Ui<'a>)) {
        let children = self.nested(title, build);
        self.widgets.push(Widget::Group { title: title.into(), children });
    }

    pub fn canvas(&mut self, key: &str, size: [f32; 2], draw: impl FnOnce(&mut Canvas)) {
        let id = self.id(key);
        let mut canvas = Canvas { size, pointer: None, pointers: Vec::new(), keys: Vec::new(), shapes: Vec::new() };
        for kind in self.events_of(id) {
            match kind {
                UiEventKind::Pointer(pointer) => canvas.pointers.push(*pointer),
                UiEventKind::Key(key) => canvas.keys.push(key.clone()),
                _ => {}
            }
        }
        canvas.pointer = canvas.pointers.last().copied();
        draw(&mut canvas);
        self.widgets.push(Widget::Canvas { id, size, shapes: canvas.shapes });
    }

    pub fn image(&mut self, key: &str, size: [f32; 2]) -> bool {
        let id = self.id(key);
        self.widgets.push(Widget::Image { id, key: key.into(), size });
        matches!(self.event(id), Some(UiEventKind::Click))
    }

    pub fn scroll(&mut self, key: &str, height: f32, build: impl FnOnce(&mut Ui<'a>)) {
        let id = self.id(key);
        let children = self.nested_in(id, build);
        self.widgets.push(Widget::Scroll { id, height, children });
    }

    pub fn collapsing(&mut self, title: &str, open_by_default: bool, build: impl FnOnce(&mut Ui<'a>)) {
        let id = self.id(title);
        let children = self.nested_in(id, build);
        self.widgets.push(Widget::Collapsing { id, title: title.into(), open: open_by_default, children });
    }

    pub fn tabs(&mut self, selected: &mut usize, options: &[&str]) -> bool {
        let id = self.id(&options.join("\u{1f}"));
        let changed = match self.event(id) {
            Some(UiEventKind::Number(new)) => std::mem::replace(selected, *new as usize) != *new as usize,
            _ => false,
        };
        let options = options.iter().map(|o| o.to_string()).collect();
        self.widgets.push(Widget::Tabs { id, options, selected: *selected as u32 });
        changed
    }

    pub fn progress(&mut self, fraction: f32, text: &str) {
        self.widgets.push(Widget::Progress { fraction, text: text.into() });
    }

    pub fn tooltip(&mut self, text: &str) {
        self.widgets.push(Widget::Tooltip { text: text.into() });
    }

    pub fn grid(&mut self, columns: u32, build: impl FnOnce(&mut Ui<'a>)) {
        let id = self.id("grid");
        let children = self.nested_in(id, build);
        self.widgets.push(Widget::Grid { id, columns, children });
    }

    pub fn drop_target(&mut self, key: &str, build: impl FnOnce(&mut Ui<'a>)) -> Option<&'a Dropped> {
        let id = self.id(key);
        let children = self.nested_in(id, build);
        self.widgets.push(Widget::DropTarget { id, children });
        self.events_of(id).rev().find_map(|kind| match kind {
            UiEventKind::Drop(dropped) => Some(dropped),
            _ => None,
        })
    }

    pub fn drag_source(&mut self, key: &str, drag: Drag, build: impl FnOnce(&mut Ui<'a>)) {
        let id = self.id(key);
        let children = self.nested_in(id, build);
        self.widgets.push(Widget::DragSource { id, drag, children });
    }

    fn nested_in(&mut self, scope: u64, build: impl FnOnce(&mut Ui<'a>)) -> Vec<Widget> {
        let outer_scope = std::mem::replace(&mut self.scope, scope);
        let outer_widgets = std::mem::take(&mut self.widgets);
        build(self);
        self.scope = outer_scope;
        std::mem::replace(&mut self.widgets, outer_widgets)
    }
}

pub struct Canvas {
    pub size: [f32; 2],
    pub pointer: Option<Pointer>,
    pub pointers: Vec<Pointer>,
    pub keys: Vec<Key>,
    shapes: Vec<Shape>,
}

impl Canvas {
    pub fn line(&mut self, points: &[[f32; 2]], color: Paint, width: f32) {
        self.shapes.push(Shape::Line { points: points.to_vec(), color, width });
    }

    pub fn bezier(&mut self, points: [[f32; 2]; 4], color: Paint, width: f32) {
        self.shapes.push(Shape::Bezier { points, color, width });
    }

    pub fn rect(&mut self, min: [f32; 2], max: [f32; 2], fill: Option<Paint>, stroke: Option<(Paint, f32)>, radius: f32) {
        self.shapes.push(Shape::Rect { min, max, fill, stroke, radius });
    }

    pub fn circle(&mut self, center: [f32; 2], radius: f32, fill: Option<Paint>, stroke: Option<(Paint, f32)>) {
        self.shapes.push(Shape::Circle { center, radius, fill, stroke });
    }

    pub fn text(&mut self, pos: [f32; 2], text: &str, color: Paint, size: f32) {
        self.shapes.push(Shape::Text { pos, text: text.into(), color, size });
    }

    pub fn polygon(&mut self, points: &[[f32; 2]], fill: Paint) {
        self.shapes.push(Shape::Polygon { points: points.to_vec(), fill });
    }

    pub fn image(&mut self, key: &str, min: [f32; 2], max: [f32; 2]) {
        self.shapes.push(Shape::Image { key: key.into(), min, max });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Modifiers, PointerPhase, UiEvent};

    fn frame(events: Vec<UiEvent>, build: impl FnOnce(&mut Ui)) -> Vec<Widget> {
        let input = UiInput { events, size: [200.0, 100.0], ..Default::default() };
        let mut ui = Ui::new(&input);
        build(&mut ui);
        ui.finish()
    }

    fn id_of(widget: &Widget) -> u64 {
        match widget {
            Widget::Button { id, .. } | Widget::Slider { id, .. } | Widget::Number { id, .. } | Widget::Checkbox { id, .. } => *id,
            other => panic!("no id on {other:?}"),
        }
    }

    #[test]
    fn number_in_clamps_a_value_outside_its_range_but_reports_a_change_only_for_an_edit() {
        let mut value = 150.0;
        let mut changed = Vec::new();
        let widgets = frame(vec![], |ui| changed.push(ui.number_in("Size", &mut value, 1.0, 0.0..=100.0, 0)));
        let id = id_of(&widgets[0]);
        frame(vec![UiEvent { id, kind: UiEventKind::Number(40.0) }], |ui| changed.push(ui.number_in("Size", &mut value, 1.0, 0.0..=100.0, 0)));
        frame(vec![UiEvent { id, kind: UiEventKind::Number(400.0) }], |ui| changed.push(ui.number_in("Size", &mut value, 1.0, 0.0..=100.0, 0)));
        assert_eq!((changed, value), (vec![false, true, true], 100.0));
    }

    #[test]
    fn a_click_reaches_only_the_button_it_was_made_on_even_with_equal_labels() {
        let first = frame(vec![], |ui| {
            ui.button("Apply");
            ui.row(|ui| {
                ui.button("Apply");
            });
        });
        let Widget::Row { children } = &first[1] else { panic!("row expected") };
        let inner = id_of(&children[0]);
        assert_ne!(id_of(&first[0]), inner);

        let mut clicks = (false, false);
        frame(vec![UiEvent { id: inner, kind: UiEventKind::Click }], |ui| {
            clicks.0 = ui.button("Apply");
            ui.row(|ui| clicks.1 = ui.button("Apply"));
        });
        assert_eq!(clicks, (false, true));
    }

    fn presets(ui: &mut Ui, names: &[&str]) -> Vec<String> {
        let mut deleted = Vec::new();
        for name in names {
            ui.keyed(name, |ui| {
                ui.row(|ui| {
                    ui.label(name);
                    if ui.button("Delete") {
                        deleted.push(name.to_string());
                    }
                });
            });
        }
        deleted
    }

    #[test]
    fn a_click_on_a_keyed_list_item_reaches_that_item_after_an_item_is_inserted_above_it() {
        let shown = frame(vec![], |ui| {
            presets(ui, &["warm", "cool", "dusk"]);
        });
        let Widget::Row { children } = &shown[2] else { panic!("row expected") };
        let dusk = id_of(&children[1]);
        let mut deleted = Vec::new();
        frame(vec![UiEvent { id: dusk, kind: UiEventKind::Click }], |ui| deleted = presets(ui, &["new", "warm", "cool", "dusk"]));
        assert_eq!(deleted, vec!["dusk".to_string()]);
    }

    #[test]
    fn a_slider_event_updates_the_value_and_the_widget_shows_the_new_value() {
        let id = id_of(
            &frame(vec![], |ui| {
                ui.slider("Speed", &mut 1.0, 0.0..=10.0);
            })[0],
        );
        let mut speed = 1.0;
        let mut changed = false;
        let widgets = frame(vec![UiEvent { id, kind: UiEventKind::Number(7.5) }], |ui| {
            changed = ui.slider("Speed", &mut speed, 0.0..=10.0);
        });
        assert!(changed);
        assert_eq!(speed, 7.5);
        assert!(matches!(widgets[0], Widget::Slider { value, .. } if value == 7.5));
    }

    #[test]
    fn a_number_in_a_range_declares_it_and_keeps_an_event_inside_it() {
        let id = id_of(
            &frame(vec![], |ui| {
                ui.number_in("Count", &mut 5.0, 1.0, 0.0..=100.0, 0);
            })[0],
        );
        let mut count = 5.0;
        let mut changed = false;
        let widgets = frame(vec![UiEvent { id, kind: UiEventKind::Number(150.0) }], |ui| {
            changed = ui.number_in("Count", &mut count, 1.0, 0.0..=100.0, 0);
        });
        assert!(changed);
        assert_eq!(count, 100.0);
        assert_eq!(widgets[0], Widget::Number { id, text: "Count".into(), value: 100.0, speed: 1.0, range: Some([0.0, 100.0]), decimals: Some(0) });
        let plain = frame(vec![], |ui| {
            ui.number("Count", &mut 5.0, 1.0);
        });
        assert_eq!(plain[0], Widget::Number { id, text: "Count".into(), value: 5.0, speed: 1.0, range: None, decimals: None });
    }

    #[test]
    fn ids_stay_stable_when_an_unrelated_widget_appears_above() {
        let before = id_of(
            &frame(vec![], |ui| {
                ui.button("Delete");
            })[0],
        );
        let after = frame(vec![], |ui| {
            ui.label("3 items");
            ui.checkbox("Confirm", &mut false);
            ui.button("Delete");
        });
        assert_eq!(id_of(&after[2]), before);
    }

    #[test]
    fn an_image_is_a_widget_with_its_key_and_size_and_a_click_on_it_returns_true() {
        let widgets = frame(vec![], |ui| {
            ui.image("thumb/3", [64.0, 48.0]);
        });
        let Widget::Image { id, key, size } = &widgets[0] else { panic!("image expected, got {widgets:?}") };
        assert_eq!((key.as_str(), *size), ("thumb/3", [64.0, 48.0]));
        let mut clicked = false;
        frame(vec![UiEvent { id: *id, kind: UiEventKind::Click }], |ui| clicked = ui.image("thumb/3", [64.0, 48.0]));
        assert!(clicked);
    }

    #[test]
    fn scroll_collapsing_and_grid_hold_their_children_with_ids_distinct_from_the_same_widget_outside() {
        let widgets = frame(vec![], |ui| {
            ui.button("Go");
            ui.scroll("list", 120.0, |ui| {
                ui.button("Go");
            });
            ui.collapsing("More", false, |ui| ui.label("hidden"));
            ui.grid(3, |ui| {
                ui.label("a");
                ui.label("b");
            });
        });
        let Widget::Scroll { height, children, .. } = &widgets[1] else { panic!("scroll expected") };
        assert_eq!(*height, 120.0);
        assert_ne!(id_of(&children[0]), id_of(&widgets[0]));
        assert!(
            matches!(&widgets[2], Widget::Collapsing { title, open: false, children, .. } if title == "More" && matches!(&children[..], [Widget::Label { text, style: TextStyle::Body, .. }] if text == "hidden"))
        );
        assert!(matches!(&widgets[3], Widget::Grid { columns: 3, children, .. } if children.len() == 2));
    }

    #[test]
    fn tabs_take_the_chosen_index_and_progress_and_tooltip_carry_their_text() {
        let Widget::Tabs { id, .. } = frame(vec![], |ui| {
            ui.tabs(&mut 0, &["A", "B"]);
        })[0]
            .clone()
        else {
            panic!("tabs expected")
        };
        let mut selected = 0;
        let mut changed = false;
        let widgets = frame(vec![UiEvent { id, kind: UiEventKind::Number(1.0) }], |ui| {
            changed = ui.tabs(&mut selected, &["A", "B"]);
            ui.progress(0.25, "1 of 4");
            ui.tooltip("Apply the preset");
        });
        assert_eq!((changed, selected), (true, 1));
        assert!(matches!(&widgets[0], Widget::Tabs { selected: 1, options, .. } if options == &vec!["A".to_string(), "B".to_string()]));
        assert_eq!(widgets[1], Widget::Progress { fraction: 0.25, text: "1 of 4".into() });
        assert_eq!(widgets[2], Widget::Tooltip { text: "Apply the preset".into() });
    }

    #[test]
    fn a_canvas_gets_polygons_images_and_the_pointer_with_modifiers_and_scroll() {
        let Widget::Canvas { id, .. } = frame(vec![], |ui| ui.canvas("graph", [100.0, 100.0], |_| {}))[0].clone() else { panic!("canvas expected") };
        let pointer = Pointer {
            pos: [4.0, 5.0],
            phase: PointerPhase::Down,
            modifiers: crate::wire::Modifiers { shift: false, alt: true, command: false },
            scroll: [0.0, -3.0],
        };
        let mut seen = None;
        let widgets = frame(vec![UiEvent { id, kind: UiEventKind::Pointer(pointer) }], |ui| {
            ui.canvas("graph", [100.0, 100.0], |c| {
                seen = c.pointer;
                c.polygon(&[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]], Paint::Accent);
                c.image("bg", [0.0, 0.0], [100.0, 100.0]);
            })
        });
        assert_eq!(seen, Some(pointer));
        let Widget::Canvas { shapes, .. } = &widgets[0] else { panic!("canvas expected") };
        assert_eq!(shapes[0], Shape::Polygon { points: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]], fill: Paint::Accent });
        assert_eq!(shapes[1], Shape::Image { key: "bg".into(), min: [0.0, 0.0], max: [100.0, 100.0] });
    }

    #[test]
    fn a_pointer_from_an_older_host_without_modifiers_still_reads() {
        let pointer: Pointer = serde_json::from_str(r#"{"pos":[1.0,2.0],"phase":"drag"}"#).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(pointer, Pointer::new([1.0, 2.0], PointerPhase::Drag));
    }

    #[test]
    fn keys_typed_while_the_panel_has_focus_reach_the_ui() {
        let input = UiInput { keys: "ab".into(), ..Default::default() };
        assert_eq!(Ui::new(&input).keys(), "ab");
    }

    fn input(events: Vec<UiEvent>, app: Vec<AppEvent>, timers: &[&str]) -> UiInput {
        UiInput { events, app, timers: timers.iter().map(|t| t.to_string()).collect(), size: [200.0, 100.0], ..Default::default() }
    }

    #[test]
    fn a_canvas_sees_every_pointer_and_key_of_the_call_in_order_and_its_pointer_is_the_last() {
        let Widget::Canvas { id, .. } = frame(vec![], |ui| ui.canvas("pad", [50.0, 50.0], |_| {}))[0].clone() else { panic!("canvas expected") };
        let shift = Modifiers { shift: true, alt: false, command: false };
        let events = vec![
            UiEvent { id, kind: UiEventKind::Pointer(Pointer::new([1.0, 1.0], PointerPhase::Down)) },
            UiEvent { id, kind: UiEventKind::Key(Key { name: "ArrowLeft".into(), modifiers: shift }) },
            UiEvent { id, kind: UiEventKind::Pointer(Pointer::new([2.0, 1.0], PointerPhase::Drag)) },
            UiEvent { id: id + 1, kind: UiEventKind::Pointer(Pointer::new([9.0, 9.0], PointerPhase::Up)) },
        ];
        let (mut pointers, mut keys, mut last) = (vec![], vec![], None);
        frame(events, |ui| {
            ui.canvas("pad", [50.0, 50.0], |c| {
                pointers = c.pointers.iter().map(|p| (p.pos, p.phase)).collect();
                keys = c.keys.clone();
                last = c.pointer.map(|p| p.pos);
            })
        });
        assert_eq!(pointers, vec![([1.0, 1.0], PointerPhase::Down), ([2.0, 1.0], PointerPhase::Drag)]);
        assert_eq!(keys, vec![Key { name: "ArrowLeft".into(), modifiers: shift }]);
        assert_eq!(last, Some([2.0, 1.0]));
    }

    #[test]
    fn a_panel_subscribes_and_arms_timers_in_its_reply_and_sees_what_was_delivered() {
        let selection = AppEvent::Selection { layers: vec![3], properties: vec![], keyframes: 0 };
        let delivered = input(vec![], vec![selection.clone()], &["tick"]);
        let mut ui = Ui::new(&delivered);
        assert_eq!(ui.on(Topic::Selection), Some(&selection));
        assert_eq!(ui.on(Topic::Time), None);
        assert!(ui.every("tick", 5));
        assert!(!ui.after("hide", 2000));
        let reply = ui.reply();
        assert_eq!(reply.topics, vec![Topic::Selection, Topic::Time]);
        assert_eq!(reply.timers, vec![Timer { name: "tick".into(), ms: MIN_TIMER_MS, repeat: true }, Timer { name: "hide".into(), ms: 2000, repeat: false }]);
    }

    #[test]
    fn a_drop_target_returns_what_was_dropped_on_it() {
        let widgets = frame(vec![], |ui| {
            ui.drop_target("bin", |ui| ui.label("Drop here"));
        });
        let Widget::DropTarget { id, children } = &widgets[0] else { panic!("drop target expected, got {widgets:?}") };
        assert_eq!(children, &vec![Widget::Label { text: "Drop here".into(), style: TextStyle::Body }]);
        let dropped = Dropped::Files { paths: vec!["/tmp/a.png".into()] };
        let mut got = None;
        frame(vec![UiEvent { id: *id, kind: UiEventKind::Drop(dropped.clone()) }], |ui| got = ui.drop_target("bin", |ui| ui.label("Drop here")).cloned());
        assert_eq!(got, Some(dropped));
    }

    #[test]
    fn a_reply_from_an_older_sdk_is_a_bare_widget_list_and_reads_as_a_legacy_reply() {
        let old: PanelReply = serde_json::from_str(r#"[{"w":"label","text":"hi","style":"body"}]"#).unwrap_or_else(|e| panic!("{e}"));
        assert!(old.legacy);
        assert_eq!(old.widgets, vec![Widget::Label { text: "hi".into(), style: TextStyle::Body }]);
        let new: PanelReply = serde_json::from_value(serde_json::to_value(PanelReply { topics: vec![Topic::Time], ..Default::default() }).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((new.legacy, new.topics), (false, vec![Topic::Time]));
    }
}
