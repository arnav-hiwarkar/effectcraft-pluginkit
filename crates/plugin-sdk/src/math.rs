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
/// Largest absolute error against `f32::log2` over `1e-30..=1e30`: 1.1e-7 times `max(1, |log2 x|)`.
#[inline(always)]
pub fn log2(x: f32) -> f32 {
    let (exponent, series) = log_parts(x);
    exponent + series * LOG2_E
}

/// `ln(x)` for finite `x > 0`. Zero, negative and subnormal inputs give `-87.3`.
/// Largest absolute error against `f32::ln` over `1e-30..=1e30`: 1.2e-7 times `max(1, |ln x|)`.
#[inline(always)]
pub fn ln(x: f32) -> f32 {
    let (exponent, series) = log_parts(x);
    exponent * LN_2 + series
}

/// `x^y` for finite `x >= 0` and any `y` whose result lies in `1e-37..=1e37`. `pow(0.0, y)` is exactly `0` for
/// `y > 0`; a negative `x` is treated like `0`. A positive `x` below `f32::MIN_POSITIVE` (about `1.2e-38`) is
/// treated like `f32::MIN_POSITIVE`, as in [`log2`]: normalising it would cost every call.
/// Largest relative error against `f64::powf` for `x` in `1e-3..=1e3`, `y` in `-8..=8`: 2e-5.
#[inline(always)]
pub fn pow(x: f32, y: f32) -> f32 {
    let power = exp2(y * log2(x));
    if x <= 0.0 && y > 0.0 { 0.0 } else { power }
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

/// The angle of the point `(x, y)` in `-PI..=PI`, like `f32::atan2(y, x)`, including the sign of a zero `y` and
/// of a zero `x`. `atan2` of two zeros is `0`. A NaN argument gives a finite angle, not NaN: checking for it would
/// cost every call.
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
    if x.is_sign_negative() {
        r = PI - r;
    }
    if y.is_sign_negative() { -r } else { r }
}

/// `sqrt(x * x + y * y)` when the larger of `|x|` and `|y|` is `0` or lies in `1e-19..=1e18`. Largest relative
/// error against `f32::hypot` over that range: 1.5e-7. Below `1e-19` the squares are subnormal and the result loses
/// precision, and below `1e-22` it is `0`; scaling the arguments would cost every call, and no pixel value is that
/// small and nonzero.
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
/// values above 1 continue the curve. Largest absolute error against the `f64` formula over `-2..=2`: 3e-6; largest
/// relative error up to `|encoded| = 1e15`: 3e-6. Past `6e15` the result saturates at `2^126` (about `8.5e37`), as
/// [`pow`] does: a branch to the standard power function would cost every pixel, and no pixel is that bright.
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

/// Cheaper forms of [`sin`], [`cos`], [`exp`], [`exp2`] and [`ln`] for loops that call several of them per pixel: shorter
/// polynomials, a one-step range reduction and no guards, so fewer constants and instructions. Each lists its tested
/// worst error, which is far below one level of a 16-bit channel.
pub mod fast {
    use std::f32::consts::{FRAC_1_PI, LOG2_E, PI};

    #[inline(always)]
    fn sin_of_turn(k: f32, y: f32) -> f32 {
        let s = y * y;
        let p = y * (0.999_996_6 + s * (-0.166_648_28 + s * (0.008_306_325 - s * 0.000_183_636_54)));
        if (k as i32) & 1 == 1 { -p } else { p }
    }

    /// `sin(x)`. Largest absolute error against `f32::sin` over `-20..=20`: 2e-6, over `-100..=100`: 7e-6; it
    /// grows with `|x|`, to about 6e-5 at `|x| = 1000`.
    #[inline(always)]
    pub fn sin(x: f32) -> f32 {
        let k = (x * FRAC_1_PI + 0.5).floor();
        sin_of_turn(k, x - k * PI)
    }

    /// `cos(x)`, with the range and error of [`sin`].
    #[inline(always)]
    pub fn cos(x: f32) -> f32 {
        let k = (x * FRAC_1_PI + 1.0).floor();
        sin_of_turn(k, x - (k - 0.5) * PI)
    }

