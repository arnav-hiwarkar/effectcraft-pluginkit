use craft_plugin::{Frame, Pass};

use crate::passes::resample::upsample_row;
use crate::pixel::{CLEAR, Mix, add, mul_add};

const MIX_FLOATS: usize = 12;

fn kernel(floats: &[f32], from: usize, reach: usize) -> Vec<f32> {
    let half = floats.get(from..=from + reach).unwrap_or(&[1.0]);
    half[1..].iter().rev().chain(half).copied().collect()
}

pub fn level(pass: &Pass, out: &mut Frame) {
    let uint = |i: usize| pass.u.get(i).copied().unwrap_or(0);
    let reach = [uint(0) as usize, uint(1) as usize];
    let rows = kernel(pass.f, MIX_FLOATS, reach[0]);
    let columns = kernel(pass.f, MIX_FLOATS + 1 + reach[0], reach[1]);
    let mix = Mix::from_floats(pass.f);
    let width = out.width;
    let top = out.origin[1] as isize - reach[1] as isize;
    let mut blurred = vec![CLEAR; (out.height + 2 * reach[1]) * width];
    let mut padded = vec![CLEAR; width + 2 * reach[0]];
    for (i, line) in blurred.chunks_exact_mut(width.max(1)).enumerate() {
        let held = pass.src.row(top + i as isize);
        let Some(row) = held.and_then(|row| row.get(..width)) else {
            continue;
        };
        if rows.len() == 1 {
            line.copy_from_slice(row);
            continue;
        }
        padded[reach[0]..reach[0] + width].copy_from_slice(row);
        for (k, w) in rows.iter().enumerate() {
            line.iter_mut().zip(&padded[k..k + width]).for_each(|(o, p)| *o = mul_add(*o, *p, *w));
        }
    }
    let coarser = pass.aux.filter(|_| uint(2) != 0);
    let halve = [uint(3) != 0, uint(4) != 0];
    let mut up = vec![CLEAR; width];
    let mut column = vec![CLEAR; coarser.map_or(0, |c| c.width)];
    for (j, (y, row)) in out.rows_mut().enumerate() {
        if columns.len() == 1 {
            row.copy_from_slice(&blurred[j * width..(j + 1) * width]);
        } else {
            row.fill(CLEAR);
            for (k, w) in columns.iter().enumerate() {
                row.iter_mut().zip(&blurred[(j + k) * width..(j + k + 1) * width]).for_each(|(o, p)| *o = mul_add(*o, *p, *w));
            }
        }
        match &coarser {
            Some(coarser) => {
                upsample_row(coarser, halve, y, &mut up, &mut column);
                row.iter_mut().zip(up.iter()).for_each(|(p, u)| *p = add(mix.apply(*p), *u));
            }
            None => row.iter_mut().for_each(|p| *p = mix.apply(*p)),
        }
    }
}

#[cfg(test)]
mod tests {
    use craft_plugin::{GpuImage, Rows};

    use super::*;
    use crate::plan::Level;

    #[test]
    fn level_keeps_energy_of_an_interior_point() {
        let mut src = vec![CLEAR; 21 * 21];
        src[10 * 21 + 10] = [1.0; 4];
        let level_plan = Level { factor: [1, 1], halve: [false, false], blur: [1.0, 2.0], mix: Mix::identity() };
        let plan = crate::gpu::level_pass(&level_plan, GpuImage::Source, GpuImage::Output, None).expect("a level pass");
        let pass = Pass {
            kernel: &plan.kernel,
            f: &plan.f,
            u: &plan.u,
            src: Rows { width: 21, height: 21, first_row: 0, pixels: &src },
            aux: None,
            out_size: [21, 21],
        };
        let mut pixels = vec![CLEAR; 21 * 21];
        let mut out = Frame { width: 21, height: 21, origin: [0, 0], pixels: &mut pixels };
        level(&pass, &mut out);
        let energy: f32 = pixels.iter().map(|p| p[0]).sum();
        assert!((energy - 1.0).abs() < 1e-5);
        assert!(pixels[10 * 21 + 13][0] < pixels[13 * 21 + 10][0]);
    }
}
