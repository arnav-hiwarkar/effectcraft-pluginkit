use std::sync::OnceLock;

use craft_plugin::math::{linear_to_srgb, srgb_to_linear};
use craft_plugin::{BlendMode, HostKernel, Measure, Pixel};

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
const TABLE_STEPS: usize = 4096;
const TABLE_BRIGHT_END: f32 = 17.0;
const DITHER_LEVELS: f32 = 255.0;
const NOISE_SCALE: f32 = 1.0 / 65536.0 / DITHER_LEVELS;

pub(crate) fn row(kernel: &HostKernel, src: &[Pixel], top: &[Pixel], y: usize, out: &mut [Pixel]) {
    let pairs = out.iter_mut().zip(src);
    match kernel {
        HostKernel::Blend { mode, amount } => pairs.zip(top).for_each(|((o, b), t)| *o = blend(*mode, *amount, *b, *t)),
        HostKernel::Curves { table } => pairs.for_each(|(o, p)| *o = curves(table, *p)),
        HostKernel::Matrix { rows } => pairs.for_each(|(o, p)| *o = rows.map(|row| row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3] * p[3])),
        HostKernel::ToLinear => {
            let curve = &transfers().decode;
            pairs.for_each(|(o, p)| *o = [curve.eval(p[0]), curve.eval(p[1]), curve.eval(p[2]), p[3]]);
        }
        HostKernel::ToSrgb => {
            let curve = &transfers().encode;
            pairs.for_each(|(o, p)| *o = [curve.eval(p[0]), curve.eval(p[1]), curve.eval(p[2]), p[3]]);
        }
        HostKernel::BrightnessCurve { measure, curve, range } => {
            let steps_per_unit = (curve.len() - 1) as f32 / range;
            pairs.for_each(|(o, p)| *o = brightness_curve(*measure, curve, steps_per_unit, *p));
        }
        HostKernel::Dither { seed, origin } => {
            let row_y = (y as i64).wrapping_sub(origin[1]) as u32;
            pairs.enumerate().for_each(|(x, (o, p))| *o = dither(*p, (x as i64).wrapping_sub(origin[0]) as u32, row_y, *seed));
        }
        HostKernel::LightAlpha => pairs.for_each(|(o, p)| *o = [p[0], p[1], p[2], p[3].max(p[0].max(p[1]).max(p[2]).clamp(0.0, 1.0))]),
        HostKernel::Blur { .. } | HostKernel::Resample { .. } | HostKernel::Convolve { .. } | HostKernel::Warp { .. } => {}
    }
}

pub(crate) fn blend_channel(mode: BlendMode, b: f32, t: f32) -> f32 {
    match mode {
        BlendMode::Over | BlendMode::Add => b + t,
        BlendMode::Subtract => b - t,
        BlendMode::Multiply => b * t,
        BlendMode::Screen => b + t - b * t,
        BlendMode::Overlay if b <= 0.5 => 2.0 * b * t,
        BlendMode::Overlay => 1.0 - 2.0 * (1.0 - b) * (1.0 - t),
        BlendMode::Darken => b.min(t),
        BlendMode::Lighten => b.max(t),
        BlendMode::Difference => (b - t).abs(),
    }
}

fn blend(mode: BlendMode, amount: f32, base: Pixel, top: Pixel) -> Pixel {
    let keep = if mode == BlendMode::Over { 1.0 - top[3] } else { 1.0 };
    std::array::from_fn(|c| {
        let blended = if mode == BlendMode::Over { top[c] + base[c] * keep } else { blend_channel(mode, base[c], top[c]) };
        base[c] + amount * (blended - base[c])
    })
}

fn curve(table: &[Pixel], channel: usize, x: f32) -> f32 {
    let last = table.len() - 1;
    let t = x.clamp(0.0, 1.0) * last as f32;
    let i = (t as usize).min(last - 1);
    let (a, b) = (table[i][channel], table[i + 1][channel]);
    a + (b - a) * (t - i as f32)
}

