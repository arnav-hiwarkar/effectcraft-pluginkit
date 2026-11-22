use craft_plugin::math::cos;
use craft_plugin::prelude::*;

use crate::grid::{Band, Grid, coarse_rows};
use crate::math::{TAU, bilinear, blur_radius, fract, hash, luma, reduction};

pub const MASKS: [&str; 3] = ["Aperture Grille", "Slot Mask", "Dots"];

#[derive(Clone, Copy, Debug)]
pub struct Crt {
    pub size: [u32; 2],
    pub curvature: f32,
    pub corner: f32,
    pub mask: u32,
    pub pitch: f32,
    pub mask_strength: f32,
    pub scan_strength: f32,
    pub scan_period: f32,
    pub roll: f32,
    pub convergence: f32,
    pub glow: f32,
    pub glow_sigma: f32,
    pub glow_threshold: f32,
    pub vignette: f32,
    pub noise: f32,
    pub flicker: f32,
    pub compensation: f32,
    pub contrast: f32,
    pub frame: u32,
}

pub fn spec() -> EffectSpec {
    EffectSpec::new("crt", "CRT Display", "Stylize")
        .group(
            "Screen",
            [
                Param::slider("curvature", "Curvature", 0.06, 0.0..=0.3),
                Param::slider("corner", "Corner Radius", 0.06, 0.0..=0.5),
                Param::percent("vignette", "Vignette", 35.0),
            ],
        )
        .group(
            "Mask",
            [Param::popup("mask", "Mask", &MASKS, 0), Param::slider("pitch", "Pitch", 3.0, 1.0..=16.0), Param::percent("mask_strength", "Strength", 45.0)],
        )
        .group(
            "Scanlines",
            [
                Param::percent("scan_strength", "Strength", 45.0),
                Param::slider("scan_period", "Period", 3.0, 1.0..=16.0),
                Param::slider("roll", "Roll Speed", 0.25, -4.0..=4.0),
            ],
        )
        .group(
            "Signal",
            [
                Param::slider("convergence", "Convergence Error", 1.2, -8.0..=8.0),
                Param::percent("noise", "Noise", 5.0),
                Param::percent("flicker", "Flicker", 3.0),
            ],
        )
        .group(
            "Phosphor Glow",
            [
                Param::percent("glow", "Amount", 60.0),
                Param::slider("glow_radius", "Radius", 10.0, 1.0..=60.0),
                Param::slider("glow_threshold", "Threshold", 0.35, 0.0..=2.0),
            ],
        )
        .group("Compensation", [Param::percent("compensation", "Brightness", 100.0), Param::slider("contrast", "Contrast", 1.05, 0.5..=2.0)])
        .time_dependent()
}

impl Crt {
    pub fn from_cx(cx: &Cx) -> Crt {
        let s = cx.scale as f32;
        let at = |id: &str| cx.f(&format!("{}/{id}", group(id))) as f32;
        let roll_rows = at("roll") as f64 * cx.time;
        let scan_period = (at("scan_period") * s).max(0.5);
        Crt {
            size: cx.size,
            curvature: at("curvature"),
            corner: at("corner"),
            mask: cx.index("Mask/mask").min(2) as u32,
            pitch: (at("pitch") * s).max(0.5),
            mask_strength: at("mask_strength") / 100.0,
            scan_strength: at("scan_strength") / 100.0,
            scan_period,
            roll: (roll_rows.rem_euclid(1.0) as f32) * scan_period,
            convergence: at("convergence") * s,
            glow: at("glow") / 100.0,
            glow_sigma: (at("glow_radius") * s).max(0.5),
            glow_threshold: at("glow_threshold"),
            vignette: at("vignette") / 100.0,
            noise: at("noise") / 100.0,
            flicker: at("flicker") / 100.0,
            compensation: at("compensation") / 100.0,
            contrast: at("contrast"),
            frame: (cx.time * cx.frame_rate.max(1.0)).round().max(0.0) as u32,
        }
    }

    pub fn factor(&self) -> u32 {
        reduction(self.glow_sigma)
    }

    pub fn coarse_sigma(&self) -> f32 {
        (self.glow_sigma / self.factor() as f32).max(0.3)
    }

    pub fn reach(&self) -> u32 {
        let d = self.factor() as i32;
        let bend = (self.curvature * self.size[1] as f32 * 0.5).ceil() as i32 + 2;
        let glow = (blur_radius(self.coarse_sigma()) + 3) * d;
        (bend + glow + self.convergence.abs().ceil() as i32) as u32
    }

