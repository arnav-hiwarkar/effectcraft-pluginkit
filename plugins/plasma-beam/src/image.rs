use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct Image<const N: usize> {
    pub width: usize,
    pub height: usize,
    pub first: usize,
    pub data: Vec<[f32; N]>,
}

impl<const N: usize> Image<N> {
    pub fn new(width: usize, height: usize) -> Self {
        Self::window(width, height, 0..height)
    }

    pub fn window(width: usize, height: usize, rows: Range<usize>) -> Self {
        Image { width, height, first: rows.start, data: vec![[0.0; N]; width * rows.len()] }
    }

    pub fn rows(&self) -> Range<usize> {
        self.first..self.first + self.data.len() / self.width.max(1)
    }

    pub fn row(&self, y: usize) -> &[[f32; N]] {
        let at = (y - self.first) * self.width;
        &self.data[at..at + self.width]
    }

    pub fn row_mut(&mut self, y: usize) -> &mut [[f32; N]] {
        let at = (y - self.first) * self.width;
        &mut self.data[at..at + self.width]
    }
}

pub fn taps(fine: usize, coarse_len: usize) -> [(usize, f32); 2] {
    let last = coarse_len.saturating_sub(1);
    let m = fine / 2;
    if fine.is_multiple_of(2) { [(m.saturating_sub(1).min(last), 0.25), (m.min(last), 0.75)] } else { [(m.min(last), 0.75), ((m + 1).min(last), 0.25)] }
}

pub fn tap_rows(fine: Range<usize>, coarse_len: usize) -> Range<usize> {
    let start = taps(fine.start, coarse_len)[0].0;
    let end = taps(fine.end.saturating_sub(1), coarse_len)[1].0 + 1;
    start..end
}

pub fn lerp_row<const N: usize>(out: &mut [[f32; N]], a: &[[f32; N]], b: &[[f32; N]], wa: f32, wb: f32) {
    for ((o, a), b) in out.iter_mut().zip(a).zip(b) {
        *o = std::array::from_fn(|c| a[c] * wa + b[c] * wb);
    }
}

pub fn sample_row<const N: usize>(row: &[[f32; N]], fine: usize) -> [f32; N] {
    let [(lo, wl), (hi, wh)] = taps(fine, row.len());
    std::array::from_fn(|c| row[lo][c] * wl + row[hi][c] * wh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsampling_taps_land_on_the_coarse_pixel_centres() {
        assert_eq!(taps(4, 10), [(1, 0.25), (2, 0.75)]);
        assert_eq!(taps(5, 10), [(2, 0.75), (3, 0.25)]);
        assert_eq!(taps(0, 10), [(0, 0.25), (0, 0.75)]);
        assert_eq!(taps(19, 10), [(9, 0.75), (9, 0.25)]);
    }

    #[test]
    fn sampling_a_ramp_interpolates_linearly() {
        let row: Vec<[f32; 1]> = (0..4).map(|i| [i as f32]).collect();
        assert_eq!(sample_row(&row, 3), [1.25]);
        assert_eq!(sample_row(&row, 4), [1.75]);
    }
}
