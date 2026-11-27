use std::f32::consts::{PI, TAU};

use crate::math::{Vec3, cross, length, normalize, sub};

pub const FLOATS_PER_VERTEX: usize = 8;
pub const LAYOUT: [u32; 3] = [3, 3, 2];

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.vertices.len() / FLOATS_PER_VERTEX
    }

    pub fn position(&self, vertex: usize) -> Vec3 {
        std::array::from_fn(|i| self.vertices[vertex * FLOATS_PER_VERTEX + i])
    }

    pub fn normal(&self, vertex: usize) -> Vec3 {
        std::array::from_fn(|i| self.vertices[vertex * FLOATS_PER_VERTEX + 3 + i])
    }

    pub fn push(&mut self, position: Vec3, normal: Vec3, uv: [f32; 2]) -> u32 {
        self.vertices.extend(position.iter().chain(&normal).chain(&uv));
        self.vertex_count() as u32 - 1
    }

    pub fn flat(&mut self, corners: &[Vec3]) {
        if corners.len() < 3 {
            return;
        }
        let normal = normalize(cross(sub(corners[1], corners[0]), sub(corners[2], corners[0])));
        let first = self.vertex_count() as u32;
        for (i, corner) in corners.iter().enumerate() {
            self.push(*corner, normal, [i as f32 / corners.len() as f32, 0.5]);
        }
        for i in 1..corners.len() as u32 - 1 {
            self.indices.extend([first, first + i, first + i + 1]);
        }
    }

    pub fn fit_unit_sphere(&mut self) {
        let count = self.vertex_count();
        if count == 0 {
            return;
        }
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for vertex in 0..count {
            let p = self.position(vertex);
            low = std::array::from_fn(|i| low[i].min(p[i]));
            high = std::array::from_fn(|i| high[i].max(p[i]));
        }
        let centre: Vec3 = std::array::from_fn(|i| (low[i] + high[i]) / 2.0);
        let radius = (0..count).map(|vertex| length(sub(self.position(vertex), centre))).fold(1e-9, f32::max);
        for vertex in self.vertices.chunks_exact_mut(FLOATS_PER_VERTEX) {
            for i in 0..3 {
                vertex[i] = (vertex[i] - centre[i]) / radius;
            }
        }
    }

    pub fn vertex_bytes(&self) -> Vec<u8> {
        self.vertices.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    pub fn index_bytes(&self) -> Vec<u8> {
        self.indices.iter().flat_map(|value| value.to_le_bytes()).collect()
    }
}

fn grid(mesh: &mut Mesh, columns: u32, rows: u32, surface: impl Fn(f32, f32) -> (Vec3, Vec3)) {
    let first = mesh.vertex_count() as u32;
    for row in 0..=rows {
        for column in 0..=columns {
            let (u, v) = (column as f32 / columns as f32, row as f32 / rows as f32);
            let (position, normal) = surface(u, v);
            mesh.push(position, normal, [u, v]);
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            let a = first + row * (columns + 1) + column;
            let b = a + columns + 1;
            mesh.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
}

pub fn cube() -> Mesh {
    let mut mesh = Mesh::default();
    let side = 1.0 / 3.0f32.sqrt();
    for axis in 0..3 {
        for sign in [-side, side] {
            let corner = |a: f32, b: f32| {
                let mut p = [0.0; 3];
                p[axis] = sign;
                p[(axis + 1) % 3] = a * side;
                p[(axis + 2) % 3] = b * side * sign.signum();
                p
            };
            mesh.flat(&[corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]);
        }
    }
    mesh
}

pub fn sphere() -> Mesh {
    let mut mesh = Mesh::default();
    grid(&mut mesh, 64, 32, |u, v| {
        let (theta, phi) = (u * TAU, v * PI);
        let normal = [phi.sin() * theta.cos(), -phi.cos(), phi.sin() * theta.sin()];
        (normal, normal)
    });
    mesh
}

pub fn torus() -> Mesh {
    const RING: f32 = 0.7;
    const TUBE: f32 = 0.3;
    let mut mesh = Mesh::default();
    grid(&mut mesh, 64, 24, |u, v| {
        let (theta, phi) = (u * TAU, v * TAU);
        let normal = [phi.cos() * theta.cos(), phi.cos() * theta.sin(), phi.sin()];
        ([(RING + TUBE * phi.cos()) * theta.cos(), (RING + TUBE * phi.cos()) * theta.sin(), TUBE * phi.sin()], normal)
    });
    mesh
}

pub fn star() -> Mesh {
    const POINTS: usize = 5;
    const INNER: f32 = 0.42;
    const HALF_DEPTH: f32 = 0.2;
    let outline: Vec<[f32; 2]> = (0..POINTS * 2)
        .map(|i| {
            let angle = i as f32 / (POINTS * 2) as f32 * TAU - PI / 2.0;
            let radius = if i % 2 == 0 { 0.97 } else { INNER };
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect();
    let mut mesh = Mesh::default();
    for (i, a) in outline.iter().enumerate() {
        let b = outline[(i + 1) % outline.len()];
        mesh.flat(&[[0.0, 0.0, -HALF_DEPTH], [b[0], b[1], -HALF_DEPTH], [a[0], a[1], -HALF_DEPTH]]);
        mesh.flat(&[[0.0, 0.0, HALF_DEPTH], [a[0], a[1], HALF_DEPTH], [b[0], b[1], HALF_DEPTH]]);
        mesh.flat(&[[a[0], a[1], -HALF_DEPTH], [b[0], b[1], -HALF_DEPTH], [b[0], b[1], HALF_DEPTH], [a[0], a[1], HALF_DEPTH]]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::normalize;

    fn dot(a: Vec3, b: Vec3) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn check(name: &str, mesh: &Mesh) {
        assert_eq!(mesh.vertices.len() % FLOATS_PER_VERTEX, 0, "{name}");
        assert_eq!(mesh.indices.len() % 3, 0, "{name}");
        assert!(mesh.indices.iter().all(|index| (*index as usize) < mesh.vertex_count()), "{name}: indices in range");
        for vertex in 0..mesh.vertex_count() {
            assert!(length(mesh.position(vertex)) <= 1.0001, "{name}: inside the unit sphere");
            assert!((length(mesh.normal(vertex)) - 1.0).abs() < 1e-4, "{name}: unit normals");
        }
    }

    #[test]
    fn primitives_are_indexed_unit_meshes() {
        check("cube", &cube());
        check("sphere", &sphere());
        check("torus", &torus());
        check("star", &star());
        assert_eq!((cube().vertex_count(), cube().indices.len()), (24, 36));
        assert_eq!((star().vertex_count(), star().indices.len()), (100, 120));
    }

    #[test]
    fn cube_and_sphere_normals_point_away_from_the_centre() {
        for mesh in [cube(), sphere()] {
            for vertex in 0..mesh.vertex_count() {
                assert!(dot(mesh.normal(vertex), normalize(mesh.position(vertex))) > 0.5);
            }
        }
    }

    #[test]
    fn fitting_centres_a_mesh_and_gives_it_radius_one() {
        let mut mesh = Mesh::default();
        mesh.flat(&[[10.0, 0.0, 0.0], [14.0, 0.0, 0.0], [14.0, 4.0, 0.0], [10.0, 4.0, 0.0]]);
        mesh.fit_unit_sphere();
        let half = 0.5f32.sqrt();
        assert!(length(sub(mesh.position(0), [-half, -half, 0.0])) < 1e-5);
        assert!(length(sub(mesh.position(2), [half, half, 0.0])) < 1e-5);
    }
}
