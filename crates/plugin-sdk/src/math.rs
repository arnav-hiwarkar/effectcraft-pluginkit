//! Fast `f32` maths for plugin hot loops.
//!
//! Every function is `#[inline(always)]`, uses only arithmetic, `floor`, `sqrt`, `abs`, `min`, `max` and bit casts
//! (single instructions on wasm and native), and has no tables and no libm call. The same code runs in a sandboxed
//! and in a native plugin, so both give the same pixels. Use these instead of `f32::exp2`, `powf`, `sin`, `atan2`
//! and friends inside per-pixel loops: on `wasm32-unknown-unknown` those are soft libm functions that are not
//! inlined and pay a timeout check on every call.
//!
//! Each function lists its valid range and its tested worst error against `std`.

use std::f32::consts::{FRAC_1_PI, FRAC_PI_2, LN_2, LOG2_E, PI, SQRT_2};

const PI_HIGH: f32 = 3.140_625;
const PI_LOW: f32 = 9.676_536e-4;
const HALF_PI_HIGH: f32 = 1.570_312_5;
const HALF_PI_LOW: f32 = 4.838_268e-4;
const LN_2_HIGH: f32 = 0.693_359_4;
const LN_2_LOW: f32 = -2.121_944_4e-4;

#[inline(always)]
fn scale_by_power_of_two(whole: f32, factor: f32) -> f32 {
    f32::from_bits(((whole as i32 + 127) as u32) << 23) * factor
}

/// `2^x` for `x` in `-126..=126`; inputs outside are clamped to it.
/// Largest relative error against `f32::exp2` over that range: 3e-7.
#[inline(always)]
pub fn exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 126.0);
    let whole = (x + 0.5).floor();
    let f = x - whole;
    let poly = 1.0 + f * (LN_2 + f * (0.240_226_5 + f * (0.055_504_11 + f * (0.009_618_129 + f * (0.001_333_355_2 + f * 0.000_154_035_3)))));
    scale_by_power_of_two(whole, poly)
}

/// `e^x` for `x` in `-87..=88`; inputs outside are clamped to it.
/// Largest relative error against `f32::exp` over that range: 5e-7.
#[inline(always)]
pub fn exp(x: f32) -> f32 {
    let x = x.clamp(-87.0, 88.0);
    let whole = (x * LOG2_E + 0.5).floor();
    let r = (x - whole * LN_2_HIGH) - whole * LN_2_LOW;
    let poly = 1.0 + r * (1.0 + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r / 5040.0))))));
    scale_by_power_of_two(whole, poly)
}

#[inline(always)]
fn log_parts(x: f32) -> (f32, f32) {
    let bits = x.max(f32::MIN_POSITIVE).to_bits();
    let mut exponent = ((bits >> 23) & 255) as i32 - 127;
    let mut mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if mantissa > SQRT_2 {
        mantissa *= 0.5;
        exponent += 1;
    }
    let t = (mantissa - 1.0) / (mantissa + 1.0);
    let s = t * t;
    let series = 2.0 * t * (1.0 + s * (1.0 / 3.0 + s * (1.0 / 5.0 + s * (1.0 / 7.0 + s * (1.0 / 9.0 + s / 11.0)))));
    (exponent as f32, series)
}

/// `log2(x)` for finite `x > 0`. Zero, negative and subnormal inputs give `-126`.
/// Largest absolute error against `f32::log2` over `1e-30..=1e30`: 2e-7 times `max(1, |log2 x|)`.
#[inline(always)]
pub fn log2(x: f32) -> f32 {
    let (exponent, series) = log_parts(x);
    exponent + series * LOG2_E
}

/// `ln(x)` for finite `x > 0`. Zero, negative and subnormal inputs give `-87.3`.
/// Largest absolute error against `f32::ln` over `1e-30..=1e30`: 3e-7 times `max(1, |ln x|)`.
#[inline(always)]
pub fn ln(x: f32) -> f32 {
    let (exponent, series) = log_parts(x);
    exponent * LN_2 + series
}

