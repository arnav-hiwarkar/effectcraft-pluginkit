use craft_plugin::math::smoothstep;
use craft_plugin::prelude::*;

use crate::flare::*;
use crate::looks::{self, Element};
use crate::spec::{Behaviour, Composite, Dirt, Ghosts, Glow, Hoop, Overall, Own, Rays, Ring, Streak};

const OCCLUSION_TAPS: usize = 8;

pub fn edge_factor(light: [f32; 2], min: [f32; 2], max: [f32; 2], width: f32) -> f32 {
    let inside = (light[0] - min[0]).min(max[0] - light[0]).min(light[1] - min[1]).min(max[1] - light[1]);
    if width <= 0.0 {
        return if inside >= 0.0 { 1.0 } else { 0.0 };
    }
    smoothstep(-0.5 * width, width, inside)
}

pub fn visibility(alpha: impl Fn(f32, f32) -> f32, light: [f32; 2], radius: f32) -> f32 {
    let mut sum = alpha(light[0], light[1]);
    let mut taps = 1.0;
    for ring in [0.5, 1.0] {
        for k in 0..OCCLUSION_TAPS {
            let a = k as f32 * std::f32::consts::TAU / OCCLUSION_TAPS as f32 + ring;
            sum += alpha(light[0] + ring * radius * a.cos(), light[1] + ring * radius * a.sin());
            taps += 1.0;
        }
    }
    1.0 - (sum / taps).clamp(0.0, 1.0)
}

fn ratio(value: f64) -> f32 {
    (value / 100.0).max(0.0) as f32
}

fn count(value: f64, look: u32) -> u32 {
    match value.round() as u32 {
        0 => look,
        n => n,
    }
}

fn element(own: Own, base: Element, gain: f32, tint: [f32; 3], size: f32) -> ([f32; 3], f32) {
    if !own.enable {
        return ([0.0; 3], 1.0);
    }
    let [r, g, b, _] = own.colour;
    let amount = base.brightness * ratio(own.brightness) * gain;
    let colour = std::array::from_fn(|c| base.colour[c] * [r, g, b][c] * tint[c] * amount);
    (colour, (base.size * ratio(own.size) * size).max(0.5))
}

fn put(f: &mut [f32; FLOATS], at: usize, values: &[f32]) {
    f[at..at + values.len()].copy_from_slice(values);
}

