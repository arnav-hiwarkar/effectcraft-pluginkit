use craft_plugin::{Frame, Pass, Pixel};

use crate::passes::resample::upsample_row;
use crate::pixel::{CLEAR, Mix, add, brightest, scale};
use crate::settings::{Blend, Settings, View};

const FULL_TO_HALF: [bool; 2] = [true, true];

pub fn composite(settings: &Settings, pass: &Pass, out: &mut Frame) {
    let core = Mix::from_floats(pass.f);
    let mut spread = vec![CLEAR; out.width];
    let mut column = vec![CLEAR; pass.aux.map_or(0, |glow| glow.width)];
    for (y, row) in out.rows_mut() {
        if let Some(glow) = &pass.aux {
            upsample_row(glow, FULL_TO_HALF, y, &mut spread, &mut column);
        }
        let source = pass.src.row(y as isize).unwrap_or(&[]);
        for (x, (o, s)) in row.iter_mut().zip(spread.iter()).enumerate() {
            let p = source.get(x).copied().unwrap_or(CLEAR);
            if p == CLEAR && *s == CLEAR {
                *o = CLEAR;
                continue;
            }
            let shaded = shade(settings, &core, p, *s);
            let dithered = match settings.dither {
                Some(d) => d.apply(shaded, x, y),
                None => shaded,
            };
            *o = with_light_alpha(dithered);
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
        let glow = if settings.view == View::GlowSource { CLEAR } else { scale(spread, settings.glow_opacity) };
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
        Blend::Add => [base[0] + glow[0], base[1] + glow[1], base[2] + glow[2], base[3]],
        Blend::Screen => {
            let screen = |b: f32, g: f32| b + g * (1.0 - b.clamp(0.0, 1.0));
            [screen(base[0], glow[0]), screen(base[1], glow[1]), screen(base[2], glow[2]), base[3]]
        }
    }
}

fn with_light_alpha(p: Pixel) -> Pixel {
    [p[0], p[1], p[2], p[3].max(brightest(p).clamp(0.0, 1.0))]
}
