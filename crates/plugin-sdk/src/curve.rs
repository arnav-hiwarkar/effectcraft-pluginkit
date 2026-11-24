pub const MAX_CURVE_POINTS: usize = 64;
pub const CURVE_CHANNELS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ms: Vec<f64>,
}

impl Curve {
    pub fn identity() -> Curve {
        Curve { xs: vec![0.0, 1.0], ys: vec![0.0, 1.0], ms: vec![1.0, 1.0] }
    }

    pub fn new(points: &[[f64; 2]]) -> Curve {
        if check_points(points).is_err() {
            return Curve::identity();
        }
        let xs: Vec<f64> = points.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = points.iter().map(|p| p[1]).collect();
        let n = xs.len();
        let d: Vec<f64> = (0..n - 1).map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k])).collect();
        let mut ms = vec![0.0; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            ms[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) * 0.5 };
        }
        for k in 0..n - 1 {
            if d[k].abs() < 1e-12 {
                ms[k] = 0.0;
                ms[k + 1] = 0.0;
                continue;
            }
            let (a, b) = (ms[k] / d[k], ms[k + 1] / d[k]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                ms[k] = t * a * d[k];
                ms[k + 1] = t * b * d[k];
            }
        }
        Curve { xs, ys, ms }
    }

    pub fn at(&self, x: f64) -> f64 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.ms[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1] + (x - self.xs[n - 1]) * self.ms[n - 1];
        }
        let k = self.xs.partition_point(|&v| v <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[k]
            + (t3 - 2.0 * t2 + t) * h * self.ms[k]
            + (3.0 * t2 - 2.0 * t3) * self.ys[k + 1]
            + (t3 - t2) * h * self.ms[k + 1]
    }

    pub fn lut<const N: usize>(&self) -> [f32; N] {
        let last = N.saturating_sub(1).max(1) as f64;
        std::array::from_fn(|i| self.at(i as f64 / last) as f32)
    }
}

pub fn check_points(points: &[[f64; 2]]) -> Result<(), String> {
    if !(2..=MAX_CURVE_POINTS).contains(&points.len()) {
        return Err(format!("a curve has 2 to {MAX_CURVE_POINTS} points, not {}", points.len()));
    }
    if let Some(p) = points.iter().find(|p| !p.iter().all(|v| (0.0..=1.0).contains(v))) {
        return Err(format!("curve point {p:?} lies outside 0..1"));
    }
    if let Some(w) = points.windows(2).find(|w| w[0][0] >= w[1][0]) {
        return Err(format!("curve points {:?} and {:?} are not in increasing x", w[0], w[1]));
    }
    Ok(())
}

pub fn parse_curves(text: &str, channels: usize) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let curves: Vec<Vec<[f64; 2]>> = text
        .split('|')
        .map(|channel| {
            let mut points: Vec<[f64; 2]> = channel
                .split(|c: char| c.is_whitespace() || c == ';')
                .filter_map(|pair| {
                    let (x, y) = pair.split_once(',')?;
                    let point = [x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?];
                    point.iter().all(|v| v.is_finite()).then(|| point.map(|v| v.clamp(0.0, 1.0)))
                })
                .collect();
            points.sort_by(|a, b| a[0].total_cmp(&b[0]));
            points.dedup_by(|a, b| a[0] == b[0]);
            points
        })
        .collect();
    if curves.len() != channels {
        return Err(format!("{channels} curve(s) separated by `|`, not {}", curves.len()));
    }
    curves.iter().try_for_each(|points| check_points(points))?;
    Ok(curves)
}

pub fn curves_text(curves: &[Vec<[f64; 2]>]) -> String {
    curves.iter().map(|points| points.iter().map(|[x, y]| format!("{x},{y}")).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" | ")
}

pub fn default_curves_text(default: &[[f64; 2]], channels: bool) -> String {
    curves_text(&vec![default.to_vec(); if channels { CURVE_CHANNELS } else { 1 }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_samples_its_monotone_spline_and_the_identity_is_exact() {
        let curve = Curve::new(&[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]]);
        assert_eq!([0.0, 0.25, 0.5, 1.0].map(|x| curve.at(x)), [0.0, 0.40625, 0.75, 1.0]);
        assert_eq!([0.0, 0.25, 1.0].map(|x| Curve::identity().at(x)), [0.0, 0.25, 1.0]);
        assert_eq!(curve.lut::<3>(), [0.0, 0.75, 1.0]);
    }

    #[test]
    fn curve_text_is_cleaned_and_checked() {
        assert_eq!(parse_curves("1,1 0,0.2 0.5,2", 1), Ok(vec![vec![[0.0, 0.2], [0.5, 1.0], [1.0, 1.0]]]));
        assert_eq!(parse_curves("0,0 1,1", 4), Err("4 curve(s) separated by `|`, not 1".into()));
        assert_eq!(parse_curves("0,0", 1), Err("a curve has 2 to 64 points, not 1".into()));
        let many: Vec<String> = (0..=64).map(|i| format!("{},0", f64::from(i) / 64.0)).collect();
        assert_eq!(parse_curves(&many.join(" "), 1), Err("a curve has 2 to 64 points, not 65".into()));
        assert_eq!(curves_text(&[vec![[0.0, 0.0], [1.0, 0.5]], vec![[0.0, 1.0], [1.0, 0.0]]]), "0,0 1,0.5 | 0,1 1,0");
    }
}
