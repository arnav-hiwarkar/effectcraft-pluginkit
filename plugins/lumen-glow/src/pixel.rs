use craft_plugin::Pixel;

pub const CLEAR: Pixel = [0.0; 4];

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

pub fn add(a: Pixel, b: Pixel) -> Pixel {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

pub fn scale(a: Pixel, s: f32) -> Pixel {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

pub fn mul_add(acc: Pixel, a: Pixel, s: f32) -> Pixel {
    [acc[0] + a[0] * s, acc[1] + a[1] * s, acc[2] + a[2] * s, acc[3] + a[3] * s]
}

pub fn luma(rgb: [f32; 3]) -> f32 {
    LUMA[0] * rgb[0] + LUMA[1] * rgb[1] + LUMA[2] * rgb[2]
}

pub fn brightest(p: Pixel) -> f32 {
    p[0].max(p[1]).max(p[2])
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    columns: [Pixel; 3],
}

impl Mix {
    pub const ZERO: Mix = Mix { columns: [CLEAR; 3] };

    pub fn tint(colour: [f32; 3], amount: f32) -> Mix {
        let column = |j: usize| {
            let mut c = [0.0; 4];
            for (i, slot) in c.iter_mut().take(3).enumerate() {
                let keep = if i == j { 1.0 - amount } else { 0.0 };
                *slot = keep + amount * colour[i] * LUMA[j];
            }
            c
        };
        Mix { columns: [column(0), column(1), column(2)] }
    }

    pub fn identity() -> Mix {
        Mix::tint([0.0; 3], 0.0)
    }

    pub fn weighted(self, channel_weights: [f32; 3]) -> Mix {
        let w = [channel_weights[0], channel_weights[1], channel_weights[2], 0.0];
        Mix { columns: self.columns.map(|c| [c[0] * w[0], c[1] * w[1], c[2] * w[2], c[3] * w[3]]) }
    }

    pub fn plus(self, other: Mix) -> Mix {
        let [a, b, c] = self.columns;
        let [x, y, z] = other.columns;
        Mix { columns: [add(a, x), add(b, y), add(c, z)] }
    }

    pub fn from_floats(floats: &[f32]) -> Mix {
        Mix { columns: std::array::from_fn(|c| std::array::from_fn(|r| floats.get(4 * c + r).copied().unwrap_or(0.0))) }
    }

    pub fn columns(&self) -> [Pixel; 3] {
        self.columns
    }

    pub fn apply(&self, p: Pixel) -> Pixel {
        let [a, b, c] = &self.columns;
        mul_add(mul_add(scale(*a, p[0]), *b, p[1]), *c, p[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_tint_keeps_luma_and_takes_the_colour() {
        let out = Mix::tint([1.0, 0.5, 0.0], 1.0).apply([0.2, 0.4, 0.8, 1.0]);
        let l = luma([0.2, 0.4, 0.8]);
        assert_eq!(out, [l, l * 0.5, 0.0, 0.0]);
    }

    #[test]
    fn weighted_identity_scales_each_channel() {
        let out = Mix::identity().weighted([2.0, 1.0, 0.5]).apply([1.0, 1.0, 1.0, 1.0]);
        assert_eq!(out, [2.0, 1.0, 0.5, 0.0]);
    }
}