/// `x^y` for finite `x >= 0` and any `y` whose result lies in `1e-37..=1e37`. `x <= 0` is treated as the smallest
/// normal number, so `pow(0.0, y)` is `0` for any `y > 0` up to rounding at `1e-38`.
/// Largest relative error against `f64::powf` for `x` in `1e-3..=1e3`, `y` in `-8..=8`: 2e-5.
#[inline(always)]
pub fn pow(x: f32, y: f32) -> f32 {
    exp2(y * log2(x))
}

#[inline(always)]
fn sin_of_reduced(x: f32, shift_high: f32, shift_low: f32) -> f32 {
    let k = ((x + shift_high) * FRAC_1_PI + 0.5).floor();
    let y = ((x - k * PI_HIGH) + shift_high) - k * PI_LOW + shift_low;
    let s = y * y;
    let p = y * (1.0 + s * (-1.0 / 6.0 + s * (1.0 / 120.0 + s * (-1.0 / 5040.0 + s * (1.0 / 362_880.0 + s * (-1.0 / 39_916_800.0))))));
    if (k as i32) & 1 == 1 { -p } else { p }
}

/// `sin(x)` for `|x| <= 1000` (the error grows with `|x|` past that).
/// Largest absolute error against `f32::sin` over `-1000..=1000`: 2e-6.
#[inline(always)]
pub fn sin(x: f32) -> f32 {
    sin_of_reduced(x, 0.0, 0.0)
}

/// `cos(x)` for `|x| <= 1000` (the error grows with `|x|` past that).
/// Largest absolute error against `f32::cos` over `-1000..=1000`: 2e-6.
#[inline(always)]
pub fn cos(x: f32) -> f32 {
    sin_of_reduced(x, HALF_PI_HIGH, HALF_PI_LOW)
}

/// `(sin(x), cos(x))` with the range and error of [`sin`] and [`cos`].
#[inline(always)]
pub fn sin_cos(x: f32) -> (f32, f32) {
    (sin(x), cos(x))
}

/// The angle of the point `(x, y)` in `-PI..=PI`, like `f32::atan2(y, x)`. `atan2(0.0, 0.0)` is `0`.
/// Largest absolute error against `f32::atan2` over a `-20..=20` grid and around the axes: 5e-6.
#[inline(always)]
pub fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (x.abs(), y.abs());
    let (hi, lo) = (ax.max(ay), ax.min(ay));
    if hi == 0.0 {
        return 0.0;
    }
    let a = lo / hi;
    let s = a * a;
    let mut r = a * (0.999_977_26 + s * (-0.332_623_47 + s * (0.193_543_46 + s * (-0.116_432_87 + s * (0.052_653_32 - 0.011_721_2 * s)))));
    if ay > ax {
        r = FRAC_PI_2 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}

