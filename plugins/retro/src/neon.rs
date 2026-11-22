use craft_plugin::math::smoothstep;
use craft_plugin::prelude::*;

use crate::grid::{Band, Grid, coarse_rows};
use crate::math::{blur_radius, hash, luma, reduction};

pub const SOURCES: [&str; 2] = ["Luma", "Alpha"];

#[derive(Clone, Copy, Debug)]
pub struct Neon {
    pub size: [u32; 2],
    pub alpha: bool,
    pub threshold: f32,
    pub softness: f32,
    pub step: i32,
    pub core: [f32; 4],
    pub glow: [f32; 4],
    pub intensity: f32,
    pub sigma: f32,
    pub falloff: f32,
    pub flicker: f32,
    pub lines_only: bool,
    pub frame: u32,
}

pub fn spec() -> EffectSpec {
    EffectSpec::new("neon", "Neon Outline", "Stylize")
        .group(
            "Edges",
            [
                Param::popup("source", "Detect On", &SOURCES, 0),
                Param::slider("threshold", "Threshold", 0.12, 0.0..=1.0),
                Param::slider("softness", "Softness", 0.06, 0.0..=0.5),
                Param::slider("thickness", "Thickness", 2.0, 1.0..=12.0),
            ],
        )
        .group("Colour", [Param::color("core", "Core", [1.0, 0.95, 1.0, 1.0]), Param::color("glow", "Glow", [1.0, 0.1, 0.75, 1.0])])
        .group(
            "Bloom",
            [
                Param::slider("intensity", "Intensity", 2.0, 0.0..=10.0),
                Param::slider("radius", "Radius", 20.0, 1.0..=80.0),
                Param::percent("falloff", "Falloff", 60.0),
                Param::percent("flicker", "Flicker", 0.0),
            ],
        )
        .param(Param::checkbox("lines_only", "Lines Only", false))
        .time_dependent()
}

impl Neon {
    pub fn from_cx(cx: &Cx) -> Neon {
        let s = cx.scale as f32;
        Neon {
            size: cx.size,
            alpha: cx.index("Edges/source") == 1,
            threshold: cx.f("Edges/threshold") as f32,
            softness: (cx.f("Edges/softness") as f32).max(1e-3),
            step: ((cx.f("Edges/thickness") as f32 * s).round() as i32).max(1),
            core: cx.color("Colour/core"),
            glow: cx.color("Colour/glow"),
            intensity: cx.f("Bloom/intensity") as f32,
            sigma: (cx.f("Bloom/radius") as f32 * s).max(0.5),
            falloff: cx.f("Bloom/falloff") as f32 / 100.0,
            flicker: cx.f("Bloom/flicker") as f32 / 100.0,
            lines_only: cx.bool("lines_only"),
            frame: (cx.time * cx.frame_rate.max(1.0)).round().max(0.0) as u32,
        }
    }

    pub fn factor(&self) -> u32 {
        reduction(self.sigma)
    }

    pub fn sigmas(&self) -> [f32; 3] {
        let d = self.factor() as f32;
        [0.25, 0.5, 1.0].map(|k| (self.sigma * k / d).max(0.3))
    }

    pub fn reach(&self) -> u32 {
        let d = self.factor() as i32;
        ((blur_radius(self.sigmas()[2]) + 3) * d + self.step + 1) as u32
    }

    pub fn floats(&self) -> Vec<f32> {
        let mut f = vec![self.threshold, self.softness, self.intensity, self.falloff, self.flicker];
        f.extend_from_slice(&self.core);
        f.extend_from_slice(&self.glow);
        f
    }

    pub fn uints(&self) -> Vec<u32> {
        vec![u32::from(self.alpha), self.step as u32, self.factor(), u32::from(self.lines_only), self.frame]
    }
}

pub fn signal(neon: &Neon, p: Pixel) -> f32 {
    if neon.alpha { p[3] } else { luma(p) }
}

pub fn line(neon: &Neon, at: impl Fn(i32, i32) -> Pixel, x: i32, y: i32) -> f32 {
    let t = neon.step;
    let g = |dx: i32, dy: i32| signal(neon, at(x + dx * t, y + dy * t));
    let gx = g(1, -1) + 2.0 * g(1, 0) + g(1, 1) - g(-1, -1) - 2.0 * g(-1, 0) - g(-1, 1);
    let gy = g(-1, 1) + 2.0 * g(0, 1) + g(1, 1) - g(-1, -1) - 2.0 * g(0, -1) - g(1, -1);
    let magnitude = (gx * gx + gy * gy).sqrt() * 0.25;
    smoothstep(neon.threshold - neon.softness, neon.threshold + neon.softness, magnitude)
}

