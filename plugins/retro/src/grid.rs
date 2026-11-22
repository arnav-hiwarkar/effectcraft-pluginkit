use craft_plugin::prelude::*;

use crate::math::{blur_radius, gauss};

pub struct Band<'a> {
    pub pixels: &'a [Pixel],
    pub width: usize,
    pub top: i32,
    pub rows: i32,
}

impl Band<'_> {
    pub fn at(&self, x: i32, y: i32) -> Pixel {
        let row = y - self.top;
        if x < 0 || x >= self.width as i32 || row < 0 || row >= self.rows {
            return [0.0; 4];
        }
        self.pixels[row as usize * self.width + x as usize]
    }
}

pub struct Grid {
    pub width: i32,
    pub first: i32,
    pub rows: i32,
    pub data: Vec<Pixel>,
}

impl Grid {
    pub fn at(&self, x: i32, y: i32) -> Pixel {
        let row = y - self.first;
        if x < 0 || x >= self.width || row < 0 || row >= self.rows {
            return [0.0; 4];
        }
        self.data[(row * self.width + x) as usize]
    }

    pub fn reduce(width: i32, first: i32, last: i32, factor: i32, emit: impl Fn(i32, i32) -> Pixel) -> Grid {
        let rows = (last - first).max(0);
        let norm = 1.0 / (factor * factor) as f32;
        let mut data = vec![[0.0; 4]; (width * rows) as usize];
        for row in 0..rows {
            for x in 0..width {
                let mut sum = [0.0f32; 4];
                for j in 0..factor {
                    for i in 0..factor {
                        let p = emit(x * factor + i, (first + row) * factor + j);
                        sum.iter_mut().zip(p).for_each(|(s, v)| *s += v);
                    }
                }
                data[(row * width + x) as usize] = sum.map(|s| s * norm);
            }
        }
        Grid { width, first, rows, data }
    }

    pub fn blur(&self, vertical: bool, fan: bool, sigmas: [f32; 3]) -> Grid {
        let radius = blur_radius(sigmas.iter().copied().fold(0.0, f32::max));
        let weights: Vec<[f32; 3]> = (-radius..=radius).map(|i| sigmas.map(|s| gauss(i, s))).collect();
        let totals: [f32; 3] = std::array::from_fn(|c| weights.iter().map(|w| w[c]).sum());
        let mut data = vec![[0.0; 4]; self.data.len()];
        for row in 0..self.rows {
            let y = self.first + row;
            for x in 0..self.width {
                let mut sum = [0.0f32; 3];
                for (k, w) in weights.iter().enumerate() {
                    let i = k as i32 - radius;
                    let p = if vertical { self.at(x, y + i) } else { self.at(x + i, y) };
                    for c in 0..3 {
                        sum[c] += w[c] * if fan { p[0] } else { p[c] };
                    }
                }
                data[(row * self.width + x) as usize] = [sum[0] / totals[0], sum[1] / totals[1], sum[2] / totals[2], 0.0];
            }
        }
        Grid { width: self.width, first: self.first, rows: self.rows, data }
    }

    pub fn sample_full(&self, factor: i32, x: f32, y: f32) -> Pixel {
        let f = factor as f32;
        crate::math::bilinear(|i, j| self.at(i, j), (x + 0.5) / f - 0.5, (y + 0.5) / f - 0.5)
    }
}

pub fn coarse_rows(top: i32, bottom: i32, factor: i32, height: u32) -> (i32, i32) {
    let limit = (height as i32 + factor - 1) / factor;
    (top / factor, ((bottom + factor - 1) / factor).min(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_averages_blocks_and_blur_keeps_a_constant() {
        let grid = Grid::reduce(2, 0, 2, 2, |x, y| [(x + 4 * y) as f32, 1.0, 1.0, 1.0]);
        assert_eq!(grid.at(0, 0)[0], 2.5);
        assert_eq!(grid.at(1, 1)[0], 12.5);
        let flat = Grid { width: 40, first: 0, rows: 1, data: vec![[2.0, 2.0, 2.0, 1.0]; 40] };
        let blurred = flat.blur(false, false, [1.0, 2.0, 3.0]);
        assert!((blurred.at(20, 0)[2] - 2.0).abs() < 1e-5);
        assert!(blurred.at(0, 0)[2] < 1.6);
    }

    #[test]
    fn fan_spreads_one_channel_into_three_widths() {
        let mut data = vec![[0.0; 4]; 41];
        data[20] = [1.0, 0.0, 0.0, 0.0];
        let line = Grid { width: 41, first: 0, rows: 1, data };
        let fanned = line.blur(false, true, [1.0, 2.0, 4.0]);
        let centre = fanned.at(20, 0);
        assert!(centre[0] > centre[1] && centre[1] > centre[2]);
        assert!(fanned.at(26, 0)[2] > fanned.at(26, 0)[0]);
    }

    #[test]
    fn coarse_rows_cover_the_band_and_stop_at_the_frame() {
        assert_eq!(coarse_rows(10, 30, 4, 1080), (2, 8));
        assert_eq!(coarse_rows(1070, 1080, 4, 1078), (267, 270));
    }
}