/// `sqrt(x * x + y * y)` for `|x|, |y| <= 1e18`. Largest relative error against `f32::hypot`: 1.5e-7.
#[inline(always)]
pub fn hypot(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// The cubic Hermite step from 0 at `edge0` to 1 at `edge1`, clamped outside; `edge0 != edge1`.
#[inline(always)]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = saturate((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

/// `a + (b - a) * t`.
#[inline(always)]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `x` clamped to `0..=1`; NaN stays NaN.
#[inline(always)]
pub fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// The sRGB transfer function's decode: an encoded value to linear light. Negative values mirror around zero and
/// values above 1 continue the curve. Largest absolute error against the `f64` formula over `-2..=2`: 3e-6.
#[inline(always)]
pub fn srgb_to_linear(encoded: f32) -> f32 {
    let a = encoded.abs();
    let linear = if a <= 0.040_45 { a / 12.92 } else { pow((a + 0.055) / 1.055, 2.4) };
    linear.copysign(encoded)
}

/// The sRGB transfer function's encode: linear light to an encoded value. Negative values mirror around zero and
/// values above 1 continue the curve. Largest absolute error against the `f64` formula over `-2..=2`: 3e-6.
#[inline(always)]
pub fn linear_to_srgb(linear: f32) -> f32 {
    let a = linear.abs();
    let encoded = if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * pow(a, 1.0 / 2.4) - 0.055 };
    encoded.copysign(linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worst(inputs: impl Iterator<Item = f32>, error: impl Fn(f32) -> f64) -> f64 {
        inputs.map(error).fold(0.0, f64::max)
    }

    fn sweep(from: f32, to: f32, steps: u32) -> impl Iterator<Item = f32> {
        (0..=steps).map(move |i| from + (to - from) * i as f32 / steps as f32)
    }

    fn geometric(from: f32, to: f32, steps: u32) -> impl Iterator<Item = f32> {
        let ratio = (to / from).ln();
        (0..=steps).map(move |i| from * (ratio * i as f32 / steps as f32).exp())
    }

    fn relative(got: f32, want: f64) -> f64 {
        ((f64::from(got) - want) / want).abs()
    }

    #[test]
    fn exp2_is_within_3e_7_relative_over_its_range() {
        let error = worst(sweep(-126.0, 126.0, 200_000), |x| relative(exp2(x), f64::from(x).exp2()));
        assert!(error < 3.0e-7, "{error}");
        assert_eq!(exp2(0.0), 1.0);
        assert_eq!(exp2(10.0), 1024.0);
        assert_eq!(exp2(-1.0), 0.5);
        assert_eq!(exp2(1000.0), exp2(126.0));
        assert_eq!(exp2(-1000.0), exp2(-126.0));
    }

    #[test]
    fn exp_is_within_5e_7_relative_over_its_range() {
        let error = worst(sweep(-87.0, 88.0, 200_000), |x| relative(exp(x), f64::from(x).exp()));
        assert!(error < 5.0e-7, "{error}");
        assert_eq!(exp(0.0), 1.0);
        assert!((exp(1.0) - std::f32::consts::E).abs() < 3e-7);
        assert_eq!(exp(500.0), exp(88.0));
        assert!(exp(-500.0) > 0.0);
    }

    #[test]
    fn log2_is_within_2e_7_of_the_exact_value() {
        let error = worst(geometric(1e-30, 1e30, 200_000), |x| {
            let want = f64::from(x).log2();
            (f64::from(log2(x)) - want).abs() / want.abs().max(1.0)
        });
        assert!(error < 2.0e-7, "{error}");
        assert_eq!(log2(1.0), 0.0);
        assert_eq!(log2(8.0), 3.0);
        assert_eq!(log2(0.25), -2.0);
        assert_eq!(log2(0.0), -126.0);
        assert_eq!(log2(-5.0), -126.0);
    }

    #[test]
    fn ln_is_within_3e_7_of_the_exact_value() {
        let error = worst(geometric(1e-30, 1e30, 200_000), |x| {
            let want = f64::from(x).ln();
            (f64::from(ln(x)) - want).abs() / want.abs().max(1.0)
        });
        assert!(error < 3.0e-7, "{error}");
        assert_eq!(ln(1.0), 0.0);
        assert!((ln(std::f32::consts::E) - 1.0).abs() < 3e-7);
    }

    #[test]
    fn pow_is_within_2e_5_relative_for_ordinary_arguments() {
        let mut error = 0.0f64;
        for y in sweep(-8.0, 8.0, 64) {
            error = error.max(worst(geometric(1e-3, 1e3, 2000), |x| {
                let want = f64::from(x).powf(f64::from(y));
                if !(1e-37..=1e37).contains(&want) { 0.0 } else { relative(pow(x, y), want) }
            }));
        }
        assert!(error < 2.0e-5, "{error}");
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(5.0, 0.0), 1.0);
        assert!(pow(0.0, 2.0) < 1e-30);
    }

    #[test]
    fn sin_and_cos_are_within_2e_6_over_1000_radians() {
        let sin_error = worst(sweep(-1000.0, 1000.0, 400_000), |x| (f64::from(sin(x)) - f64::from(x).sin()).abs());
        let cos_error = worst(sweep(-1000.0, 1000.0, 400_000), |x| (f64::from(cos(x)) - f64::from(x).cos()).abs());
        assert!(sin_error < 2.0e-6, "{sin_error}");
        assert!(cos_error < 2.0e-6, "{cos_error}");
        assert_eq!(sin(0.0), 0.0);
        assert!((cos(0.0) - 1.0).abs() < 2e-7);
        assert_eq!(sin_cos(1.0), (sin(1.0), cos(1.0)));
    }

    #[test]
    fn sin_and_cos_are_within_3e_7_over_a_few_turns() {
        let sin_error = worst(sweep(-20.0, 20.0, 200_000), |x| (f64::from(sin(x)) - f64::from(x).sin()).abs());
        let cos_error = worst(sweep(-20.0, 20.0, 200_000), |x| (f64::from(cos(x)) - f64::from(x).cos()).abs());
        assert!(sin_error < 3.0e-7, "{sin_error}");
        assert!(cos_error < 3.0e-7, "{cos_error}");
    }

    #[test]
    fn atan2_is_within_5e_6_in_every_quadrant() {
        let mut error = 0.0f64;
        for y in sweep(-20.0, 20.0, 160) {
            error = error.max(worst(sweep(-20.0, 20.0, 160), |x| (f64::from(atan2(y, x)) - f64::from(y).atan2(f64::from(x))).abs()));
        }
        for scale in [1e-20f32, 1e-3, 1.0, 1e5] {
            for (y, x) in [(0.0, 1.0), (0.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (1.0, 1.0), (-1.0, -1.0), (1e-6, -1.0), (-1e-6, -1.0)] {
                let (y, x) = (y * scale, x * scale);
                error = error.max((f64::from(atan2(y, x)) - f64::from(y).atan2(f64::from(x))).abs());
            }
        }
        assert!(error < 5.0e-6, "{error}");
        assert_eq!(atan2(0.0, 0.0), 0.0);
        assert_eq!(atan2(0.0, 1.0), 0.0);
        assert_eq!(atan2(1.0, 0.0), FRAC_PI_2);
    }

    #[test]
    fn hypot_is_within_1_5e_7_relative() {
        let mut error = 0.0f64;
        for y in sweep(-300.0, 300.0, 120) {
            error = error.max(worst(sweep(-300.0, 300.0, 120), |x| {
                let want = f64::from(x).hypot(f64::from(y));
                if want == 0.0 { 0.0 } else { relative(hypot(x, y), want) }
            }));
        }
        assert!(error < 1.5e-7, "{error}");
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
    }

    #[test]
    fn smoothstep_lerp_and_saturate_have_their_textbook_values() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.25), 0.15625);
        assert_eq!(smoothstep(2.0, 4.0, 3.0), 0.5);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert_eq!(lerp(2.0, 6.0, 0.25), 3.0);
        assert_eq!(lerp(2.0, 6.0, 1.5), 8.0);
        assert_eq!(saturate(-0.5), 0.0);
        assert_eq!(saturate(0.3), 0.3);
        assert_eq!(saturate(7.0), 1.0);
        assert!(saturate(f32::NAN).is_nan());
    }

    #[test]
    fn srgb_transfer_is_within_3e_6_and_round_trips() {
        let decode = |v: f64| if v.abs() <= 0.04045 { v / 12.92 } else { ((v.abs() + 0.055) / 1.055).powf(2.4).copysign(v) };
        let encode = |v: f64| if v.abs() <= 0.0031308 { v * 12.92 } else { (1.055 * v.abs().powf(1.0 / 2.4) - 0.055).copysign(v) };
        let decoded = worst(sweep(-2.0, 2.0, 100_000), |v| (f64::from(srgb_to_linear(v)) - decode(f64::from(v))).abs());
        let encoded = worst(sweep(-2.0, 2.0, 100_000), |v| (f64::from(linear_to_srgb(v)) - encode(f64::from(v))).abs());
        assert!(decoded < 3.0e-6, "{decoded}");
        assert!(encoded < 3.0e-6, "{encoded}");
        let trip = worst(sweep(0.0, 1.0, 1000), |v| f64::from((linear_to_srgb(srgb_to_linear(v)) - v).abs()));
        assert!(trip < 5.0e-6, "{trip}");
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 3e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 3e-6);
    }
}