pub fn bloom(neon: &Neon, b: Pixel) -> f32 {
    let f = neon.falloff;
    (b[0] + f * b[1] + f * f * b[2]) / (1.0 + f + f * f) * neon.intensity
}

pub fn composite(neon: &Neon, source: Pixel, edge: f32, blurred: Pixel) -> Pixel {
    let flick = 1.0 - neon.flicker * hash(neon.frame, 11, 5);
    let lit = edge * flick;
    let halo = bloom(neon, blurred) * flick;
    let rgb: [f32; 3] = std::array::from_fn(|c| neon.core[c] * lit + neon.glow[c] * halo);
    let cover = (lit + halo * luma(neon.glow)).min(1.0);
    if neon.lines_only {
        [rgb[0], rgb[1], rgb[2], cover]
    } else {
        let keep = 1.0 - lit;
        [source[0] * keep + rgb[0], source[1] * keep + rgb[1], source[2] * keep + rgb[2], source[3].max(cover)]
    }
}

pub fn render(neon: &Neon, frame: &mut Frame) {
    let band = Band::of(frame.input);
    let d = neon.factor() as i32;
    let width = (neon.size[0] as i32 + d - 1) / d;
    let (first, last) = coarse_rows(band.top, band.top + band.rows, d, neon.size[1]);
    let at = |x: i32, y: i32| band.at(x, y);
    let reduced = Grid::reduce(width, first, last, d, |x, y| [line(neon, at, x, y), 0.0, 0.0, 0.0]);
    let sigmas = neon.sigmas();
    let glow = reduced.blur(false, true, sigmas).blur(true, false, sigmas);
    let x0 = frame.origin[0] as i32;
    for (y, pixels) in frame.rows_mut() {
        let y = y as i32;
        for (x, pixel) in (x0..).zip(pixels.iter_mut()) {
            let edge = line(neon, at, x, y);
            *pixel = composite(neon, band.at(x, y), edge, glow.sample_full(d, x as f32, y as f32));
        }
    }
}

pub fn plan(neon: &Neon) -> GpuPlan {
    let d = neon.factor();
    let coarse = neon.size.map(|s| s.div_ceil(d));
    let sigmas = neon.sigmas();
    let mut plan = GpuPlan::new();
    let reduced = plan.scratch(coarse);
    let across = plan.scratch(coarse);
    let glow = plan.scratch(coarse);
    plan.pass(GpuPass::new("retro_neon_emit", GpuImage::Source, reduced).floats(&neon.floats()).uints(&neon.uints()))
        .pass(GpuPass::new("retro_blur", reduced, across).floats(&sigmas).uints(&[0, 1]))
        .pass(GpuPass::new("retro_blur", across, glow).floats(&sigmas).uints(&[1, 0]))
        .pass(GpuPass::new("retro_neon", GpuImage::Source, GpuImage::Output).aux(glow).floats(&neon.floats()).uints(&neon.uints()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neon() -> Neon {
        Neon {
            size: [64, 64],
            alpha: false,
            threshold: 0.2,
            softness: 0.05,
            step: 1,
            core: [1.0, 1.0, 1.0, 1.0],
            glow: [0.0, 1.0, 0.0, 1.0],
            intensity: 1.0,
            sigma: 6.0,
            falloff: 0.5,
            flicker: 0.0,
            lines_only: true,
            frame: 0,
        }
    }

    fn step_edge(x: i32, _y: i32) -> Pixel {
        if x >= 10 { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] }
    }

    #[test]
    fn a_step_edge_is_a_full_line_and_flat_areas_are_empty() {
        let n = neon();
        assert_eq!(line(&n, step_edge, 10, 5), 1.0);
        assert_eq!(line(&n, step_edge, 9, 5), 1.0);
        assert_eq!(line(&n, step_edge, 3, 5), 0.0);
        assert_eq!(line(&n, step_edge, 20, 5), 0.0);
        let alpha = Neon { alpha: true, ..n };
        assert_eq!(line(&alpha, step_edge, 10, 5), 0.0);
    }

    #[test]
    fn bloom_weights_the_three_radii_by_falloff() {
        let n = neon();
        assert!((bloom(&n, [1.0, 1.0, 1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((bloom(&n, [1.75, 0.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(n.sigmas(), [0.75, 1.5, 3.0]);
    }

    #[test]
    fn lines_only_drops_the_source_and_over_keeps_it() {
        let n = neon();
        let source = [0.2, 0.4, 0.6, 1.0];
        assert_eq!(composite(&n, source, 0.0, [0.0; 4]), [0.0, 0.0, 0.0, 0.0]);
        let over = Neon { lines_only: false, ..n };
        assert_eq!(composite(&over, source, 0.0, [0.0; 4]), source);
        assert_eq!(composite(&over, source, 1.0, [0.0; 4]), [1.0, 1.0, 1.0, 1.0]);
    }
}
