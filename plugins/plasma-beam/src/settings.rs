use std::f32::consts::LOG2_E;

use craft_plugin::Cx;

use crate::look::{LOOKS, Look};
use crate::noise::{self, Fractal};
use crate::path::{Path, Point};
use crate::shade::{self, Composite, Shading};
use crate::spec::*;

const VISIBLE_ENERGY: f32 = 0.004;
const MAX_REACH: f32 = 600.0;
const FLICKER_SEED: u32 = 0x00f1_1c3e;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    TwoPoints([Point; 2]),
    Masks,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub core_radius: f32,
    pub glow_radius: f32,
    pub taper: [f32; 2],
    pub trim: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub fractal: Fractal,
    pub wavelength: f32,
    pub evolution: f32,
    pub drift: f32,
    pub core_amplitude: f32,
    pub glow_amplitude: f32,
    pub core_width: f32,
    pub glow_width: f32,
    pub pin: f32,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowBand {
    pub sigma: f32,
    pub energy: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub source: Source,
    pub shape: Shape,
    pub motion: Motion,
    pub glow: [GlowBand; 5],
    pub reach: f32,
    pub strands: u32,
    pub shading: Shading,
}

struct Reader<'a>(&'a Cx);

impl Reader<'_> {
    fn number(&self, id: &str) -> f32 {
        let value = self.0.f(id) as f32;
        if value.is_finite() { value } else { 0.0 }
    }

    fn ratio(&self, id: &str) -> f32 {
        self.number(id).max(0.0) / 100.0
    }

    fn point(&self, id: &str) -> Point {
        self.0.point(id).map(|v| v as f32)
    }
}

impl Settings {
    pub fn read(cx: &Cx) -> Settings {
        let read = Reader(cx);
        let look = &LOOKS[cx.index(PRESET).min(LOOKS.len() - 1)];
        let scale = (cx.scale as f32).max(1e-3);
        let seed = noise::mix(cx.instance as u32, (cx.instance >> 32) as u32);
        let source = match cx.index(CORE_TYPE) {
            0 => Source::TwoPoints([read.point(CORE_START), read.point(CORE_END)]),
            _ => Source::Masks,
        };
        let core_radius = 0.5 * read.number(CORE_SIZE).max(0.0) * look.core_width * scale;
        let shape = Shape {
            core_radius,
            glow_radius: core_radius.max(0.5 * scale),
            taper: [read.ratio(START_SIZE), read.ratio(END_SIZE)],
            trim: [read.ratio(START_OFFSET).min(1.0), read.ratio(END_OFFSET).min(1.0)],
        };
        let motion = motion(look, &read, scale, cx.time as f32, seed);
        let glow = bands(look, &read, scale);
        let reach = glow.iter().map(visible_radius).fold(0.0, f32::max) + motion.core_amplitude.max(motion.glow_amplitude) + core_radius;
        let flicker = flicker(look, &read, cx.time as f32, seed);
        Settings { source, shape, motion, glow, reach: reach.min(MAX_REACH * scale), strands: look.strands.max(1), shading: shading(look, &read, flicker) }
    }

    pub fn padding(&self, paths: &[Path], size: [u32; 2]) -> u32 {
        let mut points = paths.iter().flat_map(|path| path.points().iter());
        let Some(&first) = points.next() else { return 0 };
        let (min, max) = points.fold((first, first), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
        let overflow = [self.reach - min[0], self.reach - min[1], max[0] + self.reach - size[0] as f32, max[1] + self.reach - size[1] as f32];
        overflow.into_iter().fold(0.0f32, f32::max).min(self.reach).ceil() as u32
    }
}

fn visible_radius(band: &GlowBand) -> f32 {
    let peak = band.energy[0] + band.energy[1];
    band.sigma * (2.0 * (peak / VISIBLE_ENERGY).max(1.0).ln()).sqrt()
}

fn bands(look: &Look, read: &Reader, scale: f32) -> [GlowBand; 5] {
    let spread = read.ratio(GLOW_SPREAD).max(0.01) * scale;
    let intensity = read.ratio(GLOW_INTENSITY);
    let bias = read.number(GLOW_BIAS).clamp(-100.0, 100.0) / 50.0;
    let last = (look.glow.len() - 1) as f32;
    std::array::from_fn(|i| {
        let layer = look.glow[i];
        let tightness = 1.0 - 2.0 * i as f32 / last;
        let energy = layer.weight * intensity * (bias * tightness).exp2();
        GlowBand { sigma: layer.radius * spread, energy: [energy * layer.heat, energy * (1.0 - layer.heat)] }
    })
}

fn motion(look: &Look, read: &Reader, scale: f32, time: f32, seed: u32) -> Motion {
    let d = look.distortion;
    let amplitude = d.amplitude * read.ratio(DISTORTION_AMOUNT) * scale;
    let speed = read.ratio(DISTORTION_SPEED);
    Motion {
        fractal: Fractal { octaves: d.octaves, roughness: d.roughness },
        wavelength: (d.wavelength * read.ratio(DISTORTION_SCALE) * scale).max(1.0),
        evolution: time * d.speed * speed,
        drift: time * d.flow * speed,
        core_amplitude: amplitude * d.core * read.ratio(DISTORTION_CORE),
        glow_amplitude: amplitude * d.glow * read.ratio(DISTORTION_GLOW),
        core_width: d.core_width,
        glow_width: d.glow_width,
        pin: d.pin,
        seed,
    }
}

fn flicker(look: &Look, read: &Reader, time: f32, seed: u32) -> f32 {
    let amount = (look.flicker.amount * read.ratio(FLICKER_INTENSITY)).min(1.0);
    let phase = time * look.flicker.rate * read.ratio(FLICKER_SPEED);
    let wave = Fractal { octaves: 3, roughness: 0.6 }.sample(phase, 0.5, None, noise::mix(seed, FLICKER_SEED));
    1.0 - amount * (0.5 + 0.5 * wave).clamp(0.0, 1.0)
}

fn shading(look: &Look, read: &Reader, flicker: f32) -> Shading {
    let palette = look.palette;
    let base = match read.0.index(COLOUR_SOURCE) {
        0 => palette.colour,
        _ => {
            let c = read.0.color(GLOW_COLOUR);
            [c[0], c[1], c[2]].map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
        }
    };
    let inner = shade::mix(shade::rotate_hue(base, palette.inner_hue), [1.0; 3], palette.inner_white);
    let outer = shade::rotate_hue(base, palette.outer_hue);
    let energy = flicker * LOG2_E;
    let core_gain = look.core_gain * read.ratio(CORE_BRIGHTNESS) * flicker;
    Shading {
        inner: inner.map(|c| c * energy),
        outer: outer.map(|c| c * energy),
        core: shade::mix([1.0; 3], inner, palette.core_tint).map(|c| c * core_gain),
        composite: Composite::from_index(read.0.index(COMPOSITE)),
        alpha_boost: read.ratio(ALPHA_BOOST),
    }
}
