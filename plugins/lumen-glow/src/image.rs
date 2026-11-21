use craft_plugin::Pixel;

use crate::pixel::CLEAR;

pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Pixel>,
}

impl Image {
    pub fn new(size: [usize; 2]) -> Image {
        Image {
            width: size[0],
            height: size[1],
            pixels: vec![CLEAR; size[0] * size[1]],
        }
    }

    pub fn row(&self, y: usize) -> &[Pixel] {
        &self.pixels[y * self.width..(y + 1) * self.width]
    }

    pub fn row_mut(&mut self, y: usize) -> &mut [Pixel] {
        &mut self.pixels[y * self.width..(y + 1) * self.width]
    }

    pub fn row_at(&self, y: isize) -> Option<&[Pixel]> {
        (y >= 0 && (y as usize) < self.height).then(|| self.row(y as usize))
    }
}
