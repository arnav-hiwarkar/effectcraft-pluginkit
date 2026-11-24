use craft_plugin::prelude::*;

use crate::math::{Mat4, Vec3, mul, normalize, rotate, scale, translate};

pub const MAX_LIGHTS: usize = 6;
pub const FLOATS_PER_LIGHT: usize = 12;
pub const FLOATS_PER_INSTANCE: usize = 20;
pub const INSTANCE_LAYOUT: [u32; 5] = [4, 4, 4, 4, 4];
const KEY_LIGHT_DIRECTION: Vec3 = [0.4, 0.5, 0.75];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Replicator {
    Off,
    Grid { columns: u32, rows: u32, spacing: [f32; 3] },
    Ring { count: u32, radius: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Vec3,
    pub rotation: Vec3,
    pub size: f32,
}

impl Placement {
    fn matrix(&self) -> Mat4 {
        let turn = mul(&rotate(2, self.rotation[2]), &mul(&rotate(1, self.rotation[1]), &rotate(0, self.rotation[0])));
        mul(&translate(self.position), &mul(&turn, &scale(self.size)))
    }
}

pub fn instances(placement: &Placement, replicator: Replicator, first: [f32; 4], last: [f32; 4]) -> Vec<f32> {
    let model = placement.matrix();
    let offsets: Vec<Mat4> = match replicator {
        Replicator::Off => vec![translate([0.0; 3])],
        Replicator::Grid { columns, rows, spacing } => {
            let centred = |i: u32, n: u32| i as f32 - (n as f32 - 1.0) / 2.0;
            (0..rows)
                .flat_map(|row| (0..columns).map(move |column| (centred(column, columns), centred(row, rows))))
                .map(|(x, y)| translate([x * spacing[0], y * spacing[1], x * spacing[2]]))
                .collect()
        }
        Replicator::Ring { count, radius } => (0..count)
            .map(|i| {
                let degrees = i as f32 / count as f32 * 360.0;
                mul(&rotate(1, degrees), &translate([0.0, 0.0, -radius]))
            })
            .collect(),
    };
    let about = translate(placement.position);
    let back = translate(placement.position.map(|c| -c));
    let count = offsets.len();
    let mut out = Vec::with_capacity(count * FLOATS_PER_INSTANCE);
    for (i, offset) in offsets.iter().enumerate() {
        let t = if count > 1 { i as f32 / (count - 1) as f32 } else { 0.0 };
        out.extend(mul(&about, &mul(offset, &mul(&back, &model))));
        out.extend((0..4).map(|c| first[c] + (last[c] - first[c]) * t));
    }
    out
}

pub fn lights(scene: &Scene, use_comp_lights: bool) -> Vec<f32> {
    let comp: &[Light] = if use_comp_lights { &scene.lights } else { &[] };
    let key = Light { kind: LightKind::Parallel, direction: KEY_LIGHT_DIRECTION.map(f64::from), colour: [1.0; 3], intensity: 1.0, ..Default::default() };
    let lit: Vec<Light> = if comp.is_empty() { vec![key] } else { comp.iter().take(MAX_LIGHTS).cloned().collect() };
    let mut out = Vec::with_capacity(lit.len() * FLOATS_PER_LIGHT);
    for light in lit {
        let kind = match light.kind {
            LightKind::Ambient => 0.0,
            LightKind::Point => 1.0,
            LightKind::Spot => 2.0,
            LightKind::Parallel => 3.0,
        };
        let outer = (light.cone_angle.clamp(0.0, 180.0) / 2.0).to_radians();
        let inner = outer * (1.0 - light.feather.clamp(0.0, 1.0));
        let direction = normalize(light.direction.map(|c| c as f32));
        out.extend(light.position.map(|c| c as f32));
        out.push(kind);
        out.extend(direction);
        out.push(outer.cos());
        out.extend(light.colour.map(|c| c * light.intensity));
        out.push(inner.cos().max(outer.cos() + 1e-4));
    }
    out
}

pub fn shininess(roughness: f32) -> f32 {
    let alpha = roughness.clamp(0.02, 1.0).powi(2);
    (2.0 / (alpha * alpha) - 2.0).clamp(1.0, 4096.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::point;

    const PLACEMENT: Placement = Placement { position: [100.0, 50.0, 0.0], rotation: [0.0; 3], size: 10.0 };
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn origin_of(instance: &[f32]) -> Vec3 {
        let matrix: Mat4 = std::array::from_fn(|i| instance[i]);
        point(&matrix, [0.0; 3])
    }

    #[test]
    fn a_grid_is_centred_on_the_position_and_ramps_the_colour() {
        let grid = Replicator::Grid { columns: 3, rows: 1, spacing: [40.0, 0.0, 20.0] };
        let data = instances(&PLACEMENT, grid, RED, BLUE);
        let all: Vec<&[f32]> = data.chunks_exact(FLOATS_PER_INSTANCE).collect();
        assert_eq!(all.len(), 3);
        assert_eq!(origin_of(all[0]), [60.0, 50.0, -20.0]);
        assert_eq!(origin_of(all[1]), [100.0, 50.0, 0.0]);
        assert_eq!(origin_of(all[2]), [140.0, 50.0, 20.0]);
        assert_eq!((&all[0][16..], &all[1][16..], &all[2][16..]), (&RED[..], &[0.5, 0.0, 0.5, 1.0][..], &BLUE[..]));
    }

    #[test]
    fn a_ring_stands_its_instances_around_the_vertical_axis() {
        let data = instances(&PLACEMENT, Replicator::Ring { count: 4, radius: 30.0 }, RED, RED);
        let origins: Vec<Vec3> = data.chunks_exact(FLOATS_PER_INSTANCE).map(origin_of).collect();
        let expected = [[100.0, 50.0, -30.0], [70.0, 50.0, 0.0], [100.0, 50.0, 30.0], [130.0, 50.0, 0.0]];
        for (origin, expected) in origins.iter().zip(expected) {
            assert!(crate::math::length(crate::math::sub(*origin, expected)) < 1e-3, "{origins:?}");
        }
    }

    #[test]
    fn without_comp_lights_one_key_light_shines() {
        let scene = Scene::default();
        let data = lights(&scene, true);
        assert_eq!(data.len(), FLOATS_PER_LIGHT);
        assert_eq!(data[3], 3.0);
        assert_eq!(&data[8..11], &[1.0, 1.0, 1.0]);
    }

    #[test]
    fn comp_lights_replace_the_key_light_and_carry_their_intensity() {
        let spot = Light {
            kind: LightKind::Spot,
            position: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 2.0],
            colour: [1.0, 0.5, 0.25],
            intensity: 2.0,
            cone_angle: 90.0,
            feather: 0.5,
            ..Default::default()
        };
        let scene = Scene { lights: vec![spot], ..Default::default() };
        let data = lights(&scene, true);
        assert_eq!(&data[..7], &[1.0, 2.0, 3.0, 2.0, 0.0, 0.0, 1.0]);
        assert_eq!(&data[8..11], &[2.0, 1.0, 0.5]);
        assert!((data[7] - 45f32.to_radians().cos()).abs() < 1e-6 && (data[11] - 22.5f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!(lights(&scene, false)[3], 3.0);
    }

    #[test]
    fn rougher_surfaces_have_wider_highlights() {
        assert!(shininess(0.2) > shininess(0.5) && shininess(0.5) > shininess(0.9));
    }
}
