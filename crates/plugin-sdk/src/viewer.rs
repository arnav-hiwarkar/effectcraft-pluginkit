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

fn f32s(point: [f64; 2]) -> [f32; 2] {
    point.map(|c| c as f32)
}

impl Overlay {
    pub fn new() -> Self {
        Overlay::default()
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
