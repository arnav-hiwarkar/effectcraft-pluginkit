use craft_plugin::{Pixel, Rows};

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

    #[test]
    fn flat_field_survives_upsample_away_from_the_border() {
        let half = Rows { width: 16, height: 16, first_row: 0, pixels: &[[0.6; 4]; 16 * 16] };
        let mut row = vec![CLEAR; 32];
        let mut column = vec![CLEAR; 32];
        for y in 4..28 {
            upsample_row(&half, [true, true], y, &mut row, &mut column);
            assert!(row[4..28].iter().all(|p| (p[0] - 0.6).abs() < 1e-6), "row {y}: {:?}", &row[4..28]);
        }
    }
}
