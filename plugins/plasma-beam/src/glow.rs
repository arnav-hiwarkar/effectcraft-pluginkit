use craft_plugin::math::log2;
use std::f32::consts::{SQRT_2, TAU};

use crate::beam::Node;
use std::ops::Range;

use crate::image::{Image, lerp_row, sample_row, tap_rows, taps};
use crate::plane::{Bounds, Plane};
use crate::settings::GlowBand;
use crate::shade::{Rgb, Shading};

const REDUCE: [f32; 4] = [0.125, 0.375, 0.375, 0.125];

#[derive(Clone, Debug, PartialEq)]
pub struct Glow {
    pub image: Image<3>,
    pub guard: [usize; 2],
}

fn even_guard(overflow: f32, reach: f32) -> usize {
    let guard = overflow.clamp(0.0, reach.max(0.0)).ceil() as usize;
    guard + guard % 2
}

fn base_guard(bounds: Bounds, reach: f32, size: [usize; 2]) -> ([usize; 2], [usize; 2]) {
    let margin = 2.0;
    let lo = [0, 1].map(|axis| even_guard(margin - bounds.min[axis], reach));
    let hi = [0, 1].map(|axis| even_guard(bounds.max[axis] + margin - size[axis] as f32, reach));
    let dims = [0, 1].map(|axis| (size[axis] + lo[axis] + hi[axis]).div_ceil(2));
    (dims, lo)
}

fn source_rows(rows: &Range<usize>, height: usize) -> Range<usize> {
    (2 * rows.start).saturating_sub(1)..(2 * rows.end + 1).min(height)
}

fn hull(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
}

pub fn windows(heights: &[usize], wanted: Range<usize>) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut sums = vec![wanted];
    for &height in &heights[1..] {
        let next = tap_rows(sums[sums.len() - 1].clone(), height);
        sums.push(next);
    }
    let mut levels = sums.clone();
    for k in (0..heights.len() - 1).rev() {
        levels[k] = hull(sums[k].clone(), source_rows(&levels[k + 1], heights[k]));
    }
    (levels, sums)
}

fn level_count(bands: &[GlowBand], base: [usize; 2]) -> usize {
    let wanted = bands.iter().filter(|b| b.energy[0] + b.energy[1] > 0.0).map(|b| log2((b.sigma / SQRT_2).max(1.0)).ceil() as usize + 1).max().unwrap_or(1);
    let available = (0..).take_while(|&k| base[0].min(base[1]) >> k > 1).count().max(1);
    wanted.min(available)
}

pub fn level_weights(bands: &[GlowBand], source_radius: f32, count: usize) -> Vec<[f32; 2]> {
    let mut weights = vec![[0.0; 2]; count];
    let top = (count - 1) as f32;
    for band in bands {
        let level = log2((band.sigma / SQRT_2).max(1e-6)).clamp(0.0, top);
        let low = level.floor() as usize;
        let share = level - low as f32;
        let norm = (TAU.sqrt() * band.sigma / (2.0 * source_radius.max(1e-6))).max(0.0);
        for (index, part) in [(low, 1.0 - share), ((low + 1).min(count - 1), share)] {
            for (weight, energy) in weights[index].iter_mut().zip(band.energy) {
                *weight += energy * norm * part;
            }
        }
    }
    weights
}

fn reduce_row(src: &[[f32; 1]], out: &mut [[f32; 1]]) {
    let last = src.len() as isize - 1;
    for (j, cell) in out.iter_mut().enumerate() {
        let first = 2 * j as isize - 1;
        cell[0] = REDUCE
            .iter()
            .enumerate()
            .map(|(k, w)| {
                let i = first + k as isize;
                if (0..=last).contains(&i) { src[i as usize][0] * w } else { 0.0 }
            })
            .sum();
    }
}

pub fn downsample(src: &Image<1>, wanted: Range<usize>) -> Image<1> {
    let (width, height) = (src.width.div_ceil(2), src.height.div_ceil(2));
    let read = source_rows(&wanted, src.height);
    let mut rows = Image::<1>::window(width, src.height, read.clone());
    for y in read {
        reduce_row(src.row(y), rows.row_mut(y));
    }
    let mut out = Image::<1>::window(width, height, wanted.clone());
    for j in wanted {
        let out_row = out.row_mut(j);
        for (k, w) in REDUCE.iter().enumerate() {
            let Some(y) = (2 * j + k).checked_sub(1).filter(|&y| y < src.height) else { continue };
            for (o, r) in out_row.iter_mut().zip(rows.row(y)) {
                o[0] += r[0] * w;
            }
        }
    }
    out
}

