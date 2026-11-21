pub type Point = [f32; 2];

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    points: Vec<Point>,
    lengths: Vec<f32>,
    closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: Point,
    pub normal: Point,
    pub distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub samples: Vec<Sample>,
    pub from: f32,
    pub to: f32,
    pub loops: bool,
}

impl Span {
    pub fn length(&self) -> f32 {
        self.to - self.from
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn lerp(a: Point, b: Point, t: f32) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

impl Path {
    pub fn new(points: Vec<Point>, closed: bool) -> Path {
        let mut points: Vec<Point> = points.into_iter().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
        let closed = closed && points.len() > 2;
        if closed {
            points.push(points[0]);
        }
        let lengths = points
            .iter()
            .scan((0.0, None), |(total, previous): &mut (f32, Option<Point>), &p| {
                *total += previous.map_or(0.0, |q| distance(q, p));
                *previous = Some(p);
                Some(*total)
            })
            .collect();
        Path { points, lengths, closed }
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn point_at(&self, s: f32) -> Point {
        let i = self.lengths.partition_point(|&l| l <= s).clamp(1, self.points.len().max(2) - 1);
        let (l0, l1) = (self.lengths[i - 1], self.lengths[i]);
        let t = if l1 > l0 { ((s - l0) / (l1 - l0)).clamp(0.0, 1.0) } else { 0.0 };
        lerp(self.points[i - 1], self.points[i], t)
    }

    pub fn span(&self, range: [f32; 2], step: f32) -> Span {
        let total = self.length();
        let (from, to) = (range[0].min(range[1]).clamp(0.0, 1.0) * total, range[0].max(range[1]).clamp(0.0, 1.0) * total);
        let loops = self.closed && from <= 0.0 && to >= total;
        let empty = Span { samples: Vec::new(), from, to, loops: false };
        if self.points.is_empty() || range[0] == range[1] {
            return empty;
        }
        if total <= 0.0 || self.points.len() < 2 {
            let at = self.points[0];
            return Span { samples: vec![Sample { at, normal: [0.0, 0.0], distance: 0.0 }], ..empty };
        }
        let mut stations = vec![(self.point_at(from), from)];
        stations.extend(
            self.points.iter().zip(&self.lengths).filter(|&(_, &l)| l > from && l < to).map(|(&p, &l)| (p, l)),
        );
        stations.push((self.point_at(to), to));
        let mut points: Vec<(Point, f32)> = Vec::with_capacity(stations.len() + ((to - from) / step) as usize + 1);
        for pair in stations.windows(2) {
            let ((a, la), (b, lb)) = (pair[0], pair[1]);
            let pieces = ((lb - la) / step).ceil().max(1.0) as usize;
            points.extend((0..pieces).map(|k| {
                let t = k as f32 / pieces as f32;
                (lerp(a, b, t), la + (lb - la) * t)
            }));
        }
        points.push(stations[stations.len() - 1]);
        let samples = with_normals(&points, loops);
        Span { samples, from, to, loops }
    }
}

fn with_normals(points: &[(Point, f32)], loops: bool) -> Vec<Sample> {
    let n = points.len();
    let neighbour = |i: usize, back: bool| -> Point {
        match (back, i) {
            (true, 0) if loops && n > 2 => points[n - 2].0,
            (true, 0) => points[0].0,
            (true, i) => points[i - 1].0,
            (false, i) if i + 1 < n => points[i + 1].0,
            (false, _) if loops && n > 2 => points[1].0,
            (false, i) => points[i].0,
        }
    };
    (0..n)
        .map(|i| {
            let (prev, next) = (neighbour(i, true), neighbour(i, false));
            let tangent = [next[0] - prev[0], next[1] - prev[1]];
            let len = tangent[0].hypot(tangent[1]);
            let normal = if len > 1e-6 { [-tangent[1] / len, tangent[0] / len] } else { [0.0, 0.0] };
            Sample { at: points[i].0, normal, distance: points[i].1 }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ell() -> Path {
        Path::new(vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0]], false)
    }

    #[test]
    fn trimming_keeps_the_requested_share_of_arc_length() {
        let span = ell().span([0.25, 0.875], 10.0);
        assert_eq!((span.from, span.to), (37.5, 131.25));
        assert_eq!(span.samples.first().map(|s| s.at), Some([37.5, 0.0]));
        assert_eq!(span.samples.last().map(|s| s.at), Some([100.0, 31.25]));
        let walked: f32 = span.samples.windows(2).map(|w| distance(w[0].at, w[1].at)).sum();
        assert!((walked - 93.75).abs() < 1e-3);
    }

    #[test]
    fn subdivision_keeps_corners_and_bounds_the_step() {
        let span = ell().span([0.0, 1.0], 7.0);
        assert!(span.samples.iter().any(|s| s.at == [100.0, 0.0]));
        assert!(span.samples.windows(2).all(|w| distance(w[0].at, w[1].at) <= 7.0 + 1e-4));
    }

    #[test]
    fn reversed_offsets_trim_the_same_span() {
        assert_eq!(ell().span([0.9, 0.2], 10.0), ell().span([0.2, 0.9], 10.0));
    }

    #[test]
    fn equal_offsets_draw_nothing() {
        assert!(ell().span([0.4, 0.4], 5.0).samples.is_empty());
    }

    #[test]
    fn closed_path_includes_the_closing_edge_and_loops() {
        let square = Path::new(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true);
        assert_eq!(square.length(), 40.0);
        let span = square.span([0.0, 1.0], 100.0);
        assert!(span.loops);
        assert_eq!(span.samples[0].normal, span.samples[span.samples.len() - 1].normal);
    }

    #[test]
    fn degenerate_paths_yield_a_single_still_sample() {
        let dot = Path::new(vec![[5.0, 5.0], [5.0, 5.0]], false).span([0.0, 1.0], 2.0);
        assert_eq!(dot.samples, vec![Sample { at: [5.0, 5.0], normal: [0.0, 0.0], distance: 0.0 }]);
        assert_eq!(Path::new(vec![[1.0, 2.0]], true).span([0.0, 1.0], 2.0).samples.len(), 1);
        assert!(Path::new(Vec::new(), false).span([0.0, 1.0], 2.0).samples.is_empty());
    }
}
