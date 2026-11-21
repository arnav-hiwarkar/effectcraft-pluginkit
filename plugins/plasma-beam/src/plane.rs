use crate::beam::Node;
use crate::image::Image;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Bounds {
    pub fn of_nodes<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Option<Bounds> {
        nodes.into_iter().fold(None, |bounds: Option<Bounds>, n| {
            let node = Bounds { min: [n.at[0] - n.radius, n.at[1] - n.radius], max: [n.at[0] + n.radius, n.at[1] + n.radius] };
            Some(bounds.map_or(node, |b| b.union(node)))
        })
    }

    fn union(self, other: Bounds) -> Bounds {
        Bounds {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub image: Image<1>,
    pub origin: [i32; 2],
    pub pitch: f32,
}

pub fn coverage(distance: f32, radius: f32) -> f32 {
    ((distance + 0.5).min(radius) - (distance - 0.5).max(-radius)).clamp(0.0, 1.0)
}

fn pixel_range(lo: f32, hi: f32, len: usize) -> std::ops::Range<usize> {
    let start = lo.floor().clamp(0.0, len as f32) as usize;
    let end = hi.ceil().clamp(0.0, len as f32) as usize;
    start..end.max(start)
}

impl Plane {
    pub fn new(origin: [i32; 2], size: [usize; 2], pitch: f32) -> Plane {
        Plane { image: Image::new(size[0], size[1]), origin, pitch }
    }

    pub fn draw(&mut self, nodes: &[Node]) {
        match nodes {
            [] => {}
            [only] => self.capsule(*only, *only),
            _ => nodes.windows(2).for_each(|pair| self.capsule(pair[0], pair[1])),
        }
    }

    fn local(&self, node: Node) -> Node {
        let at = [
            (node.at[0] - self.origin[0] as f32) / self.pitch,
            (node.at[1] - self.origin[1] as f32) / self.pitch,
        ];
        Node { at, radius: node.radius / self.pitch }
    }

    fn capsule(&mut self, a: Node, b: Node) {
        let (a, b) = (self.local(a), self.local(b));
        let reach = a.radius.max(b.radius) + 1.0;
        let xs = pixel_range(a.at[0].min(b.at[0]) - reach, a.at[0].max(b.at[0]) + reach, self.image.width);
        let ys = pixel_range(a.at[1].min(b.at[1]) - reach, a.at[1].max(b.at[1]) + reach, self.image.height);
        let axis = [b.at[0] - a.at[0], b.at[1] - a.at[1]];
        let length_sq = axis[0] * axis[0] + axis[1] * axis[1];
        let inverse = if length_sq > 1e-12 { 1.0 / length_sq } else { 0.0 };
        let grow = b.radius - a.radius;
        for y in ys {
            let py = y as f32 + 0.5 - a.at[1];
            let row = &mut self.image.row_mut(y)[xs.clone()];
            for (x, cell) in xs.clone().zip(row) {
                let px = x as f32 + 0.5 - a.at[0];
                let t = ((px * axis[0] + py * axis[1]) * inverse).clamp(0.0, 1.0);
                let (ex, ey) = (px - t * axis[0], py - t * axis[1]);
                let value = coverage((ex * ex + ey * ey).sqrt(), a.radius + grow * t);
                cell[0] = cell[0].max(value);
            }
        }
    }

    pub fn row_at(&self, y: usize) -> Option<(usize, &[[f32; 1]])> {
        let local = usize::try_from(y as i64 - i64::from(self.origin[1])).ok()?;
        (local < self.image.height).then(|| (self.origin[0].max(0) as usize, self.image.row(local)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_conserves_the_width_of_thin_lines() {
        assert_eq!(coverage(0.0, 3.0), 1.0);
        assert_eq!(coverage(3.0, 3.0), 0.5);
        assert_eq!(coverage(0.0, 0.2), 0.4);
        assert_eq!(coverage(5.0, 3.0), 0.0);
    }

    #[test]
    fn joined_segments_leave_no_seam_at_the_joint() {
        let mut joined = Plane::new([0, 0], [40, 20], 1.0);
        let node = |x: f32| Node { at: [x, 10.0], radius: 3.0 };
        joined.draw(&[node(5.0), node(20.0), node(35.0)]);
        let mut single = Plane::new([0, 0], [40, 20], 1.0);
        single.draw(&[node(5.0), node(35.0)]);
        assert_eq!(joined, single);
    }

    #[test]
    fn a_degenerate_stroke_draws_a_finite_round_dot() {
        let mut plane = Plane::new([0, 0], [16, 16], 1.0);
        plane.draw(&[Node { at: [8.0, 8.0], radius: 2.0 }]);
        let total: f32 = plane.image.data.iter().map(|p| p[0]).sum();
        assert!(plane.image.data.iter().all(|p| p[0].is_finite()));
        assert!((total - 4.0 * std::f32::consts::PI).abs() < 2.0);
    }
}