pub fn pack(cx: &Cx, occlusion: f32) -> Pack {
    let overall = Overall::read(cx);
    let behaviour = Behaviour::read(cx);
    let look = looks::look(overall.look);
    let scale = cx.scale as f32;
    let size = scale * ratio(overall.scale);
    let light = overall.light.map(|v| v as f32);
    let origin = cx.offset.map(|v| v as f32);
    let extent = [0, 1].map(|i| (cx.layer_size[i] * cx.scale) as f32);
    let centre = [origin[0] + 0.5 * extent[0], origin[1] + 0.5 * extent[1]];
    let edge =
        if behaviour.edge { edge_factor(light, origin, [origin[0] + extent[0], origin[1] + extent[1]], (behaviour.edge_width * cx.scale) as f32) } else { 1.0 };
    let shimmer = ratio(behaviour.shimmer);
    let phase = (cx.time * behaviour.shimmer_speed) as f32;
    let flicker = 1.0 + 0.06 * shimmer * ((7.3 * phase).sin() * 0.6 + (13.1 * phase).sin() * 0.4);
    let gain = ratio(overall.brightness) * edge * occlusion * flicker;
    let [tint_r, tint_g, tint_b, _] = overall.color;
    let tint = [tint_r, tint_g, tint_b];
    let rotation = (overall.rotation as f32).to_radians();

    let mut f = [0.0; FLOATS];
    let mut u = [0; UINTS];
    put(&mut f, LIGHT, &light);
    put(&mut f, CENTRE, &centre);
    put(&mut f, ROTATION, &[rotation.cos(), rotation.sin()]);
    put(&mut f, PHASE, &[phase, shimmer]);
    put(&mut f, ORIGIN, &origin);

    let glow = Glow::read(cx);
    let (colour, radius) = element(glow.own(), look.glow, gain, tint, size);
    put(&mut f, GLOW, &colour);
    put(&mut f, GLOW_RADIUS, &[radius, (look.glow.shape * ratio(glow.falloff)).max(0.05)]);

    let rays = Rays::read(cx);
    let (colour, length) = element(rays.own(), look.rays, gain, tint, size);
    put(&mut f, RAYS, &colour);
    let sharpness = look.rays.shape * ratio(rays.sharpness);
    put(&mut f, RAYS_LENGTH, &[length, sharpness]);
    put(&mut f, RAYS_CUTOFF, &[rays_cutoff(sharpness)]);
    u[RAY_COUNT] = count(rays.count, look.ray_count);

    let streak = Streak::read(cx);
    let (colour, length) = element(streak.own(), look.streak, gain, tint, size);
    put(&mut f, STREAK, &colour);
    put(&mut f, STREAK_LENGTH, &[length, (look.streak.shape * ratio(streak.thickness) * size).max(0.25)]);

    let ring = Ring::read(cx);
    let (colour, radius) = element(ring.own(), look.ring, gain, tint, size);
    put(&mut f, RING, &colour);
    let width = (look.ring.shape * ratio(ring.width) * size).max(0.25);
    put(&mut f, RING_RADIUS, &[radius, width, look.fringe * ratio(ring.fringe)]);

    let ghosts = Ghosts::read(cx);
    let (colour, ghost) = element(ghosts.own(), look.ghosts, gain, tint, size);
    put(&mut f, GHOSTS, &colour);
    let iris = (ghosts.iris_rotation as f32).to_radians() + rotation;
    put(&mut f, GHOST_SIZE, &[ghost, iris, look.ghosts.shape * ratio(ghosts.spread)]);
    u[GHOST_COUNT] = count(ghosts.count, look.ghost_count).min(MAX_GHOSTS);
    u[BLADES] = count(ghosts.blades, look.blades);
    u[SEED] = ghosts.seed.max(0.0) as u32;

    let (colour, radius) = element(Dirt::read(cx).own(), look.dirt, gain, tint, size);
    put(&mut f, DIRT, &colour);
    put(&mut f, DIRT_RADIUS, &[radius, (48.0 * scale).max(1.0)]);

    let hoop = Hoop::read(cx);
    let (colour, radius) = element(hoop.own(), look.hoop, gain, tint, size);
    put(&mut f, HOOP, &colour);
    put(&mut f, HOOP_RADIUS, &[radius, (look.hoop.shape * ratio(hoop.width) * size).max(0.25)]);

    let composite = Composite::read(cx);
    u[BLEND] = composite.blend.min(1) as u32;
    u[FLARE_ONLY] = u32::from(composite.flare_only);
    Pack { f, u }
}

pub fn occlusion(cx: &Cx) -> f32 {
    let behaviour = Behaviour::read(cx);
    if !behaviour.occlusion {
        return 1.0;
    }
    let Some(layer) = behaviour.occluder.image(cx) else { return 1.0 };
    let light = Overall::read(cx).light.map(|v| v as f32);
    let radius = (behaviour.occlusion_radius * cx.scale) as f32;
    visibility(|x, y| layer.sample(f64::from(x) - 0.5, f64::from(y) - 0.5)[3], light, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_trigger_dims_the_flare_near_and_past_the_edge() {
        let (min, max) = ([0.0, 0.0], [1000.0, 500.0]);
        assert_eq!(edge_factor([500.0, 250.0], min, max, 100.0), 1.0);
        assert!((edge_factor([0.0, 250.0], min, max, 100.0) - 7.0 / 27.0).abs() < 1e-6);
        assert_eq!(edge_factor([-60.0, 250.0], min, max, 100.0), 0.0);
        assert_eq!(edge_factor([500.0, 600.0], min, max, 0.0), 0.0);
    }

    #[test]
    fn visibility_is_the_uncovered_share_around_the_light() {
        assert_eq!(visibility(|_, _| 1.0, [10.0, 10.0], 5.0), 0.0);
        assert_eq!(visibility(|_, _| 0.0, [10.0, 10.0], 5.0), 1.0);
        let half = visibility(|x, _| if x < 10.0 { 1.0 } else { 0.0 }, [10.0, 10.0], 5.0);
        assert!((half - 9.0 / 17.0).abs() < 1e-6, "{half}");
    }
}