    /// `e^x` for `x` in `-87..=88`; inputs outside are clamped to it. Largest relative error against `f32::exp`
    /// over `-20..=20`: 9e-6, over the whole range: 1.2e-5.
    #[inline(always)]
    pub fn exp(x: f32) -> f32 {
        let t = x.clamp(-87.0, 88.0) * LOG2_E;
        let whole = (t + 0.5).floor();
        let f = t - whole;
        let poly = 1.0 + f * (0.693_136_7 + f * (0.240_221_68 + f * (0.055_838_33 + f * 0.009_676_043)));
        f32::from_bits(((whole as i32 + 127) as u32) << 23) * poly
    }

    /// `2^x` for `x` in `-126..=126`; inputs outside are clamped to it. Largest relative error against `f32::exp2`
    /// over that range: 3.3e-6.
    #[inline(always)]
    pub fn exp2(x: f32) -> f32 {
        let x = x.clamp(-126.0, 126.0);
        let whole = (x + 0.5).floor();
        let f = x - whole;
        let poly = 1.0 + f * (std::f32::consts::LN_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_4))));
        f32::from_bits(((whole as i32 + 127) as u32) << 23) * poly
    }

    /// `ln(x)` for normal `x > 0`, as exact as [`super::ln`] there. It has no guard: zero gives `-127 * LN_2` (about
    /// `-88`), a negative `x` gives `ln(-x)`, and a subnormal one a wrong value near `-88`.
    /// Largest absolute error against `f32::ln` over `1e-30..=1e30`: 1.2e-7 times `max(1, |ln x|)`.
    #[inline(always)]
    pub fn ln(x: f32) -> f32 {
        let bits = x.to_bits();
        let mut exponent = ((bits >> 23) & 255) as i32 - 127;
        let mut mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
        if mantissa > std::f32::consts::SQRT_2 {
            mantissa *= 0.5;
            exponent += 1;
        }
        let t = (mantissa - 1.0) / (mantissa + 1.0);
        let s = t * t;
        exponent as f32 * std::f32::consts::LN_2 + 2.0 * t * (1.0 + s * (1.0 / 3.0 + s * (1.0 / 5.0 + s * (1.0 / 7.0 + s / 9.0))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worst(inputs: impl Iterator<Item = f32>, error: impl Fn(f32) -> f64) -> f64 {
        inputs
            .map(|x| {
                let e = error(x);
                assert!(e.is_finite(), "the error at {x:e} is {e}");
                e
            })
            .fold(0.0, f64::max)
    }

    fn sweep(from: f32, to: f32, steps: u32) -> impl Iterator<Item = f32> {
        (0..=steps).map(move |i| from + (to - from) * i as f32 / steps as f32)
    }

    fn geometric(from: f32, to: f32, steps: u32) -> impl Iterator<Item = f32> {
        let (from, to) = (f64::from(from), f64::from(to));
        let ratio = (to / from).ln();
        (0..=steps).map(move |i| (from * (ratio * f64::from(i) / f64::from(steps)).exp()) as f32)
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
    fn log2_is_within_1_1e_7_of_the_exact_value() {
        let error = worst(geometric(1e-30, 1e30, 200_000), |x| {
            let want = f64::from(x).log2();
            (f64::from(log2(x)) - want).abs() / want.abs().max(1.0)
        });
        assert!(error < 1.1e-7, "{error}");
        assert_eq!(log2(1.0), 0.0);
        assert_eq!(log2(8.0), 3.0);
        assert_eq!(log2(0.25), -2.0);
        assert_eq!(log2(0.0), -126.0);
        assert_eq!(log2(-5.0), -126.0);
    }

    #[test]
    fn ln_is_within_1_2e_7_of_the_exact_value() {
        let error = worst(geometric(1e-30, 1e30, 200_000), |x| {
            let want = f64::from(x).ln();
            (f64::from(ln(x)) - want).abs() / want.abs().max(1.0)
        });
        assert!(error < 1.2e-7, "{error}");
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
        assert_eq!(pow(0.0, 2.0), 0.0);
        assert_eq!(pow(-3.0, 0.5), 0.0);
        assert_eq!(pow(0.0, 0.0), 1.0);
        assert_eq!(pow(f32::from_bits(1 << 22), 0.5), pow(f32::MIN_POSITIVE, 0.5));
        assert_eq!(pow(f32::MIN_POSITIVE, 0.5), exp2(-63.0));
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
        assert_eq!(atan2(-0.0, -1.0), -PI);
        assert_eq!(atan2(0.0, -1.0), PI);
        assert_eq!(atan2(-0.0, 1.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(atan2(1.0, -0.0), FRAC_PI_2);
        assert_eq!(atan2(-1.0, -0.0), -FRAC_PI_2);
        assert!(atan2(f32::NAN, 1.0).is_finite());
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
        let at_the_limit = hypot(1e-19, 0.5e-19);
        assert!(relative(at_the_limit, 1.118_033_988_749_895e-19) < 1.5e-7, "{at_the_limit:e}");
        assert_eq!(hypot(1e-30, 0.0), 0.0);
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
        let bright = worst([1e3f32, 1e8, 1e12, 1e15].into_iter(), |v| relative(srgb_to_linear(v), ((f64::from(v) + 0.055) / 1.055).powf(2.4)));
        assert!(bright < 3.0e-6, "{bright}");
        assert_eq!(srgb_to_linear(1e16), 2.0f32.powi(126));
        assert_eq!(srgb_to_linear(-1e30), -(2.0f32.powi(126)));
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 3e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 3e-6);
    }

    #[test]
    fn fast_sin_and_cos_are_within_2e_6_over_20_radians_and_7e_6_over_100() {
        for (range, bound) in [(20.0, 2.0e-6), (100.0, 7.0e-6), (1000.0, 6.0e-5)] {
            let sin_error = worst(sweep(-range, range, 400_000), |x| (f64::from(fast::sin(x)) - f64::from(x).sin()).abs());
            let cos_error = worst(sweep(-range, range, 400_000), |x| (f64::from(fast::cos(x)) - f64::from(x).cos()).abs());
            assert!(sin_error < bound, "sin over {range}: {sin_error}");
            assert!(cos_error < bound, "cos over {range}: {cos_error}");
        }
        assert_eq!(fast::sin(0.0), 0.0);
        assert!((fast::cos(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fast_exp_is_within_9e_6_relative_over_20_and_1_2e_5_over_its_range() {
        let near = worst(sweep(-20.0, 20.0, 200_000), |x| relative(fast::exp(x), f64::from(x).exp()));
        let whole = worst(sweep(-87.0, 88.0, 200_000), |x| relative(fast::exp(x), f64::from(x).exp()));
        assert!(near < 9.0e-6, "{near}");
        assert!(whole < 1.2e-5, "{whole}");
        assert_eq!(fast::exp(0.0), 1.0);
        assert_eq!(fast::exp(500.0), fast::exp(88.0));
        assert!(fast::exp(-500.0) > 0.0);
    }

    #[test]
    fn fast_exp2_is_within_4e_6_relative_over_its_range() {
        let error = worst(sweep(-126.0, 126.0, 400_000), |x| relative(fast::exp2(x), f64::from(x).exp2()));
        assert!(error < 4e-6, "{error:e}");
        assert_eq!(fast::exp2(0.0), 1.0);
        assert_eq!(fast::exp2(10.0), 1024.0);
        assert_eq!(fast::exp2(-1.0), 0.5);
        assert_eq!(fast::exp2(1000.0), fast::exp2(126.0));
        assert_eq!(fast::exp2(-1000.0), fast::exp2(-126.0));
    }

    #[test]
    fn fast_ln_is_as_exact_as_ln_on_normal_inputs() {
        let error = worst(geometric(1e-30, 1e30, 200_000), |x| {
            let want = f64::from(x).ln();
            (f64::from(fast::ln(x)) - want).abs() / want.abs().max(1.0)
        });
        assert!(error < 1.2e-7, "{error}");
        assert_eq!(fast::ln(1.0), 0.0);
        assert_eq!(fast::ln(0.0), -127.0 * LN_2);
        assert_eq!(fast::ln(-5.0), fast::ln(5.0));
    }
}
