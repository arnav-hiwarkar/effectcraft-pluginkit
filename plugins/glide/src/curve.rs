use serde::{Deserialize, Serialize};

pub const Y_MIN: f64 = -0.6;
pub const Y_MAX: f64 = 1.6;
pub const MIN_INFLUENCE: f64 = 0.001;
const FLAT_SLOPE: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    pub speed: f64,
    pub influence: f64,
}

impl Default for Curve {
    fn default() -> Self {
        Curve::DEFAULT
    }
}

impl Curve {
    pub const DEFAULT: Curve = Curve::new(0.42, 0.0, 0.58, 1.0);

    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Curve {
        Curve { x1, y1, x2, y2 }
    }

    pub fn clamped(self) -> Curve {
        let x = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let y = |v: f64| if v.is_finite() { v.clamp(Y_MIN, Y_MAX) } else { 0.0 };
        Curve { x1: x(self.x1), y1: y(self.y1), x2: x(self.x2), y2: y(self.y2) }
    }

    pub fn handle(self, handle: Handle) -> [f64; 2] {
        match handle {
            Handle::Start => [self.x1, self.y1],
            Handle::End => [self.x2, self.y2],
        }
    }

    pub fn anchor(handle: Handle) -> [f64; 2] {
        match handle {
            Handle::Start => [0.0, 0.0],
            Handle::End => [1.0, 1.0],
        }
    }

    pub fn with_handle(self, handle: Handle, [x, y]: [f64; 2]) -> Curve {
        match handle {
            Handle::Start => Curve { x1: x, y1: y, ..self },
            Handle::End => Curve { x2: x, y2: y, ..self },
        }
        .clamped()
    }

    pub fn nearest_handle(self, point: [f64; 2], scale: [f64; 2]) -> Handle {
        let distance = |h: [f64; 2]| ((h[0] - point[0]) * scale[0]).hypot((h[1] - point[1]) * scale[1]);
        if distance(self.handle(Handle::End)) < distance(self.handle(Handle::Start)) { Handle::End } else { Handle::Start }
    }

    pub fn outgoing(self, slope: f64) -> Ease {
        let influence = self.x1.max(MIN_INFLUENCE);
        Ease { speed: self.y1 / influence * slope, influence }
    }

    pub fn incoming(self, slope: f64) -> Ease {
        let influence = (1.0 - self.x2).max(MIN_INFLUENCE);
        Ease { speed: (1.0 - self.y2) / influence * slope, influence }
    }

    pub fn from_eases(outgoing: Ease, incoming: Ease, slope: f64) -> Option<Curve> {
        if slope.abs() < FLAT_SLOPE || !slope.is_finite() {
            return None;
        }
        Some(
            Curve {
                x1: outgoing.influence,
                y1: outgoing.speed * outgoing.influence / slope,
                x2: 1.0 - incoming.influence,
                y2: 1.0 - incoming.speed * incoming.influence / slope,
            }
            .clamped(),
        )
    }

    pub fn value_at(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        bezier(self.y1, self.y2, solve(self.x1, self.x2, t))
    }

    pub fn css(self) -> String {
        format!("cubic-bezier({}, {}, {}, {})", short(self.x1), short(self.y1), short(self.x2), short(self.y2))
    }
}

fn short(v: f64) -> String {
    let rounded = format!("{v:.3}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" { "0".into() } else { trimmed.into() }
}

fn bezier(p1: f64, p2: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u
}

fn bezier_slope(p1: f64, p2: f64, u: f64) -> f64 {
    let v = 1.0 - u;
    3.0 * v * v * p1 + 6.0 * v * u * (p2 - p1) + 3.0 * u * u * (1.0 - p2)
}

fn solve(x1: f64, x2: f64, x: f64) -> f64 {
    let mut u = x;
    for _ in 0..8 {
        let error = bezier(x1, x2, u) - x;
        if error.abs() < 1e-12 {
            return u;
        }
        let slope = bezier_slope(x1, x2, u);
        if slope.abs() < 1e-9 {
            break;
        }
        u = (u - error / slope).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        if bezier(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_curve_becomes_speed_and_influence_scaled_by_the_segment_slope() {
        let curve = Curve::new(0.42, 0.1, 0.58, 1.0);
        let out = curve.outgoing(50.0);
        let inn = curve.incoming(50.0);
        assert!(close(out.influence, 0.42) && close(out.speed, 0.1 / 0.42 * 50.0));
        assert!(close(inn.influence, 0.42) && close(inn.speed, 0.0));
    }

    #[test]
    fn eases_read_back_into_the_curve_they_came_from() {
        let curve = Curve::new(0.34, 1.56, 0.64, 1.0);
        let slope = -37.5;
        let back = Curve::from_eases(curve.outgoing(slope), curve.incoming(slope), slope);
        assert!(back.is_some_and(|b| close(b.x1, 0.34) && close(b.y1, 1.56) && close(b.x2, 0.64) && close(b.y2, 1.0)));
    }

    #[test]
    fn a_handle_on_its_anchor_column_keeps_a_finite_minimum_influence() {
        let curve = Curve::new(0.0, 0.5, 1.0, 0.2);
        let out = curve.outgoing(10.0);
        let inn = curve.incoming(10.0);
        assert_eq!((out.influence, out.speed), (0.001, 5000.0));
        assert_eq!(inn.influence, 0.001);
        assert!(close(inn.speed, 8000.0));
    }

    #[test]
    fn a_flat_segment_has_no_curve_to_read() {
        let flat = Ease { speed: 0.0, influence: 0.3 };
        assert_eq!(Curve::from_eases(flat, flat, 0.0), None);
    }

    #[test]
    fn handles_stay_inside_the_editable_range() {
        let curve = Curve::default().with_handle(Handle::Start, [-0.4, 2.5]).with_handle(Handle::End, [1.7, -3.0]);
        assert_eq!(curve, Curve::new(0.0, 1.6, 1.0, -0.6));
    }

    #[test]
    fn the_nearer_handle_is_found_in_screen_proportions() {
        let curve = Curve::new(0.2, 0.0, 0.8, 1.0);
        assert_eq!(curve.nearest_handle([0.3, 0.1], [1.0, 1.0]), Handle::Start);
        assert_eq!(curve.nearest_handle([0.7, 0.8], [1.0, 1.0]), Handle::End);
    }

    #[test]
    fn the_css_line_trims_trailing_zeros() {
        assert_eq!(Curve::new(0.42, 0.0, 0.58, 1.0).css(), "cubic-bezier(0.42, 0, 0.58, 1)");
        assert_eq!(Curve::new(1.0 / 3.0, -0.0, 0.6667, 1.0).css(), "cubic-bezier(0.333, 0, 0.667, 1)");
    }
}
