use craft_plugin::math::pow;

use crate::noise;
use crate::path::{Path, Point, Span};
use crate::settings::{Motion, Shape};

const MAX_NODES: f32 = 40_000.0;
const SWELL_SEED: u32 = 0x5bd1_e995;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub at: Point,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub core: Vec<Node>,
    pub glow: Vec<Node>,
}

struct Field<'a> {
    motion: &'a Motion,
    cells_per_px: f32,
    period: Option<i32>,
    seed: u32,
}

impl Field<'_> {
    fn along<'a>(path: &Path, motion: &'a Motion, salt: u32) -> Field<'a> {
        let length = path.length();
        let seed = noise::mix(motion.seed, salt);
        if path.is_closed() && length > 0.0 {
            let cells = (length / motion.wavelength).round().max(1.0);
            Field { motion, cells_per_px: cells / length, period: Some(cells as i32), seed }
        } else {
            Field { motion, cells_per_px: 1.0 / motion.wavelength, period: None, seed }
        }
    }

    fn sample(&self, distance: f32, seed: u32) -> f32 {
        let x = distance * self.cells_per_px - self.motion.drift;
        self.motion.fractal.sample(x, self.motion.evolution, self.period, seed)
    }

    fn bend(&self, distance: f32) -> f32 {
        self.sample(distance, self.seed)
    }

    fn swell(&self, distance: f32) -> f32 {
        self.sample(distance, self.seed ^ SWELL_SEED)
    }
}

fn node_step(length: f32, motion: &Motion) -> f32 {
    let finest = motion.wavelength / (1u32 << motion.fractal.octaves.clamp(1, 12).saturating_sub(1)) as f32;
    (finest / 4.0).clamp(1.0, 6.0).max(length / MAX_NODES)
}

fn envelope(span: &Span, distance: f32, motion: &Motion) -> f32 {
    if span.loops || motion.pin <= 0.0 || span.length() <= 0.0 {
        return 1.0;
    }
    let along = ((distance - span.from) / span.length()).clamp(0.0, 1.0);
    pow(4.0 * along * (1.0 - along), 0.5 * motion.pin)
}

pub fn taper(ends: [f32; 2], along: f32) -> f32 {
    ends[0] + (ends[1] - ends[0]) * along
}

pub fn stroke(path: &Path, shape: &Shape, motion: &Motion, salt: u32) -> Stroke {
    let span = path.span(shape.trim, node_step(path.length(), motion));
    let field = Field::along(path, motion, salt);
    let length = span.length();
    let (core, glow) = span
        .samples
        .iter()
        .map(|sample| {
            let along = if length > 0.0 { (sample.distance - span.from) / length } else { 0.0 };
            let size = taper(shape.taper, along);
            let bend = field.bend(sample.distance) * envelope(&span, sample.distance, motion);
            let swell = field.swell(sample.distance);
            let node = |amplitude: f32, radius: f32, wobble: f32| Node {
                at: [sample.at[0] + sample.normal[0] * bend * amplitude, sample.at[1] + sample.normal[1] * bend * amplitude],
                radius: radius * size * (1.0 + wobble * swell).max(0.0),
            };
            (node(motion.core_amplitude, shape.core_radius, motion.core_width), node(motion.glow_amplitude, shape.glow_radius, motion.glow_width))
        })
        .unzip();
    Stroke { core, glow }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Fractal;

    fn still() -> Motion {
        Motion {
            fractal: Fractal { octaves: 3, roughness: 0.5 },
            wavelength: 50.0,
            evolution: 0.0,
            drift: 0.0,
            core_amplitude: 0.0,
            glow_amplitude: 0.0,
            core_width: 0.0,
            glow_width: 0.0,
            pin: 0.0,
            seed: 1,
        }
    }

    fn shape(taper: [f32; 2]) -> Shape {
        Shape { core_radius: 4.0, glow_radius: 4.0, taper, trim: [0.0, 1.0] }
    }

    #[test]
    fn taper_interpolates_size_along_arc_length() {
        let path = Path::new(vec![[0.0, 0.0], [30.0, 0.0], [30.0, 10.0]], false);
        let stroke = stroke(&path, &shape([1.0, 0.0]), &still(), 0);
        let corner = stroke.core.iter().find(|n| n.at == [30.0, 0.0]).map(|n| n.radius);
        assert_eq!(corner, Some(1.0));
        assert_eq!(stroke.core.last().map(|n| n.radius), Some(0.0));
    }

    #[test]
    fn distortion_displaces_along_the_normal_and_pins_the_ends() {
        let motion = Motion { core_amplitude: 10.0, pin: 1.0, ..still() };
        let path = Path::new(vec![[0.0, 0.0], [400.0, 0.0]], false);
        let stroke = stroke(&path, &shape([1.0, 1.0]), &motion, 0);
        assert!(stroke.core.iter().all(|n| (n.at[0] - 200.0).abs() <= 200.0 + 1e-3));
        assert_eq!(stroke.core[0].at, [0.0, 0.0]);
        assert!(stroke.core.iter().any(|n| n.at[1].abs() > 1.0));
        assert!(stroke.glow.iter().all(|n| n.at[1] == 0.0));
    }
}
