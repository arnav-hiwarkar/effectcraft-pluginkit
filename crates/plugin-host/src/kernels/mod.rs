mod fuse;
mod gpu;
mod pointwise;
#[cfg(test)]
mod tests;

use craft_plugin::{BlurShape, Filter, HostKernel, Pixel, Taps};
use rayon::prelude::*;

pub(crate) use fuse::run as run_fused;
pub use fuse::{Fused, Op, Ref, Step, steps};
pub use gpu::{GpuStep, Lowered, StepImage, lower};

const CLEAR: Pixel = [0.0; 4];
const MIN_TASK_PIXELS: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Image<'a> {
    pub size: [u32; 2],
    pub pixels: &'a [Pixel],
}

impl Image<'_> {
    fn width(&self) -> usize {
        self.size[0] as usize
    }

    fn row(&self, y: usize) -> &[Pixel] {
        let width = self.width();
        &self.pixels[y * width..(y + 1) * width]
    }

    fn at(&self, x: isize, y: isize) -> Pixel {
        let [w, h] = self.size.map(|side| side as isize);
        if x < 0 || y < 0 || x >= w || y >= h { CLEAR } else { self.pixels[y as usize * w as usize + x as usize] }
    }
}

pub(crate) fn blur_taps(shape: BlurShape, radius: f32) -> Vec<f32> {
    let reach = radius.ceil().max(0.0) as isize;
    if reach == 0 {
        return vec![1.0];
    }
    let weight = |i: isize| match shape {
        BlurShape::Gaussian if i == 0 => 1.0,
        BlurShape::Gaussian => {
            let sigma = radius / 3.0;
            (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()
        }
        BlurShape::Box => (radius + 1.0 - i.unsigned_abs() as f32).clamp(0.0, 1.0),
    };
    let raw: Vec<f32> = (-reach..=reach).map(weight).collect();
    let total: f32 = raw.iter().sum();
    raw.iter().map(|w| w / total).collect()
}

pub(crate) fn resample_scale(scale: Option<[f32; 2]>, src: [u32; 2], out: [u32; 2]) -> [f32; 2] {
    scale.unwrap_or([0, 1].map(|axis| src[axis] as f32 / out[axis].max(1) as f32))
}

pub(crate) fn run(kernel: &HostKernel, src: Image, aux: Option<Image>, size: [u32; 2], out: &mut [Pixel]) {
    let top = aux.unwrap_or(Image { size, pixels: &[] });
    let width = size[0] as usize;
    match kernel {
        HostKernel::Blur { shape, radius } => {
            let [x, y] = radius.map(|r| blur_taps(*shape, r));
            convolve_separable(src, &x, &y, width, out);
        }
        HostKernel::Convolve { taps: Taps::Separable { x, y } } => convolve_separable(src, x, y, width, out),
        HostKernel::Convolve { taps: Taps::Full { width: side, weights } } => convolve_full(src, *side as usize, weights, width, out),
        HostKernel::Resample { filter, scale } => resample(src, *filter, resample_scale(*scale, src.size, size), size, out),
        HostKernel::Warp { amount } => each_pixel(src, out, width, |x, y, _| {
            let d = top.at(x as isize, y as isize);
            bilinear(src, x as f32 + amount[0] * d[0], y as f32 + amount[1] * d[1])
        }),
        HostKernel::Blend { .. }
        | HostKernel::Curves { .. }
        | HostKernel::Matrix { .. }
        | HostKernel::ToLinear
        | HostKernel::ToSrgb
        | HostKernel::BrightnessCurve { .. }
        | HostKernel::Dither { .. }
        | HostKernel::LightAlpha => {
            let clear = vec![CLEAR; width];
            each_row(out, width, || (), |_, y, row| pointwise::row(kernel, src.row(y), top.pixels.get(y * width..(y + 1) * width).unwrap_or(&clear), y, row));
        }
    }
}

fn rows_per_task(width: usize) -> usize {
    (MIN_TASK_PIXELS / width.max(1)).max(1)
}

fn each_row<S>(out: &mut [Pixel], width: usize, init: impl Fn() -> S + Sync + Send, f: impl Fn(&mut S, usize, &mut [Pixel]) + Sync + Send) {
    if width == 0 {
        return;
    }
    out.par_chunks_mut(width).enumerate().with_min_len(rows_per_task(width)).for_each_init(init, |state, (y, row)| f(state, y, row));
}

fn each_pixel(src: Image, out: &mut [Pixel], width: usize, f: impl Fn(usize, usize, Pixel) -> Pixel + Sync + Send) {
    each_row(
        out,
        width,
        || (),
        |_, y, row| {
            let held = src.row(y);
            for (x, (o, p)) in row.iter_mut().zip(held).enumerate() {
                *o = f(x, y, *p);
            }
        },
    );
}

fn mul_add(acc: &mut Pixel, p: &Pixel, w: f32) {
    for c in 0..4 {
        acc[c] += p[c] * w;
    }
}

fn convolve_row(row: &[Pixel], taps: &[f32], padded: &mut Vec<Pixel>, out: &mut [Pixel]) {
    let reach = taps.len() / 2;
    padded.clear();
    padded.resize(reach, CLEAR);
    padded.extend_from_slice(row);
    padded.resize(row.len() + 2 * reach, CLEAR);
    out.fill(CLEAR);
    for (k, w) in taps.iter().enumerate() {
        out.iter_mut().zip(&padded[k..]).for_each(|(o, p)| mul_add(o, p, *w));
    }
}

fn convolve_separable(src: Image, x: &[f32], y: &[f32], width: usize, out: &mut [Pixel]) {
    let mut across = vec![CLEAR; out.len()];
    each_row(&mut across, width, Vec::new, |padded, row_y, row| convolve_row(src.row(row_y), x, padded, row));
    let height = out.len() / width.max(1);
    let reach = y.len() / 2;
    each_row(
        out,
        width,
        || (),
        |_, row_y, row| {
            row.fill(CLEAR);
            for (k, w) in y.iter().enumerate() {
                let Some(from) = (row_y + k).checked_sub(reach).filter(|from| *from < height) else { continue };
                row.iter_mut().zip(&across[from * width..(from + 1) * width]).for_each(|(o, p)| mul_add(o, p, *w));
            }
        },
    );
}

fn convolve_full(src: Image, side: usize, weights: &[f32], width: usize, out: &mut [Pixel]) {
    let rows = weights.len() / side.max(1);
    let (reach_x, reach_y) = (side / 2, rows / 2);
    let height = src.size[1] as usize;
    each_row(out, width, Vec::new, |padded: &mut Vec<Pixel>, row_y, row| {
        row.fill(CLEAR);
        for (ky, line) in weights.chunks_exact(side).enumerate() {
            let Some(from) = (row_y + ky).checked_sub(reach_y).filter(|from| *from < height) else { continue };
            padded.clear();
            padded.resize(reach_x, CLEAR);
            padded.extend_from_slice(src.row(from));
            padded.resize(width + 2 * reach_x, CLEAR);
            for (kx, w) in line.iter().enumerate() {
                row.iter_mut().zip(&padded[kx..]).for_each(|(o, p)| mul_add(o, p, *w));
            }
        }
    });
}

pub(crate) fn filter_weight(filter: Filter, d: f32) -> f32 {
    let d = d.abs();
    match filter {
        Filter::Bilinear => (1.0 - d).max(0.0),
        Filter::Bicubic if d < 1.0 => (1.5 * d - 2.5) * d * d + 1.0,
        Filter::Bicubic if d < 2.0 => ((-0.5 * d + 2.5) * d - 4.0) * d + 2.0,
        Filter::Bicubic => 0.0,
    }
}

fn support(filter: Filter) -> f32 {
    match filter {
        Filter::Bilinear => 1.0,
        Filter::Bicubic => 2.0,
    }
}

struct AxisTaps {
    first: usize,
    weights: Vec<f32>,
}

fn axis_taps(filter: Filter, scale: f32, out_len: usize, src_len: usize) -> Vec<AxisTaps> {
    let widen = scale.max(1.0);
    let reach = support(filter) * widen;
    (0..out_len)
        .map(|i| {
            let centre = (i as f32 + 0.5) * scale - 0.5;
            let first = (centre - reach).floor() as isize + 1;
            let last = (centre + reach).ceil() as isize - 1;
            let raw: Vec<(isize, f32)> = (first..=last).map(|s| (s, filter_weight(filter, (s as f32 - centre) / widen))).collect();
            let total: f32 = raw.iter().map(|(_, w)| w).sum();
            let inside: Vec<(isize, f32)> = raw.into_iter().filter(|(s, _)| (0..src_len as isize).contains(s)).collect();
            let start = inside.iter().position(|(_, w)| *w != 0.0).unwrap_or(inside.len());
            let end = inside.iter().rposition(|(_, w)| *w != 0.0).map_or(start, |last| last + 1);
            let held = &inside[start..end];
            AxisTaps { first: held.first().map_or(0, |(s, _)| *s as usize), weights: held.iter().map(|(_, w)| w / total).collect() }
        })
        .collect()
}

fn resample(src: Image, filter: Filter, scale: [f32; 2], size: [u32; 2], out: &mut [Pixel]) {
    let [width, height] = size.map(|side| side as usize);
    let [src_width, src_height] = src.size.map(|side| side as usize);
    let (across_taps, down_taps) = (axis_taps(filter, scale[0], width, src_width), axis_taps(filter, scale[1], height, src_height));
    let mut across = vec![CLEAR; width * src_height];
    each_row(
        &mut across,
        width,
        || (),
        |_, y, row| {
            let held = src.row(y);
            for (o, taps) in row.iter_mut().zip(&across_taps) {
                *o = CLEAR;
                taps.weights.iter().zip(&held[taps.first..]).for_each(|(w, p)| mul_add(o, p, *w));
            }
        },
    );
    let clamp = filter == Filter::Bicubic;
    each_row(
        out,
        width,
        || (),
        |_, y, row| {
            row.fill(CLEAR);
            let taps = &down_taps[y];
            for (k, w) in taps.weights.iter().enumerate() {
                let from = taps.first + k;
                row.iter_mut().zip(&across[from * width..(from + 1) * width]).for_each(|(o, p)| mul_add(o, p, *w));
            }
            if clamp {
                row.iter_mut().flatten().for_each(|v| *v = v.max(0.0));
            }
        },
    );
}

fn bilinear(src: Image, x: f32, y: f32) -> Pixel {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    let mut out = CLEAR;
    for (dx, dy, w) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
        mul_add(&mut out, &src.at(x0 + dx, y0 + dy), w);
    }
    out
}
