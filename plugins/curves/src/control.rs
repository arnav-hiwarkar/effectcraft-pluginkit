use craft_plugin::prelude::*;

use crate::curve::{CHANNELS, Curves, Point, Spline, identity};

pub const GRAB: f32 = 8.0;
pub const DROP_OFF: f32 = 24.0;
const SAMPLES: usize = 96;

#[derive(Default)]
pub struct ControlState {
    pub channel: usize,
    pub dragging: Option<usize>,
}

pub fn side(ui: &Ui) -> f32 {
    (ui.available()[0] - 8.0).clamp(120.0, 320.0)
}

pub fn channel_paint(channel: usize) -> Paint {
    match channel {
        1 => Paint::Rgba([230, 80, 80, 255]),
        2 => Paint::Rgba([90, 200, 110, 255]),
        3 => Paint::Rgba([90, 140, 240, 255]),
        4 => Paint::Weak,
        _ => Paint::Text,
    }
}

fn to_canvas(p: Point, side: f32) -> [f32; 2] {
    [p[0] as f32 * side, (1.0 - p[1] as f32) * side]
}

fn to_curve(pos: [f32; 2], side: f32) -> Point {
    [f64::from(pos[0] / side).clamp(0.0, 1.0), f64::from(1.0 - pos[1] / side).clamp(0.0, 1.0)]
}

fn nearest(points: &[Point], pos: [f32; 2], side: f32) -> Option<usize> {
    let distance = |p: &Point| {
        let [x, y] = to_canvas(*p, side);
        ((x - pos[0]).powi(2) + (y - pos[1]).powi(2)).sqrt()
    };
    points.iter().enumerate().map(|(i, p)| (i, distance(p))).filter(|(_, d)| *d <= GRAB).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

fn outside(pos: [f32; 2], side: f32) -> bool {
    pos[0] < -DROP_OFF || pos[1] < -DROP_OFF || pos[0] > side + DROP_OFF || pos[1] > side + DROP_OFF
}

pub fn edit(state: &mut ControlState, points: &mut Vec<Point>, pointer: Pointer, side: f32) -> bool {
    match pointer.phase {
        PointerPhase::Down if pointer.modifiers.alt => {
            state.dragging = None;
            match nearest(points, pointer.pos, side) {
                Some(index) if points.len() > 2 => {
                    points.remove(index);
                    true
                }
                _ => false,
            }
        }
        PointerPhase::Down => {
            if let Some(index) = nearest(points, pointer.pos, side) {
                state.dragging = Some(index);
                return false;
            }
            let point = to_curve(pointer.pos, side);
            let index = points.partition_point(|p| p[0] < point[0]);
            if points.get(index).is_some_and(|p| (p[0] - point[0]).abs() < 1e-3) {
                return false;
            }
            points.insert(index, point);
            state.dragging = Some(index);
            true
        }
        PointerPhase::Drag => {
            let Some(index) = state.dragging.filter(|i| *i < points.len()) else { return false };
            if outside(pointer.pos, side) && points.len() > 2 {
                points.remove(index);
                state.dragging = None;
                return true;
            }
            let lo = if index == 0 { 0.0 } else { points[index - 1][0] + 1e-3 };
            let hi = if index + 1 == points.len() { 1.0 } else { points[index + 1][0] - 1e-3 };
            let [x, y] = to_curve(pointer.pos, side);
            let moved = [x.clamp(lo, hi.max(lo)), y];
            let changed = points[index] != moved;
            points[index] = moved;
            changed
        }
        PointerPhase::Up => {
            state.dragging = None;
            false
        }
        PointerPhase::Hover => false,
    }
}

pub fn draw_curve(canvas: &mut Canvas, points: &[Point], side: f32, paint: Paint, width: f32) {
    let spline = Spline::new(points);
    let line: Vec<[f32; 2]> = (0..=SAMPLES)
        .map(|i| {
            let x = i as f64 / SAMPLES as f64;
            [x as f32 * side, (1.0 - spline.at(x).clamp(0.0, 1.0) as f32) * side]
        })
        .collect();
    canvas.line(&line, paint, width);
}

pub fn draw(state: &mut ControlState, value: &mut String, ui: &mut Ui) {
    let mut curves = Curves::parse(value);
    let mut changed = false;
    ui.row(|ui| {
        ui.popup("Channel", &mut state.channel, &CHANNELS);
        if ui.button("Reset") {
            curves.channels[state.channel] = identity();
            changed = true;
        }
    });
    ui.tooltip("Reset this channel to a straight line");
    let channel = state.channel.min(CHANNELS.len() - 1);
    let side = side(ui);
    ui.canvas("graph", [side, side], |c| {
        if let Some(pointer) = c.pointer {
            changed |= edit(state, &mut curves.channels[channel], pointer, side);
        }
        c.rect([0.0, 0.0], [side, side], Some(Paint::Surface), Some((Paint::Border, 1.0)), 0.0);
        for k in 1..4 {
            let at = side * k as f32 / 4.0;
            c.line(&[[at, 0.0], [at, side]], Paint::Border, 1.0);
            c.line(&[[0.0, at], [side, at]], Paint::Border, 1.0);
        }
        c.line(&[[0.0, side], [side, 0.0]], Paint::Weak, 1.0);
        draw_curve(c, &curves.channels[channel], side, channel_paint(channel), 2.0);
        for (i, point) in curves.channels[channel].iter().enumerate() {
            let fill = if state.dragging == Some(i) { Paint::Accent } else { Paint::Background };
            c.circle(to_canvas(*point, side), 4.0, Some(fill), Some((Paint::Text, 1.0)));
        }
    });
    ui.weak("Click to add a point, drag to move it, drag it off or Alt-click to remove it.");
    if changed {
        *value = curves.to_text();
    }
}
