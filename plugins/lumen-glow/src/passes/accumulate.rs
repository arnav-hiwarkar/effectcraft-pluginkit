use craft_plugin::Pixel;

use crate::image::Image;
use crate::passes::resample::upsample_row;
use crate::pixel::{Mix, add};

pub struct Coarser<'a> {
    pub image: &'a Image,
    pub halve: [bool; 2],
}

pub fn accumulate(
    level: &mut Image,
    mix: &Mix,
    coarser: Option<Coarser>,
    up: &mut [Pixel],
    column: &mut [Pixel],
) {
    let up = &mut up[..level.width];
    for y in 0..level.height {
        let row = level.row_mut(y);
        match &coarser {
            Some(c) => {
                upsample_row(c.image, c.halve, y, up, column);
                row.iter_mut()
                    .zip(up.iter())
                    .for_each(|(p, u)| *p = add(mix.apply(*p), *u));
            }
            None => row.iter_mut().for_each(|p| *p = mix.apply(*p)),
        }
    }
}
