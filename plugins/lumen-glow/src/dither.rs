use craft_plugin::Pixel;

use crate::settings::Dither;

const LEVELS: f32 = 255.0;
const STEP: f32 = 1.0 / LEVELS;
const HALF_RANGE: f32 = 1.0 / 65536.0;

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

pub fn triangular_noise(x: i64, y: i64, seed: u32) -> f32 {
    let h = hash(x as u32, y as u32, seed);
    ((h & 0xffff) as f32 - (h >> 16) as f32) * HALF_RANGE
}

fn quantisable_room(v: f32) -> f32 {
    (v * LEVELS).clamp(0.0, 1.0) * ((1.0 - v) * LEVELS).clamp(0.0, 1.0)
}

impl Dither {
    pub fn apply(&self, p: Pixel, x: usize, y: usize) -> Pixel {
        let room = [p[0], p[1], p[2]].map(quantisable_room);
        if room == [0.0; 3] {
            return p;
        }
        let n = triangular_noise(
            x as i64 - self.origin[0],
            y as i64 - self.origin[1],
            self.seed,
        ) * STEP;
        [
            p[0] + n * room[0],
            p[1] + n * room[1],
            p[2] + n * room[2],
            p[3],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_triangular_and_centred() {
        let samples: Vec<f32> = (0..256)
            .flat_map(|y| (0..256).map(move |x| triangular_noise(x, y, 7)))
            .collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let inner = samples.iter().filter(|n| n.abs() < 0.5).count() as f32 / samples.len() as f32;
        assert!(
            mean.abs() < 0.01 && (inner - 0.75).abs() < 0.01,
            "mean {mean} inner {inner}"
        );
    }

    #[test]
    fn black_and_white_are_left_exact() {
        let d = Dither {
            seed: 3,
            origin: [0, 0],
        };
        assert_eq!(d.apply([0.0, 1.0, 2.0, 0.5], 5, 9), [0.0, 1.0, 2.0, 0.5]);
    }
}
