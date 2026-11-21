use craft_plugin::Pixel;

pub type Rgb = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composite {
    Add,
    Screen,
    Transparent,
    Black,
}

impl Composite {
    pub fn from_index(index: usize) -> Composite {
        [Composite::Add, Composite::Screen, Composite::Transparent, Composite::Black][index.min(3)]
    }

    pub fn keeps_source(self) -> bool {
        matches!(self, Composite::Add | Composite::Screen)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shading {
    pub inner: Rgb,
    pub outer: Rgb,
    pub core: Rgb,
    pub composite: Composite,
    pub alpha_boost: f32,
}

pub fn exp2_neg(v: f32) -> f32 {
    let x = -v.clamp(0.0, 126.0);
    let whole = x.round();
    let f = x - whole;
    let poly = 1.0 + f * (std::f32::consts::LN_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_4))));
    f32::from_bits(((whole as i32 + 127) as u32) << 23) * poly
}

pub fn one_minus_exp2_neg(v: f32) -> f32 {
    if v < 0.03 {
        let x = v * std::f32::consts::LN_2;
        x * (1.0 - 0.5 * x)
    } else {
        1.0 - exp2_neg(v)
    }
}

pub fn rotate_hue(colour: Rgb, degrees: f32) -> Rgb {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let k = (1.0 - cos) / 3.0;
    let s = sin / 3.0f32.sqrt();
    let rows = [[cos + k, k - s, k + s], [k + s, cos + k, k - s], [k - s, k + s, cos + k]];
    rows.map(|r| (r[0] * colour[0] + r[1] * colour[1] + r[2] * colour[2]).max(0.0))
}

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t)
}

impl Shading {
    pub fn glow(&self, energy: [f32; 2]) -> Rgb {
        if energy[0] + energy[1] <= 1e-7 {
            return [0.0; 3];
        }
        std::array::from_fn(|c| one_minus_exp2_neg(energy[0] * self.inner[c] + energy[1] * self.outer[c]))
    }

    pub fn beam(&self, glow: Rgb, coverage: f32) -> Rgb {
        std::array::from_fn(|c| glow[c] + (self.core[c] - glow[c]) * coverage)
    }

    pub fn blend(&self, source: Pixel, beam: Rgb) -> Pixel {
        let alpha = (beam[0].max(beam[1]).max(beam[2]) * self.alpha_boost).clamp(0.0, 1.0);
        let over = |a: f32| a + alpha * (1.0 - a);
        match self.composite {
            Composite::Add => [source[0] + beam[0], source[1] + beam[1], source[2] + beam[2], over(source[3])],
            Composite::Screen => {
                let screen = |c: usize| source[c] + beam[c] * (1.0 - source[c].clamp(0.0, 1.0));
                [screen(0), screen(1), screen(2), over(source[3])]
            }
            Composite::Transparent => [beam[0], beam[1], beam[2], alpha],
            Composite::Black => [beam[0], beam[1], beam[2], 1.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_exponential_tracks_the_exact_one() {
        for v in [0.0, 0.3, 1.0, 2.7, 9.5, 40.0] {
            let (fast, exact) = (exp2_neg(v), (-v).exp2());
            assert!((fast - exact).abs() <= exact * 1e-4, "{v}: {fast} vs {exact}");
        }
        assert_eq!(exp2_neg(1000.0), (-126.0f32).exp2());
    }

    #[test]
    fn faint_glow_keeps_relative_precision() {
        for v in [1e-7f32, 1e-4, 0.029, 0.031, 0.5] {
            let exact = -(-(v as f64) * std::f64::consts::LN_2).exp_m1();
            assert!((f64::from(one_minus_exp2_neg(v)) - exact).abs() <= exact * 1e-4, "{v}");
        }
    }

    #[test]
    fn hue_rotation_by_a_third_turn_cycles_the_primaries() {
        let rotated = rotate_hue([1.0, 0.0, 0.0], 120.0);
        assert!((rotated[1] - 1.0).abs() < 1e-5 && rotated[0] < 1e-5 && rotated[2] < 1e-5);
    }

    #[test]
    fn glow_saturates_toward_white_as_energy_grows() {
        let shading = Shading {
            inner: [0.2, 0.6, 1.4],
            outer: [0.2, 0.6, 1.4],
            core: [1.5; 3],
            composite: Composite::Add,
            alpha_boost: 1.0,
        };
        let faint = shading.glow([0.0, 0.2]);
        let hot = shading.glow([0.0, 50.0]);
        assert!(faint[2] / faint[0] > 6.0);
        assert!(hot[2] / hot[0] < 1.02);
    }
}
