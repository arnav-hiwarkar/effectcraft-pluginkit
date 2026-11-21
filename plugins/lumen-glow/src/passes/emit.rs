use craft_plugin::Pixel;

use crate::image::Image;
use crate::knee::Knee;
use crate::passes::resample::{DOWN_TAPS, decimate_row};
use crate::pixel::{CLEAR, brightest, mul_add};
use crate::transfer::Transfer;

pub struct Source<'a> {
    pub pixels: &'a [Pixel],
    pub width: usize,
}

pub fn emit(
    source: &Source,
    knee: Knee,
    transfer: Transfer,
    out: &mut Image,
    line: &mut [Pixel],
    reduced: &mut [Pixel],
) {
    out.pixels.fill(CLEAR);
    let line = &mut line[..source.width];
    let reduced = &mut reduced[..out.width];
    for (y, row) in source.pixels.chunks_exact(source.width).enumerate() {
        let mut lit = false;
        for (e, p) in line.iter_mut().zip(row) {
            *e = if brightest(*p) > 0.0 {
                lit = true;
                knee.emit(transfer.decode(*p))
            } else {
                CLEAR
            };
        }
        if !lit {
            continue;
        }
        decimate_row(line, reduced);
        for (k, w) in DOWN_TAPS.iter().enumerate() {
            let shifted = y + 1;
            if shifted < k || (shifted - k) % 2 != 0 {
                continue;
            }
            let j = (shifted - k) / 2;
            if j < out.height {
                out.row_mut(j)
                    .iter_mut()
                    .zip(reduced.iter())
                    .for_each(|(o, r)| *o = mul_add(*o, *r, *w));
            }
        }
    }
}