    pub fn floats(&self) -> Vec<f32> {
        vec![
            self.curvature,
            self.corner,
            self.pitch,
            self.mask_strength,
            self.scan_strength,
            self.scan_period,
            self.roll,
            self.convergence,
            self.glow,
            self.vignette,
            self.noise,
            self.flicker,
            self.compensation,
            self.contrast,
            self.glow_threshold,
        ]
    }

    pub fn uints(&self) -> Vec<u32> {
        vec![self.mask, self.frame, self.factor()]
    }

    pub fn gain(&self) -> f32 {
        let average = (1.0 - self.mask_strength * 0.5) * (1.0 - self.scan_strength * 0.5);
        1.0 + self.compensation * (1.0 / average.max(0.05) - 1.0)
    }
}

fn group(id: &str) -> &'static str {
    match id {
        "curvature" | "corner" | "vignette" => "Screen",
        "pitch" | "mask_strength" => "Mask",
        "scan_strength" | "scan_period" | "roll" => "Scanlines",
        "convergence" | "noise" | "flicker" => "Signal",
        "glow" | "glow_radius" | "glow_threshold" => "Phosphor Glow",
        _ => "Compensation",
    }
}

pub fn bright(p: Pixel, threshold: f32) -> Pixel {
    let l = luma(p);
    let k = (l - threshold).max(0.0) / l.max(1e-4);
    [p[0] * k, p[1] * k, p[2] * k, 0.0]
}

pub fn mask_rgb(crt: &Crt, x: f32, y: f32) -> [f32; 3] {
    let p = crt.pitch;
    let s = crt.mask_strength;
    let (phase, rows) = match crt.mask {
        1 => {
            let triad = (x / (3.0 * p)).floor();
            let shift = if triad.rem_euclid(2.0) >= 1.0 { 0.5 } else { 0.0 };
            let slot = 0.5 + 0.5 * cos(TAU * fract(y / (3.0 * p) + shift));
            (fract(x / (3.0 * p)), 1.0 - s * 0.6 * smooth_gap(slot))
        }
        2 => {
            let row = (y / p).floor();
            let offset = if row.rem_euclid(2.0) >= 1.0 { 0.5 } else { 0.0 };
            let dot = 0.5 - 0.5 * cos(TAU * fract(y / p));
            (fract(x / (3.0 * p) + offset), 1.0 - s * (1.0 - dot))
        }
        _ => (fract(x / (3.0 * p)), 1.0),
    };
    std::array::from_fn(|c| {
        let w = 0.5 + 0.5 * cos(TAU * fract(phase - c as f32 / 3.0));
        (1.0 - s + s * w * w * 1.5).min(1.0 + s) * rows
    })
}

fn smooth_gap(slot: f32) -> f32 {
    slot * slot * slot * slot
}

pub fn scan(crt: &Crt, y: f32) -> f32 {
    let w = 0.5 + 0.5 * cos(TAU * fract((y + crt.roll) / crt.scan_period));
    1.0 - crt.scan_strength * (1.0 - w)
}

