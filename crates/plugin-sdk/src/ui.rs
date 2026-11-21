use std::collections::HashMap;
use std::ops::RangeInclusive;

use crate::wire::{Paint, Pointer, Shape, TextStyle, UiEvent, UiEventKind, UiInput, Widget};

pub trait Panel: Send + 'static {
    fn ui(&mut self, ui: &mut Ui);
}

pub struct Ui<'a> {
    events: &'a [UiEvent],
    size: [f32; 2],
    widgets: Vec<Widget>,
    seen: HashMap<u64, u32>,
    scope: u64,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn hash(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

impl<'a> Ui<'a> {
    pub fn new(input: &'a UiInput) -> Self {
        Ui { events: &input.events, size: input.size, widgets: Vec::new(), seen: HashMap::new(), scope: FNV_OFFSET }
    }

    pub fn finish(self) -> Vec<Widget> {
        self.widgets
    }

    pub fn available(&self) -> [f32; 2] {
        self.size
    }

    fn id(&mut self, key: &str) -> u64 {
        let base = hash(self.scope, key.as_bytes());
        let nth = self.seen.entry(base).or_insert(0);
        *nth += 1;
        hash(base, &nth.to_le_bytes())
    }

    fn event(&self, id: u64) -> Option<&'a UiEventKind> {
        self.events.iter().rev().find(|e| e.id == id).map(|e| &e.kind)
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
        self.widgets.push(Widget::Number { id, text: text.into(), value: *value, speed });
        changed
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

    fn nested(&mut self, key: &str, build: impl FnOnce(&mut Ui<'a>)) -> Vec<Widget> {
        let scope = self.id(key);
        let outer_scope = std::mem::replace(&mut self.scope, scope);
        let outer_widgets = std::mem::take(&mut self.widgets);
        build(self);
        self.scope = outer_scope;
        std::mem::replace(&mut self.widgets, outer_widgets)
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
        let pointer = match self.event(id) {
            Some(UiEventKind::Pointer(pointer)) => Some(*pointer),
            _ => None,
        };
        let mut canvas = Canvas { size, pointer, shapes: Vec::new() };
        draw(&mut canvas);
        self.widgets.push(Widget::Canvas { id, size, shapes: canvas.shapes });
    }
}

pub struct Canvas {
    pub size: [f32; 2],
    pub pointer: Option<Pointer>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(events: Vec<UiEvent>, build: impl FnOnce(&mut Ui)) -> Vec<Widget> {
        let input = UiInput { events, size: [200.0, 100.0] };
        let mut ui = Ui::new(&input);
        build(&mut ui);
        ui.finish()
    }

    fn id_of(widget: &Widget) -> u64 {
        match widget {
            Widget::Button { id, .. } | Widget::Slider { id, .. } | Widget::Checkbox { id, .. } => *id,
            other => panic!("no id on {other:?}"),
        }
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
}
