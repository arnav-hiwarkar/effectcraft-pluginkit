use serde::{Deserialize, Serialize};

use crate::Result;
use crate::effect::Cx;
use crate::wire::{Modifiers, Paint, Shape};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Space {
    #[default]
    Layer,
    Comp,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    #[serde(default)]
    pub space: Space,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "bind", rename_all = "snake_case")]
pub enum Bind {
    Point { param: String },
    Angle { param: String, centre: [f64; 2] },
    Radius { param: String, centre: [f64; 2], unit: f64 },
    Rect { min: String, max: String },
    Custom { param: String, key: String },
}

impl Bind {
    pub fn params(&self) -> Vec<&str> {
        match self {
            Bind::Point { param } | Bind::Angle { param, .. } | Bind::Radius { param, .. } | Bind::Custom { param, .. } => vec![param],
            Bind::Rect { min, max } => vec![min, max],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Handle {
    pub at: [f64; 2],
    #[serde(flatten)]
    pub bind: Bind,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub handles: Vec<Handle>,
    #[serde(skip)]
    space: Space,
    #[serde(skip)]
    paint: Option<Paint>,
}

pub const MAX_MARKS: usize = 4096;
pub const MAX_POINTS: usize = 4096;
pub const MAX_LABEL_CHARS: usize = 512;
pub const MAX_HANDLES: usize = 256;

fn f32s(point: [f64; 2]) -> [f32; 2] {
    point.map(|c| c as f32)
}

impl Overlay {
    pub fn new() -> Self {
        Overlay::default()
    }

    pub fn capped(mut self) -> (Self, Option<String>) {
        let (marks, handles) = (self.marks.len(), self.handles.len());
        self.marks.truncate(MAX_MARKS);
        self.handles.truncate(MAX_HANDLES);
        let mut cut = marks - self.marks.len() + handles - self.handles.len();
        for mark in &mut self.marks {
            match &mut mark.shape {
                Shape::Line { points, .. } | Shape::Polygon { points, .. } if points.len() > MAX_POINTS => {
                    cut += points.len() - MAX_POINTS;
                    points.truncate(MAX_POINTS);
                }
                Shape::Text { text, .. } if text.chars().nth(MAX_LABEL_CHARS).is_some() => {
                    let end = text.char_indices().nth(MAX_LABEL_CHARS).map_or(text.len(), |(at, _)| at);
                    cut += text[end..].chars().count();
                    text.truncate(end);
                }
                _ => {}
            }
        }
        let note = (cut > 0).then(|| {
            format!(
                "the overlay drew {marks} shapes and {handles} handles; the viewer keeps the first {MAX_MARKS} shapes, {MAX_POINTS} points per line, {MAX_LABEL_CHARS} label characters and {MAX_HANDLES} handles ({cut} items dropped)"
            )
        });
        (self, note)
    }

    pub fn in_space(space: Space) -> Self {
        Overlay { space, ..Default::default() }
    }

    pub fn space(&mut self, space: Space) -> &mut Self {
        self.space = space;
        self
    }

    pub fn paint(&mut self, paint: Paint) -> &mut Self {
        self.paint = Some(paint);
        self
    }

    fn pen(&self) -> Paint {
        self.paint.unwrap_or(Paint::Accent)
    }

    pub fn shape(&mut self, shape: Shape) -> &mut Self {
        self.marks.push(Mark { shape, space: self.space });
        self
    }

    pub fn line(&mut self, points: &[[f64; 2]]) -> &mut Self {
        self.shape(Shape::Line { points: points.iter().copied().map(f32s).collect(), color: self.pen(), width: 1.5 })
    }

    pub fn circle(&mut self, centre: [f64; 2], radius: f64) -> &mut Self {
        self.shape(Shape::Circle { center: f32s(centre), radius: radius as f32, fill: None, stroke: Some((self.pen(), 1.5)) })
    }

    pub fn rectangle(&mut self, min: [f64; 2], max: [f64; 2]) -> &mut Self {
        self.shape(Shape::Rect { min: f32s(min), max: f32s(max), fill: None, stroke: Some((self.pen(), 1.5)), radius: 0.0 })
    }

    pub fn text(&mut self, at: [f64; 2], text: &str) -> &mut Self {
        self.shape(Shape::Text { pos: f32s(at), text: text.into(), color: self.pen(), size: 12.0 })
    }

    pub fn handle(&mut self, at: [f64; 2], bind: Bind) -> &mut Self {
        self.handles.push(Handle { at, bind, name: String::new() });
        self
    }

    pub fn named(&mut self, name: &str) -> &mut Self {
        if let Some(handle) = self.handles.last_mut() {
            handle.name = name.into();
        }
        self
    }

    pub fn point(&mut self, cx: &Cx, param: &str) -> &mut Self {
        self.handle(cx.point(param), Bind::Point { param: param.into() })
    }

    pub fn angle(&mut self, param: &str, centre: [f64; 2], at: [f64; 2]) -> &mut Self {
        self.handle(at, Bind::Angle { param: param.into(), centre })
    }

    pub fn radius(&mut self, param: &str, centre: [f64; 2], pixels: f64, unit: f64) -> &mut Self {
        self.circle(centre, pixels);
        self.handle([centre[0] + pixels, centre[1]], Bind::Radius { param: param.into(), centre, unit })
    }

    pub fn rect(&mut self, cx: &Cx, min: &str, max: &str) -> &mut Self {
        let ([x0, y0], [x1, y1]) = (cx.point(min), cx.point(max));
        self.rectangle([x0, y0], [x1, y1]);
        self.handle([(x0 + x1) / 2.0, (y0 + y1) / 2.0], Bind::Rect { min: min.into(), max: max.into() });
        self.point(cx, min).point(cx, max)
    }

    pub fn custom(&mut self, param: &str, key: &str, at: [f64; 2]) -> &mut Self {
        self.handle(at, Bind::Custom { param: param.into(), key: key.into() })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cursor {
    #[default]
    Crosshair,
    Arrow,
    Move,
    Grab,
    Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Vec<Shape>,
    #[serde(default)]
    pub cursor: Cursor,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolEventKind {
    Down,
    Move,
    Up,
    Scroll,
    Key,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolEvent {
    pub kind: ToolEventKind,
    pub comp: [f64; 2],
    #[serde(default)]
    pub layer: Option<[f64; 2]>,
    #[serde(default)]
    pub layer_id: Option<u64>,
    #[serde(default)]
    pub modifiers: Modifiers,
    #[serde(default)]
    pub pressure: f32,
    #[serde(default)]
    pub scroll: [f32; 2],
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub zoom: f64,
    #[serde(default)]
    pub time: f64,
}

pub trait Tool: Send + 'static {
    fn icon(&self) -> Vec<Shape> {
        vec![Shape::Circle { center: [8.0, 8.0], radius: 5.0, fill: None, stroke: Some((Paint::Text, 1.5)) }]
    }

    fn cursor(&self) -> Cursor {
        Cursor::Crosshair
    }

    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_keep_the_first_items_in_order_and_say_what_was_dropped() {
        let mut overlay = Overlay::new();
        for n in 0..5000 {
            overlay.circle([n as f64, 0.0], 1.0);
        }
        overlay.line(&vec![[1.0, 2.0]; 5000]);
        overlay.text([0.0, 0.0], &"é".repeat(600));
        for n in 0..300 {
            overlay.custom("p", "k", [n as f64, 0.0]);
        }
        let (capped, note) = overlay.capped();
        assert_eq!(capped.marks.len(), 4096);
        assert_eq!(capped.handles.len(), 256);
        assert_eq!(capped.handles[255].at, [255.0, 0.0]);
        let Shape::Circle { center, .. } = capped.marks[4095].shape else { panic!("a circle") };
        assert_eq!(center, [4095.0, 0.0]);
        let note = note.unwrap();
        assert!(note.contains("5002 shapes and 300 handles") && note.contains("(950 items dropped)"), "{note}");
    }

    #[test]
    fn long_lines_and_labels_are_cut_and_small_overlays_pass_untouched() {
        let mut overlay = Overlay::new();
        overlay.line(&vec![[1.0, 2.0]; 5000]).text([0.0, 0.0], &"é".repeat(600));
        let (capped, note) = overlay.capped();
        let Shape::Line { points, .. } = &capped.marks[0].shape else { panic!("a line") };
        let Shape::Text { text, .. } = &capped.marks[1].shape else { panic!("a text") };
        assert_eq!((points.len(), text.chars().count()), (4096, 512));
        assert!(note.unwrap().contains("(992 items dropped)"));
        let mut small = Overlay::new();
        small.circle([1.0, 1.0], 2.0);
        let (same, note) = small.clone().capped();
        assert_eq!((same, note), (small, None));
    }
}