pub fn warp(crt: &Crt, x: f32, y: f32) -> (f32, f32, f32) {
    let [w, h] = crt.size.map(|v| v as f32);
    let u = x / w * 2.0 - 1.0;
    let v = y / h * 2.0 - 1.0;
    let su = u * (1.0 + crt.curvature * v * v);
    let sv = v * (1.0 + crt.curvature * u * u);
    let r = crt.corner;
    let aspect = w / h;
    let qx = (su.abs() - 1.0) * aspect + r;
    let qy = sv.abs() - 1.0 + r;
    let outside = (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt();
    let distance = outside + qx.max(qy).min(0.0) - r;
    let shape = (0.5 - distance * h * 0.5).clamp(0.0, 1.0);
    ((su + 1.0) * 0.5 * w - 0.5, (sv + 1.0) * 0.5 * h - 0.5, shape * (1.0 - crt.vignette * 0.5 * (su * su + sv * sv)).max(0.0))
}

pub fn shade(crt: &Crt, source: impl Fn(i32, i32) -> Pixel, glow: impl Fn(f32, f32) -> Pixel, x: u32, y: u32) -> Pixel {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let (sx, sy, shape) = warp(crt, fx, fy);
    if shape <= 0.0 {
        return [0.0; 4];
    }
    let red = bilinear(&source, sx + crt.convergence, sy);
    let green = bilinear(&source, sx, sy);
    let blue = bilinear(&source, sx - crt.convergence, sy);
    let halo = glow(sx, sy);
    let mask = mask_rgb(crt, fx, fy);
    let lines = scan(crt, fy);
    let gain = crt.gain();
    let alpha = green[3];
    let flick = 1.0 + crt.flicker * (hash(crt.frame, 7, 3) - 0.5) * 2.0;
    let grain = (hash(x, y, crt.frame) - 0.5) * crt.noise;
    let base = [red[0], green[1], blue[2]];
    let rgb: [f32; 3] = std::array::from_fn(|c| {
        let lit = base[c] * mask[c] * lines * gain + halo[c] * crt.glow;
        let contrasted = (lit - 0.18 * alpha) * crt.contrast + 0.18 * alpha;
        ((contrasted + grain * alpha) * flick).max(0.0) * shape
    });
    [rgb[0], rgb[1], rgb[2], alpha * shape]
}

pub fn render(crt: &Crt, frame: &mut Frame) {
    let band = Band::of(frame.input);
    let d = crt.factor() as i32;
    let width = (crt.size[0] as i32 + d - 1) / d;
    let (first, last) = coarse_rows(band.top, band.top + band.rows, d, crt.size[1]);
    let threshold = crt.glow_threshold;
    let reduced = Grid::reduce(width, first, last, d, |x, y| bright(band.at(x, y), threshold));
    let sigma = crt.coarse_sigma();
    let glow = reduced.blur(false, false, [sigma; 3]).blur(true, false, [sigma; 3]);
    let x0 = frame.origin[0] as u32;
    for (y, pixels) in frame.rows_mut() {
        for (x, pixel) in (x0..).zip(pixels.iter_mut()) {
            *pixel = shade(crt, |i, j| band.at(i, j), |gx, gy| glow.sample_full(d, gx, gy), x, y as u32);
        }
    }
}

pub fn plan(crt: &Crt) -> GpuPlan {
    let d = crt.factor();
    let coarse = crt.size.map(|s| s.div_ceil(d));
    let sigma = crt.coarse_sigma();
    let mut plan = GpuPlan::new();
    let reduced = plan.scratch(coarse);
    let across = plan.scratch(coarse);
    let glow = plan.scratch(coarse);
    plan.pass(GpuPass::new("retro_crt_emit", GpuImage::Source, reduced).floats(&[crt.glow_threshold]).uints(&[d]))
        .pass(GpuPass::new("retro_blur", reduced, across).floats(&[sigma; 3]).uints(&[0, 0]))
        .pass(GpuPass::new("retro_blur", across, glow).floats(&[sigma; 3]).uints(&[1, 0]))
        .pass(GpuPass::new("retro_crt", GpuImage::Source, GpuImage::Output).aux(glow).floats(&crt.floats()).uints(&crt.uints()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crt() -> Crt {
        Crt {
            size: [200, 100],
            curvature: 0.0,
            corner: 0.0,
            mask: 0,
            pitch: 2.0,
            mask_strength: 0.5,
            scan_strength: 0.5,
            scan_period: 4.0,
            roll: 0.0,
            convergence: 0.0,
            glow: 0.0,
            glow_sigma: 10.0,
            glow_threshold: 0.5,
            vignette: 0.0,
            noise: 0.0,
            flicker: 0.0,
            compensation: 1.0,
            contrast: 1.0,
            frame: 0,
        }
    }

    #[test]
    fn bright_keeps_only_light_above_the_threshold() {
        assert_eq!(bright([0.2, 0.2, 0.2, 1.0], 0.5), [0.0; 4]);
        let b = bright([1.0, 1.0, 1.0, 1.0], 0.5);
        assert!((b[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn scanlines_are_bright_at_the_line_and_dark_between() {
        let c = crt();
        assert!((scan(&c, 0.0) - 1.0).abs() < 1e-6);
        assert!((scan(&c, 2.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn compensation_undoes_the_average_darkening() {
        let c = crt();
        assert!((c.gain() - 1.0 / 0.5625).abs() < 1e-5);
        assert_eq!(Crt { compensation: 0.0, ..c }.gain(), 1.0);
    }

    #[test]
    fn grille_lights_one_channel_per_stripe() {
        let c = crt();
        let red = mask_rgb(&c, 0.0, 0.0);
        assert!(red[0] > red[1] && red[0] > red[2]);
        let green = mask_rgb(&c, 2.0, 0.0);
        assert!(green[1] > green[0] && green[1] > green[2]);
    }

    #[test]
    fn warp_is_identity_when_flat_and_blacks_the_corners_when_curved() {
        let flat = crt();
        let (sx, sy, shape) = warp(&flat, 50.5, 20.5);
        assert!((sx - 50.0).abs() < 1e-4 && (sy - 20.0).abs() < 1e-4 && shape == 1.0);
        let curved = Crt { curvature: 0.2, corner: 0.1, ..flat };
        assert_eq!(warp(&curved, 0.5, 0.5).2, 0.0);
        assert_eq!(warp(&curved, 100.0, 50.0).2, 1.0);
    }
}
