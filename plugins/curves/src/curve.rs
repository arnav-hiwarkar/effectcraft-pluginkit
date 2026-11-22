use serde::{Deserialize, Serialize};

pub const LUT_SIZE: usize = 1024;
pub const CHANNELS: [&str; 5] = ["RGB", "Red", "Green", "Blue", "Alpha"];

pub type Point = [f64; 2];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Curves {
    pub channels: [Vec<Point>; 5],
}

pub fn identity() -> Vec<Point> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

impl Default for Curves {
    fn default() -> Self {
        Curves { channels: std::array::from_fn(|_| identity()) }
    }
}

impl Curves {
    pub fn parse(text: &str) -> Curves {
        let mut curves: Curves = serde_json::from_str(text).unwrap_or_default();
        for points in &mut curves.channels {
            *points = tidy(std::mem::take(points));
        }
        curves
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn default_text() -> String {
        Curves::default().to_text()
    }
}

pub fn tidy(points: Vec<Point>) -> Vec<Point> {
    let mut points: Vec<Point> = points.into_iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
    if points.len() < 2 { identity() } else { points }
}

pub fn is_identity(points: &[Point]) -> bool {
    points == identity().as_slice()
}

pub struct Spline {
    points: Vec<Point>,
    tangents: Vec<f64>,
}

impl Spline {
    pub fn new(points: &[Point]) -> Spline {
        let points = tidy(points.to_vec());
        let n = points.len();
        let secant: Vec<f64> = points.windows(2).map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0])).collect();
        let mut tangents = vec![0.0; n];
        tangents[0] = secant[0];
        tangents[n - 1] = secant[n - 2];
        for k in 1..n - 1 {
            tangents[k] = if secant[k - 1] * secant[k] <= 0.0 { 0.0 } else { (secant[k - 1] + secant[k]) / 2.0 };
        }
        for (k, d) in secant.iter().enumerate() {
            if *d == 0.0 {
                tangents[k] = 0.0;
                tangents[k + 1] = 0.0;
                continue;
            }
            let (a, b) = (tangents[k] / d, tangents[k + 1] / d);
            let length = a * a + b * b;
            if length > 9.0 {
                let t = 3.0 / length.sqrt();
                tangents[k] = t * a * d;
                tangents[k + 1] = t * b * d;
            }
        }
        Spline { points, tangents }
    }

    pub fn at(&self, x: f64) -> f64 {
        let (first, last) = (self.points[0], self.points[self.points.len() - 1]);
        if x <= first[0] {
            return first[1];
        }
        if x >= last[0] {
            return last[1];
        }
        let k = self.points.partition_point(|p| p[0] <= x).saturating_sub(1).min(self.points.len() - 2);
        let ([x0, y0], [x1, y1]) = (self.points[k], self.points[k + 1]);
        let h = x1 - x0;
        let t = (x - x0) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * self.tangents[k] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * self.tangents[k + 1]
    }
}

pub struct Lut {
    table: Option<Box<[f32; LUT_SIZE]>>,
}

impl Lut {
    pub fn new(points: &[Point]) -> Lut {
        if is_identity(&tidy(points.to_vec())) {
            return Lut { table: None };
        }
        let spline = Spline::new(points);
        let mut table = Box::new([0.0f32; LUT_SIZE]);
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = spline.at(i as f64 / (LUT_SIZE - 1) as f64) as f32;
        }
        Lut { table: Some(table) }
    }

    pub fn apply(&self, v: f32) -> f32 {
        let Some(table) = &self.table else { return v };
        let last = (LUT_SIZE - 1) as f32;
        if v <= 0.0 {
            return table[0] + v * (table[1] - table[0]) * last;
        }
        if v >= 1.0 {
            return table[LUT_SIZE - 1] + (v - 1.0) * (table[LUT_SIZE - 1] - table[LUT_SIZE - 2]) * last;
        }
        let at = v * last;
        let i = (at as usize).min(LUT_SIZE - 2);
        let f = at - i as f32;
        table[i] + (table[i + 1] - table[i]) * f
    }
}

pub struct Luts {
    master: Lut,
    rgb: [Lut; 3],
    alpha: Lut,
}

impl Luts {
    pub fn new(curves: &Curves) -> Luts {
        let [master, r, g, b, a] = &curves.channels;
        Luts { master: Lut::new(master), rgb: [Lut::new(r), Lut::new(g), Lut::new(b)], alpha: Lut::new(a) }
    }

    pub fn pixel(&self, [r, g, b, a]: [f32; 4]) -> [f32; 4] {
        let alpha = self.alpha.apply(a);
        if a <= 0.0 {
            return [0.0, 0.0, 0.0, alpha.max(0.0)];
        }
        let straight = [r / a, g / a, b / a];
        let out: [f32; 3] = std::array::from_fn(|c| self.rgb[c].apply(self.master.apply(straight[c])) * alpha);
        [out[0], out[1], out[2], alpha]
    }
}