fn curves(table: &[Pixel], p: Pixel) -> Pixel {
    let alpha = curve(table, 3, p[3]);
    let straight = |c: usize| if p[3] > 0.0 { p[c] / p[3] } else { 0.0 };
    let colour = |c: usize| curve(table, c, straight(c)) * alpha;
    [colour(0), colour(1), colour(2), alpha]
}

pub(crate) fn measure(measure: Measure, p: Pixel) -> f32 {
    match measure {
        Measure::MaxChannel => p[0].max(p[1]).max(p[2]),
        Measure::Luma => LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2],
    }
}

pub(crate) fn lookup(curve: &[f32], steps_per_unit: f32, m: f32) -> f32 {
    let last = curve.len() - 1;
    let t = m * steps_per_unit;
    let i = (t as usize).min(last - 1);
    let (a, b) = (curve[i], curve[i + 1]);
    a + (b - a) * (t - i as f32)
}

fn brightness_curve(kind: Measure, curve: &[f32], steps_per_unit: f32, p: Pixel) -> Pixel {
    let m = measure(kind, p);
    if m <= 0.0 {
        return [0.0, 0.0, 0.0, p[3]];
    }
    let gain = lookup(curve, steps_per_unit, m) / m;
    [p[0] * gain, p[1] * gain, p[2] * gain, p[3]]
}

pub(crate) fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn room(v: f32) -> f32 {
    (v * DITHER_LEVELS).clamp(0.0, 1.0) * ((1.0 - v) * DITHER_LEVELS).clamp(0.0, 1.0)
}

fn dither(p: Pixel, x: u32, y: u32, seed: u32) -> Pixel {
    let room = [room(p[0]), room(p[1]), room(p[2])];
    if room == [0.0; 3] {
        return p;
    }
    let h = hash(x, y, seed);
    let n = ((h & 0xffff) as f32 - (h >> 16) as f32) * NOISE_SCALE;
    [p[0] + n * room[0], p[1] + n * room[1], p[2] + n * room[2], p[3]]
}

struct Table {
    start: f32,
    steps_per_unit: f32,
    values: Vec<f32>,
}

impl Table {
    fn sampled(exact: fn(f32) -> f32, start: f32, end: f32) -> Table {
        let steps_per_unit = TABLE_STEPS as f32 / (end - start);
        Table { start, steps_per_unit, values: (0..=TABLE_STEPS).map(|i| exact(start + i as f32 / steps_per_unit)).collect() }
    }

    fn lookup(&self, x: f32) -> f32 {
        let t = (x - self.start) * self.steps_per_unit;
        let i = (t as usize).min(TABLE_STEPS - 1);
        let (a, b) = (self.values[i], self.values[i + 1]);
        a + (b - a) * (t - i as f32)
    }
}

struct Transfer {
    exact: fn(f32) -> f32,
    unit: Table,
    bright: Table,
}

impl Transfer {
    fn new(exact: fn(f32) -> f32) -> Transfer {
        Transfer { exact, unit: Table::sampled(exact, 0.0, 1.0), bright: Table::sampled(exact, 1.0, TABLE_BRIGHT_END) }
    }

    fn eval(&self, x: f32) -> f32 {
        if (0.0..1.0).contains(&x) {
            self.unit.lookup(x)
        } else if (1.0..TABLE_BRIGHT_END).contains(&x) {
            self.bright.lookup(x)
        } else {
            (self.exact)(x)
        }
    }
}

struct Transfers {
    decode: Transfer,
    encode: Transfer,
}

fn transfers() -> &'static Transfers {
    static TRANSFERS: OnceLock<Transfers> = OnceLock::new();
    TRANSFERS.get_or_init(|| Transfers { decode: Transfer::new(srgb_to_linear), encode: Transfer::new(linear_to_srgb) })
}