fn upsample_add(coarse: &Image<2>, fine: &Image<1>, weight: [f32; 2], wanted: Range<usize>) -> Image<2> {
    let mut out = Image::<2>::window(fine.width, fine.height, wanted.clone());
    let mut column = vec![[0.0; 2]; coarse.width];
    for y in wanted {
        let [(a, wa), (b, wb)] = taps(y, coarse.height);
        lerp_row(&mut column, coarse.row(a), coarse.row(b), wa, wb);
        for (x, (o, f)) in out.row_mut(y).iter_mut().zip(fine.row(y)).enumerate() {
            let up = sample_row(&column, x);
            *o = [up[0] + weight[0] * f[0], up[1] + weight[1] * f[0]];
        }
    }
    out
}

pub fn heights(base: usize, count: usize) -> Vec<usize> {
    std::iter::successors(Some(base), |h| Some(h.div_ceil(2))).take(count).collect()
}

pub fn energy(base: Image<1>, weights: &[[f32; 2]], level_rows: &[Range<usize>], sum_rows: &[Range<usize>]) -> Image<2> {
    let mut levels = vec![base];
    while levels.len() < weights.len() {
        let next = downsample(&levels[levels.len() - 1], level_rows[levels.len()].clone());
        levels.push(next);
    }
    let top = &levels[levels.len() - 1];
    let weight = weights[levels.len() - 1];
    let data = top.data.iter().map(|v| [v[0] * weight[0], v[0] * weight[1]]).collect();
    let mut acc = Image::<2> { width: top.width, height: top.height, first: top.first, data };
    for ((level, weight), rows) in levels.iter().zip(weights).zip(sum_rows).rev().skip(1) {
        acc = upsample_add(&acc, level, *weight, rows.clone());
    }
    acc
}

pub fn render<'a>(
    strokes: impl Iterator<Item = &'a [Node]> + Clone,
    bands: &[GlowBand],
    source_radius: f32,
    reach: f32,
    shading: &Shading,
    size: [usize; 2],
    rows: Range<usize>,
) -> Option<Glow> {
    let bounds = Bounds::of_nodes(strokes.clone().flatten())?;
    let (dims, guard) = base_guard(bounds, reach, size);
    let count = level_count(bands, dims);
    let wanted = tap_rows(rows.start + guard[1]..rows.end + guard[1], dims[1]);
    let (level_rows, sum_rows) = windows(&heights(dims[1], count), wanted);
    let mut base = Plane::window([-(guard[0] as i32), -(guard[1] as i32)], dims, 2.0, level_rows[0].clone());
    strokes.for_each(|nodes| base.draw(nodes));
    let energy = energy(base.image, &level_weights(bands, source_radius, count), &level_rows, &sum_rows);
    let data: Vec<Rgb> = energy.data.iter().map(|e| shading.glow(*e)).collect();
    Some(Glow { image: Image { width: energy.width, height: energy.height, first: energy.first, data }, guard })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_sigma(level: f32) -> f32 {
        SQRT_2 * level.exp2()
    }

    #[test]
    fn downsampling_preserves_energy() {
        let mut image = Image::<1>::new(32, 32);
        image.data[16 * 32 + 15] = [8.0];
        let total: f32 = downsample(&image, 0..16).data.iter().map(|p| p[0]).sum();
        assert_eq!(total, 2.0);
    }

    #[test]
    fn a_band_between_levels_splits_its_weight() {
        let band = GlowBand { sigma: level_sigma(1.25), energy: [2.0, 0.0] };
        let weights = level_weights(&[band], 1.0, 4);
        let norm = TAU.sqrt() * band.sigma / 2.0;
        assert!((weights[1][0] - 1.5 * norm).abs() < 1e-4);
        assert!((weights[2][0] - 0.5 * norm).abs() < 1e-4);
        assert_eq!(weights[0], [0.0; 2]);
    }

    #[test]
    fn glow_of_a_straight_line_peaks_near_its_weight() {
        let mut base = Plane::new([0, 0], [96, 96], 2.0);
        base.draw(&[Node { at: [20.0, 96.0], radius: 3.0 }, Node { at: [172.0, 96.0], radius: 3.0 }]);
        let band = GlowBand { sigma: level_sigma(2.0), energy: [0.0, 1.0] };
        let (levels, sums) = windows(&heights(96, 4), 0..96);
        let energy = energy(base.image, &level_weights(&[band], 3.0, 4), &levels, &sums);
        let peak = energy.row(47)[48][1].max(energy.row(48)[48][1]);
        assert!((peak - 1.0).abs() < 0.25, "peak {peak}");
        assert!(energy.data.iter().all(|e| e[0] == 0.0 && e[1].is_finite()));
    }
}
