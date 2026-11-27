use craft_plugin::math::exp;
use craft_plugin::prelude::Pixel;

pub const TAU: f32 = std::f32::consts::TAU;

pub fn luma(p: Pixel) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

pub fn fract(x: f32) -> f32 {
    x - x.floor()
}

pub fn hash(x: u32, y: u32, z: u32) -> f32 {
    let mut h = x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ z.wrapping_mul(83_492_791);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

pub fn blur_radius(sigma: f32) -> i32 {
    (2.5 * sigma).ceil() as i32
}

pub fn gauss(i: i32, sigma: f32) -> f32 {
    let d = i as f32;
    exp(-(d * d) / (2.0 * sigma * sigma))
}

pub fn reduction(sigma_px: f32) -> u32 {
    match sigma_px {
        s if s < 8.0 => 2,
        s if s < 32.0 => 4,
        _ => 8,
    }
}

pub fn bilinear(at: impl Fn(i32, i32) -> Pixel, x: f32, y: f32) -> Pixel {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (at(xi, yi), at(xi + 1, yi), at(xi, yi + 1), at(xi + 1, yi + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bottom = c[k] + (d[k] - c[k]) * tx;
        top + (bottom - top) * ty
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_weights_sum_to_one_for_white() {
        assert!((luma([1.0, 1.0, 1.0, 1.0]) - 1.0).abs() < 1e-6);
        assert!((luma([0.0, 1.0, 0.0, 1.0]) - 0.7152).abs() < 1e-6);
    }

    #[test]
    fn hash_is_fixed_and_in_the_unit_interval() {
        assert_eq!(hash(0, 0, 0), 0.0);
        let h = hash(12, 34, 5);
        assert_eq!(h, hash(12, 34, 5));
        assert!((0.0..1.0).contains(&h));
        assert_ne!(h, hash(12, 34, 6));
    }

    #[test]
    fn blur_radius_and_reduction_follow_sigma() {
        assert_eq!(blur_radius(2.0), 5);
        assert_eq!(blur_radius(0.3), 1);
        assert_eq!(reduction(4.0), 2);
        assert_eq!(reduction(12.0), 4);
        assert_eq!(reduction(40.0), 8);
        assert_eq!(gauss(0, 3.0), 1.0);
        assert!((gauss(2, 2.0) - (-0.5f32).exp()).abs() < 3e-7);
    }

    #[test]
    fn bilinear_interpolates_between_four_texels() {
        let at = |x: i32, y: i32| [(x + 10 * y) as f32, 0.0, 0.0, 1.0];
        assert_eq!(bilinear(at, 0.5, 0.5)[0], 5.5);
        assert_eq!(bilinear(at, 1.0, 2.0)[0], 21.0);
        assert_eq!(bilinear(at, 0.25, 0.0)[0], 0.25);
    }
}
