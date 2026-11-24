use craft_plugin::{Mark, Paint, Shape, Space};
use egui::{Color32, Stroke};

use crate::{Theme, Triangulate};

const CURVE_STEPS: usize = 48;

pub fn draw_marks(t: &Theme, painter: &egui::Painter, marks: &[Mark], triangulate: Triangulate, screen: impl Fn(Space, [f64; 2]) -> Option<egui::Pos2>) {
    for mark in marks {
        let to_screen = |p: [f32; 2]| -> Option<egui::Pos2> { screen(mark.space, [f64::from(p[0]), f64::from(p[1])]).filter(|s| s.is_finite()) };
        let path = |points: Vec<[f32; 2]>, least: usize| {
            let mut screen = points.into_iter().map(to_screen).collect::<Option<Vec<_>>>()?;
            screen.dedup();
            (screen.len() >= least).then_some(screen)
        };
        let outline = |points: Vec<[f32; 2]>, fill: Option<Paint>, stroke: Option<(Paint, f32)>| {
            let Some(points) = path(points, 3) else { return };
            let fill = fill.map_or(Color32::TRANSPARENT, |paint| t.paint(paint));
            let stroke = stroke.map_or(Stroke::NONE, |(paint, width)| Stroke::new(width.clamp(0.0, 16.0), t.paint(paint)));
            painter.add(egui::Shape::Path(egui::epaint::PathShape { points, closed: true, fill, stroke: stroke.into() }));
        };
        match &mark.shape {
            Shape::Line { points, color, width } => {
                if let Some(points) = path(points.clone(), 2) {
                    painter.add(egui::Shape::line(points, Stroke::new(width.clamp(0.0, 16.0), t.paint(*color))));
                }
            }
            Shape::Bezier { points: [a, b, c, d], color, width } => {
                let at = |s: f32| {
                    let r = 1.0 - s;
                    std::array::from_fn(|i| r * r * r * a[i] + 3.0 * r * r * s * b[i] + 3.0 * r * s * s * c[i] + s * s * s * d[i])
                };
                if let Some(points) = path((0..=CURVE_STEPS).map(|i| at(i as f32 / CURVE_STEPS as f32)).collect(), 2) {
                    painter.add(egui::Shape::line(points, Stroke::new(width.clamp(0.0, 16.0), t.paint(*color))));
                }
            }
            Shape::Rect { min, max, fill, stroke, .. } => outline(vec![*min, [max[0], min[1]], *max, [min[0], max[1]]], *fill, *stroke),
            Shape::Circle { center, radius, fill, stroke } => {
                let around = (0..CURVE_STEPS).map(|i| {
                    let a = i as f32 / CURVE_STEPS as f32 * std::f32::consts::TAU;
                    [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
                });
                outline(around.collect(), *fill, *stroke)
            }
            Shape::Polygon { points, fill } => {
                if let Some(points) = path(points.clone(), 3) {
                    painter.add(filled_polygon(points, t.paint(*fill), triangulate));
                }
            }
            Shape::Text { pos, text, color, size } => {
                if let Some(at) = to_screen(*pos) {
                    painter.text(at, egui::Align2::LEFT_TOP, text, egui::FontId::proportional(size.clamp(6.0, 48.0)), t.paint(*color));
                }
            }
            Shape::Image { .. } => {}
        }
    }
}

const MAX_CONCAVE_POINTS: usize = 2048;

pub fn filled_polygon(mut points: Vec<egui::Pos2>, fill: Color32, triangulate: Triangulate) -> egui::Shape {
    points.dedup();
    while points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() > MAX_CONCAVE_POINTS {
        return egui::Shape::convex_polygon(points, fill, Stroke::NONE);
    }
    let outline: Vec<[f64; 2]> = points.iter().map(|point| [f64::from(point.x), f64::from(point.y)]).collect();
    if crosses_itself(&outline) {
        return egui::Shape::Noop;
    }
    let mut mesh = egui::Mesh::default();
    mesh.vertices.extend(points.iter().map(|point| egui::epaint::Vertex { pos: *point, uv: egui::epaint::WHITE_UV, color: fill }));
    for [a, b, c] in triangulate(&outline) {
        if side(outline[a], outline[b], outline[c]) != 0.0 {
            mesh.add_triangle(a as u32, b as u32, c as u32);
        }
    }
    egui::Shape::mesh(mesh)
}

fn side(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn crosses_itself(outline: &[[f64; 2]]) -> bool {
    let n = outline.len();
    let edge = |i: usize| (outline[i], outline[(i + 1) % n]);
    (0..n).any(|i| {
        let (a, b) = edge(i);
        (i + 2..n).filter(|&j| (j + 1) % n != i).any(|j| {
            let (c, d) = edge(j);
            side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
        })
    })
}
