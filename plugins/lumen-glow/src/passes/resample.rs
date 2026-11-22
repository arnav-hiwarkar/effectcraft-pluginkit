use craft_plugin::{Frame, Pixel, Rows};

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
                if x >= 0 && (x as usize) < n { mul_add(acc, src[x as usize], *w) } else { acc }
            })
        };
    }
}

pub fn downsample(src: &Rows, out: &mut Frame, halve: [bool; 2]) {
    let mut column = vec![CLEAR; src.width];
    for (j, dst) in out.rows_mut() {
        column.fill(CLEAR);
        if halve[1] {
            for (k, w) in DOWN_TAPS.iter().enumerate() {
                if let Some(row) = src.row(2 * j as isize + k as isize - 1) {
                    column.iter_mut().zip(row).for_each(|(c, p)| *c = mul_add(*c, *p, *w));
                }
            }
        } else if let Some(row) = src.row(j as isize) {
            column.copy_from_slice(row);
        }
        if halve[0] {
            decimate_row(&column, dst);
        } else {
            dst.iter_mut().zip(&column).for_each(|(d, c)| *d = *c);
        }
    }
}

fn neighbour(index: usize) -> isize {
    let half = (index / 2) as isize;
    if index.is_multiple_of(2) { half - 1 } else { half + 1 }
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

pub fn upsample_row(coarse: &Rows, halve: [bool; 2], y: usize, out: &mut [Pixel], column: &mut [Pixel]) {
    let column = &mut column[..coarse.width];
    let near = coarse.row(if halve[1] { y / 2 } else { y } as isize);
    let Some(near) = near else {
        out.fill(CLEAR);
        return;
    };
    if halve[1] {
        match coarse.row(neighbour(y)) {
            Some(far) => column.iter_mut().zip(near.iter().zip(far)).for_each(|(c, (n, f))| *c = mul_add(scale(*n, NEAR), *f, FAR)),
            None => column.iter_mut().zip(near).for_each(|(c, n)| *c = scale(*n, NEAR)),
        }
    } else {
        column.copy_from_slice(near);
    }
    if halve[0] {
        expand_row(column, out);
    } else {
        out.iter_mut().zip(column.iter()).for_each(|(o, c)| *o = *c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn halved(src: &[Pixel]) -> Vec<Pixel> {
        let src = Rows { width: 32, height: 32, first_row: 0, pixels: src };
        let mut half = vec![CLEAR; 16 * 16];
        let mut out = Frame { width: 16, height: 16, origin: [0, 0], pixels: &mut half };
        downsample(&src, &mut out, [true, true]);
        half
    }

    #[test]
    fn flat_field_survives_downsample_then_upsample_away_from_the_border() {
        let half = halved(&[[0.6; 4]; 32 * 32]);
        let half = Rows { width: 16, height: 16, first_row: 0, pixels: &half };
        let mut row = vec![CLEAR; 32];
        let mut column = vec![CLEAR; 32];
        for y in 4..28 {
            upsample_row(&half, [true, true], y, &mut row, &mut column);
            assert!(row[4..28].iter().all(|p| (p[0] - 0.6).abs() < 1e-6), "row {y}: {:?}", &row[4..28]);
        }
    }

    #[test]
    fn downsample_conserves_energy_and_centre_of_a_point() {
        let mut src = vec![CLEAR; 32 * 32];
        src[16 * 32 + 16] = [8.0; 4];
        let half = halved(&src);
        let energy: f32 = half.iter().map(|p| p[0] * 4.0).sum();
        let centre_x: f32 = half.iter().enumerate().map(|(i, p)| p[0] * 4.0 * ((i % 16) as f32 + 0.5) * 2.0).sum::<f32>() / energy;
        assert_eq!(energy, 8.0);
        assert_eq!(centre_x, 16.5);
    }
}
