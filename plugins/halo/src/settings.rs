use craft_plugin::prelude::*;

use crate::flare::*;
use crate::looks::{self, Element};

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

struct Read<'a>(&'a Cx);

impl Read<'_> {
    fn ratio(&self, id: &str) -> f32 {
        (self.0.f(id) / 100.0).max(0.0) as f32
    }

    fn colour(&self, id: &str) -> [f32; 3] {
        let [r, g, b, _] = self.0.color(id);
        [r, g, b]
    }

    fn count(&self, id: &str, look: u32) -> u32 {
        match self.0.f(id).round() as u32 {
            0 => look,
            n => n,
        }
    }

    fn element(&self, group: &str, base: Element, gain: f32, tint: [f32; 3], size: f32) -> ([f32; 3], f32) {
        if !self.0.bool(&format!("{group}/enable")) {
            return ([0.0; 3], 1.0);
        }
        let own = self.colour(&format!("{group}/color"));
        let amount = base.brightness * self.ratio(&format!("{group}/brightness")) * gain;
        let colour = std::array::from_fn(|c| base.colour[c] * own[c] * tint[c] * amount);
        (colour, (base.size * self.ratio(&format!("{group}/size")) * size).max(0.5))
    }
}

fn put(f: &mut [f32; FLOATS], at: usize, values: &[f32]) {
    f[at..at + values.len()].copy_from_slice(values);
}

pub fn pack(cx: &Cx, occlusion: f32) -> Pack {
    let read = Read(cx);
    let look = looks::look(cx.index("look"));
    let scale = cx.scale as f32;
    let size = scale * read.ratio("scale");
    let light = cx.point("light").map(|v| v as f32);
    let origin = cx.offset.map(|v| v as f32);
    let extent = [0, 1].map(|i| (cx.layer_size[i] * cx.scale) as f32);
    let centre = [origin[0] + 0.5 * extent[0], origin[1] + 0.5 * extent[1]];
    let edge = if cx.bool("Behaviour/edge") {
        edge_factor(light, origin, [origin[0] + extent[0], origin[1] + extent[1]], cx.px("Behaviour/edge_width") as f32)
    } else {
        1.0
    };
    let shimmer = read.ratio("Behaviour/shimmer");
    let phase = (cx.time * cx.f("Behaviour/shimmer_speed")) as f32;
    let flicker = 1.0 + 0.06 * shimmer * ((7.3 * phase).sin() * 0.6 + (13.1 * phase).sin() * 0.4);
    let gain = read.ratio("brightness") * edge * occlusion * flicker;
    let tint = read.colour("color");
    let rotation = (cx.f("rotation") as f32).to_radians();

    let mut f = [0.0; FLOATS];
    let mut u = [0; UINTS];
    put(&mut f, LIGHT, &light);
    put(&mut f, CENTRE, &centre);
    put(&mut f, ROTATION, &[rotation.cos(), rotation.sin()]);
    put(&mut f, PHASE, &[phase, shimmer]);
    put(&mut f, ORIGIN, &origin);

    let (colour, radius) = read.element("Glow", look.glow, gain, tint, size);
    put(&mut f, GLOW, &colour);
    put(&mut f, GLOW_RADIUS, &[radius, (look.glow.shape * read.ratio("Glow/falloff")).max(0.05)]);

    let (colour, length) = read.element("Rays", look.rays, gain, tint, size);
    put(&mut f, RAYS, &colour);
    let sharpness = look.rays.shape * read.ratio("Rays/sharpness");
    put(&mut f, RAYS_LENGTH, &[length, sharpness]);
    put(&mut f, RAYS_CUTOFF, &[rays_cutoff(sharpness)]);
    u[RAY_COUNT] = read.count("Rays/count", look.ray_count);

    let (colour, length) = read.element("Streak", look.streak, gain, tint, size);
    put(&mut f, STREAK, &colour);
    put(&mut f, STREAK_LENGTH, &[length, (look.streak.shape * read.ratio("Streak/thickness") * size).max(0.25)]);

    let (colour, radius) = read.element("Ring", look.ring, gain, tint, size);
    put(&mut f, RING, &colour);
    let width = (look.ring.shape * read.ratio("Ring/width") * size).max(0.25);
    put(&mut f, RING_RADIUS, &[radius, width, look.fringe * read.ratio("Ring/fringe")]);

    let (colour, ghost) = read.element("Ghosts", look.ghosts, gain, tint, size);
    put(&mut f, GHOSTS, &colour);
    let iris = (cx.f("Ghosts/iris_rotation") as f32).to_radians() + rotation;
    put(&mut f, GHOST_SIZE, &[ghost, iris, look.ghosts.shape * read.ratio("Ghosts/spread")]);
    u[GHOST_COUNT] = read.count("Ghosts/count", look.ghost_count).min(MAX_GHOSTS);
    u[BLADES] = read.count("Ghosts/blades", look.blades);
    u[SEED] = cx.f("Ghosts/seed").max(0.0) as u32;

    let (colour, radius) = read.element("Dirt", look.dirt, gain, tint, size);
    put(&mut f, DIRT, &colour);
    put(&mut f, DIRT_RADIUS, &[radius, (48.0 * scale).max(1.0)]);

    let (colour, radius) = read.element("Hoop", look.hoop, gain, tint, size);
    put(&mut f, HOOP, &colour);
    put(&mut f, HOOP_RADIUS, &[radius, (look.hoop.shape * read.ratio("Hoop/width") * size).max(0.25)]);

    u[BLEND] = cx.index("Composite/blend").min(1) as u32;
    u[FLARE_ONLY] = u32::from(cx.bool("Composite/flare_only"));
    Pack { f, u }
}

pub fn occlusion(cx: &Cx) -> f32 {
    if !cx.bool("Behaviour/occlusion") {
        return 1.0;
    }
    let Some(layer) = cx.layer("Behaviour/occluder") else { return 1.0 };
    let light = cx.point("light").map(|v| v as f32);
    let radius = cx.px("Behaviour/occlusion_radius") as f32;
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
