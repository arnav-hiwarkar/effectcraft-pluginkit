use craft_plugin::Pixel;

use crate::image::Image;
use crate::passes::resample::upsample_row;
use crate::pixel::{CLEAR, Mix, add, brightest, scale};
use crate::settings::{Blend, Settings, View};

const FULL_TO_HALF: [bool; 2] = [true, true];

pub struct Target<'a> {
    pub pixels: &'a mut [Pixel],
    pub width: usize,
}

pub fn composite(
    target: Target,
    settings: &Settings,
    core: &Mix,
    glow: Option<&Image>,
    spread: &mut [Pixel],
    column: &mut [Pixel],
) {
    let spread = &mut spread[..target.width];
    spread.fill(CLEAR);
    for (y, row) in target.pixels.chunks_exact_mut(target.width).enumerate() {
        if let Some(glow) = glow {
            upsample_row(glow, FULL_TO_HALF, y, spread, column);
        }
        for (x, (p, s)) in row.iter_mut().zip(spread.iter()).enumerate() {
            if *p == CLEAR && *s == CLEAR {
                continue;
            }
            let shaded = shade(settings, core, *p, *s);
            let dithered = match settings.dither {
                Some(d) => d.apply(shaded, x, y),
                None => shaded,
            };
            *p = with_light_alpha(dithered);
        }
    }
}

fn shade(settings: &Settings, core: &Mix, src: Pixel, spread: Pixel) -> Pixel {
    let (light, base_alpha) = light(settings, core, src, spread);
    let encoded = settings.transfer.encode(light);
    [encoded[0], encoded[1], encoded[2], base_alpha]
}

fn light(settings: &Settings, core: &Mix, src: Pixel, spread: Pixel) -> (Pixel, f32) {
    if src == CLEAR {
        let glow = if settings.view == View::GlowSource {
            CLEAR
        } else {
            scale(spread, settings.glow_opacity)
        };
        return (glow, 0.0);
    }
    let linear = settings.transfer.decode(src);
    let emission = settings.knee.emit(linear);
    let glow = || scale(add(core.apply(emission), spread), settings.glow_opacity);
    match settings.view {
        View::GlowSource => (emission, 0.0),
        View::GlowOnly => (glow(), 0.0),
        View::Final => {
            let base = scale(linear, settings.source_opacity);
            (blend(settings.blend, base, glow()), base[3])
        }
    }
}

fn blend(mode: Blend, base: Pixel, glow: Pixel) -> Pixel {
    match mode {
        Blend::Add => [
            base[0] + glow[0],
            base[1] + glow[1],
            base[2] + glow[2],
            base[3],
        ],
        Blend::Screen => {
            let screen = |b: f32, g: f32| b + g * (1.0 - b.clamp(0.0, 1.0));
            [
                screen(base[0], glow[0]),
                screen(base[1], glow[1]),
                screen(base[2], glow[2]),
                base[3],
            ]
        }
    }
}

fn with_light_alpha(p: Pixel) -> Pixel {
    [p[0], p[1], p[2], p[3].max(brightest(p).clamp(0.0, 1.0))]
}
