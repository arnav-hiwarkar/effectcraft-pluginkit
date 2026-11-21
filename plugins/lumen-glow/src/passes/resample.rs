use craft_plugin::Pixel;

use crate::image::Image;
use crate::pixel::{CLEAR, add, mul_add, scale};

pub const DOWN_TAPS: [f32; 4] = [0.125, 0.375, 0.375, 0.125];
const NEAR: f32 = 0.75;
const FAR: f32 = 0.25;

pub fn decimate_row(src: &[Pixel], out: &mut [Pixel]) {
    let n = src.len();
    for (i, o) in out.iter_mut().enumerate() {
        let first = 2 * i as isize - 1;
        *o = if first >= 0 && first as usize + 3 < n {
            let s = &src[first as usize..first as usize + 4];
            let outer = add(s[0], s[3]);
            let inner = add(s[1], s[2]);
            mul_add(scale(outer, DOWN_TAPS[0]), inner, DOWN_TAPS[1])
        } else {
            DOWN_TAPS.iter().enumerate().fold(CLEAR, |acc, (k, w)| {
                let x = first + k as isize;
                if x >= 0 && (x as usize) < n {
                    mul_add(acc, src[x as usize], *w)
                } else {
                    acc
                }
            })
        };
    }
}

pub fn downsample(src: &Image, out: &mut Image, halve: [bool; 2], column: &mut [Pixel]) {
    let column = &mut column[..src.width];
    for j in 0..out.height {
        if halve[1] {
            column.fill(CLEAR);
            for (k, w) in DOWN_TAPS.iter().enumerate() {
                if let Some(row) = src.row_at(2 * j as isize + k as isize - 1) {
                    column
                        .iter_mut()
                        .zip(row)
                        .for_each(|(c, p)| *c = mul_add(*c, *p, *w));
                }
            }
        } else {
            column.copy_from_slice(src.row(j));
        }
        let dst = out.row_mut(j);
        if halve[0] {
            decimate_row(column, dst);
        } else {
            dst.copy_from_slice(column);
        }
    }
}

fn neighbour(index: usize) -> isize {
    let half = (index / 2) as isize;
    if index.is_multiple_of(2) {
        half - 1
    } else {
        half + 1
    }
}

pub fn expand_row(src: &[Pixel], out: &mut [Pixel]) {
    for (i, pair) in out.chunks_mut(2).enumerate() {
        let left = i.checked_sub(1).map_or(CLEAR, |l| src[l]);
        let right = src.get(i + 1).copied().unwrap_or(CLEAR);
        write_pair(pair, left, src[i], right);
    }
}

fn write_pair(pair: &mut [Pixel], left: Pixel, centre: Pixel, right: Pixel) {
    let near = scale(centre, NEAR);
    pair[0] = mul_add(near, left, FAR);
    if let Some(second) = pair.get_mut(1) {
        *second = mul_add(near, right, FAR);
    }
}

pub fn upsample_row(
    coarse: &Image,
    halve: [bool; 2],
    y: usize,
    out: &mut [Pixel],
    column: &mut [Pixel],
) {
    let column = &mut column[..coarse.width];
    if halve[1] {
        let near = coarse.row(y / 2);
        match coarse.row_at(neighbour(y)) {
            Some(far) => column
                .iter_mut()
                .zip(near.iter().zip(far))
                .for_each(|(c, (n, f))| *c = mul_add(scale(*n, NEAR), *f, FAR)),
            None => column
                .iter_mut()
                .zip(near)
                .for_each(|(c, n)| *c = scale(*n, NEAR)),
        }
    } else {
        column.copy_from_slice(coarse.row(y));
    }
    if halve[0] {
        expand_row(column, out);
    } else {
        out.copy_from_slice(column);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(size: [usize; 2], value: f32) -> Image {
        let mut image = Image::new(size);
        image.pixels.fill([value; 4]);
        image
    }

    #[test]
    fn flat_field_survives_downsample_then_upsample_away_from_the_border() {
        let src = flat([32, 32], 0.6);
        let mut half = Image::new([16, 16]);
        downsample(&src, &mut half, [true, true], &mut vec![CLEAR; 32]);
        let mut row = vec![CLEAR; 32];
        let mut column = vec![CLEAR; 32];
        for y in 4..28 {
            upsample_row(&half, [true, true], y, &mut row, &mut column);
            assert!(
                row[4..28].iter().all(|p| (p[0] - 0.6).abs() < 1e-6),
                "row {y}: {:?}",
                &row[4..28]
            );
        }
    }

    #[test]
    fn downsample_conserves_energy_and_centre_of_a_point() {
        let mut src = Image::new([32, 32]);
        src.pixels[16 * 32 + 16] = [8.0; 4];
        let mut half = Image::new([16, 16]);
        downsample(&src, &mut half, [true, true], &mut vec![CLEAR; 32]);
        let energy: f32 = half.pixels.iter().map(|p| p[0] * 4.0).sum();
        let centre_x: f32 = half
            .pixels
            .iter()
            .enumerate()
            .map(|(i, p)| p[0] * 4.0 * ((i % 16) as f32 + 0.5) * 2.0)
            .sum::<f32>()
            / energy;
        assert_eq!(energy, 8.0);
        assert_eq!(centre_x, 16.5);
    }
}
