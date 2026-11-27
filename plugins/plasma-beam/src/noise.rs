use std::f32::consts::FRAC_1_SQRT_2;

const GRADIENTS: [[f32; 2]; 8] = [
    [1.0, 0.0],
    [-1.0, 0.0],
    [0.0, 1.0],
    [0.0, -1.0],
    [FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    [-FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    [FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
    [-FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
];

pub fn mix(seed: u32, value: u32) -> u32 {
    let mut h = seed ^ value.wrapping_mul(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn lattice(x: i32, y: i32, seed: u32) -> [f32; 2] {
    GRADIENTS[(mix(mix(seed, x as u32), y as u32) & 7) as usize]
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

pub fn gradient(x: f32, y: f32, period: Option<i32>, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let wrap = |i: i32| period.map_or(i, |p| i.rem_euclid(p.max(1)));
    let corner = |dx: i32, dy: i32| {
        let g = lattice(wrap(ix + dx), iy + dy, seed);
        g[0] * (fx - dx as f32) + g[1] * (fy - dy as f32)
    };
    let (u, v) = (fade(fx), fade(fy));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    (top + (bottom - top) * v) * std::f32::consts::SQRT_2
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractal {
    pub octaves: u32,
    pub roughness: f32,
}

impl Fractal {
    pub fn sample(&self, x: f32, y: f32, period: Option<i32>, seed: u32) -> f32 {
        let (mut sum, mut norm, mut amplitude, mut frequency) = (0.0, 0.0, 1.0, 1.0);
        for octave in 0..self.octaves.max(1) {
            let lane = octave as f32;
            let period = period.map(|p| p << octave);
            let value = gradient(x * frequency, y * (1.0 + 0.6 * lane) + 31.7 * lane, period, mix(seed, octave));
            sum += value * amplitude;
            norm += amplitude * amplitude;
            amplitude *= self.roughness;
            frequency *= 2.0;
        }
        sum / norm.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRACTAL: Fractal = Fractal { octaves: 4, roughness: 0.55 };

    #[test]
    fn noise_is_deterministic_for_a_seed_and_changes_with_it() {
        let a = FRACTAL.sample(3.3, 1.7, None, 42);
        assert_eq!(a, FRACTAL.sample(3.3, 1.7, None, 42));
        assert_ne!(a, FRACTAL.sample(3.3, 1.7, None, 43));
    }

    #[test]
    fn noise_is_continuous_in_time() {
        let step = 1.0 / 30.0 * 0.5;
        let worst = (0..300)
            .map(|frame| {
                let t = frame as f32 * step;
                (FRACTAL.sample(2.25, t + step, None, 7) - FRACTAL.sample(2.25, t, None, 7)).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(worst < 0.12, "largest change between frames {worst}");
    }

    #[test]
    fn noise_is_zero_on_lattice_points_and_bounded() {
        assert_eq!(gradient(4.0, -2.0, None, 9), 0.0);
        let peak = (0..2000).map(|i| gradient(i as f32 * 0.137, i as f32 * 0.071, None, 3).abs()).fold(0.0, f32::max);
        assert!(peak <= 1.0 && peak > 0.5);
    }

    #[test]
    fn periodic_noise_wraps_seamlessly() {
        let period = 5;
        for y in [0.2, 1.4, 7.9] {
            assert!((FRACTAL.sample(0.0, y, Some(period), 11) - FRACTAL.sample(5.0, y, Some(period), 11)).abs() < 1e-5);
        }
    }
}
