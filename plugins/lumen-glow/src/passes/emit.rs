use craft_plugin::{Frame, Rows};

use crate::knee::Knee;
use crate::passes::resample::{DOWN_TAPS, decimate_row};
use crate::pixel::{CLEAR, brightest, mul_add};
use crate::transfer::Transfer;

pub fn emit(source: &Rows, knee: Knee, transfer: Transfer, out: &mut Frame) {
    out.pixels.fill(CLEAR);
    let width = out.width;
    let bands = out.origin[1]..out.origin[1] + out.height;
    let mut line = vec![CLEAR; source.width];
    let mut reduced = vec![CLEAR; width];
    let rows = (2 * bands.start).saturating_sub(1)..(2 * bands.end + 1).min(source.height);
    for y in rows {
        let Some(row) = source.row(y as isize) else {
            continue;
        };
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
        decimate_row(&line, &mut reduced);
        for (k, w) in DOWN_TAPS.iter().enumerate() {
            let shifted = y + 1;
            if shifted < k || (shifted - k) % 2 != 0 {
                continue;
            }
            let j = (shifted - k) / 2;
            if bands.contains(&j) {
                let at = (j - bands.start) * width;
                out.pixels[at..at + width].iter_mut().zip(reduced.iter()).for_each(|(o, r)| *o = mul_add(*o, *r, *w));
            }
        }
    }
}
