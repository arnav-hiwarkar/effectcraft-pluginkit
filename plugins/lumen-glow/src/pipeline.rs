use craft_plugin::Pixel;

use crate::image::Image;
use crate::passes::accumulate::{Coarser, accumulate};
use crate::passes::blur::blur;
use crate::passes::composite::{Target, composite};
use crate::passes::emit::{Source, emit};
use crate::passes::resample::downsample;
use crate::pixel::CLEAR;
use crate::plan::{Pass, Plan};
use crate::settings::Settings;

struct Scratch {
    line: Vec<Pixel>,
    column: Vec<Pixel>,
    padded: Vec<Pixel>,
    copy: Vec<Pixel>,
}

pub fn render(settings: &Settings, pixels: &mut [Pixel], width: usize) {
    if width == 0 || pixels.is_empty() {
        return;
    }
    let height = pixels.len() / width;
    let plan = Plan::new(settings);
    let mut levels: Vec<Image> = plan
        .sizes([width, height])
        .into_iter()
        .map(Image::new)
        .collect();
    let mut scratch = Scratch {
        line: vec![CLEAR; width],
        column: vec![CLEAR; width],
        padded: Vec::new(),
        copy: Vec::new(),
    };
    for pass in plan.passes() {
        match pass {
            Pass::Emit => emit(
                &Source { pixels, width },
                settings.knee,
                settings.transfer,
                &mut levels[0],
                &mut scratch.line,
                &mut scratch.column,
            ),
            Pass::Downsample { level } => {
                let (finer, coarser) = levels.split_at_mut(level);
                downsample(
                    &finer[level - 1],
                    &mut coarser[0],
                    plan.levels[level].halve,
                    &mut scratch.column,
                );
            }
            Pass::Blur { level } => blur(
                &mut levels[level],
                plan.levels[level].blur,
                &mut scratch.padded,
                &mut scratch.copy,
            ),
            Pass::Accumulate { level } => {
                let (finer, coarser) = levels.split_at_mut(level + 1);
                let coarser = coarser.first().map(|image| Coarser {
                    image,
                    halve: plan.levels[level + 1].halve,
                });
                accumulate(
                    &mut finer[level],
                    &plan.levels[level].mix,
                    coarser,
                    &mut scratch.line,
                    &mut scratch.column,
                );
            }
            Pass::Composite => composite(
                Target { pixels, width },
                settings,
                &plan.core,
                levels.first(),
                &mut scratch.line,
                &mut scratch.column,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use craft_plugin::Value;

    use super::*;
    use crate::settings::tests::settings_with;

    const SIZE: usize = 257;
    const CENTRE: usize = SIZE / 2;

    fn glow_of_point(overrides: &[(&str, Value)]) -> Vec<Pixel> {
        glow_of_point_with_brightness(1.0, overrides)
    }

    fn glow_of_point_with_brightness(brightness: f32, overrides: &[(&str, Value)]) -> Vec<Pixel> {
        let mut base = vec![
            ("Output/view", Value::Number(1.0)),
            ("Output/dither", Value::Bool(false)),
        ];
        base.extend(overrides.iter().cloned());
        let mut pixels = vec![CLEAR; SIZE * SIZE];
        pixels[CENTRE * SIZE + CENTRE] = [brightness, brightness, brightness, 1.0];
        render(&settings_with(&base), &mut pixels, SIZE);
        pixels
    }

    fn at(pixels: &[Pixel], dx: isize, dy: isize) -> f32 {
        pixels[(CENTRE as isize + dy) as usize * SIZE + (CENTRE as isize + dx) as usize][1]
    }

    #[test]
    fn a_single_bright_pixel_glows_radially_symmetric() {
        let glow = glow_of_point(&[("Glow/radius", Value::Number(120.0))]);
        for d in [8, 20, 50, 90] {
            let axis = [
                at(&glow, d, 0),
                at(&glow, -d, 0),
                at(&glow, 0, d),
                at(&glow, 0, -d),
            ];
            let diagonal_d = (d as f32 / std::f32::consts::SQRT_2).round() as isize;
            let diagonal = at(&glow, diagonal_d, diagonal_d);
            let (lo, hi) = axis
                .iter()
                .fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            assert!(lo > 0.0 && hi / lo < 1.05, "distance {d}: {axis:?}");
            assert!(
                (diagonal / hi - 1.0).abs() < 0.1,
                "distance {d}: diagonal {diagonal} axis {hi}"
            );
        }
    }

    #[test]
    fn glow_falls_off_monotonically_from_the_core() {
        let glow = glow_of_point(&[
            ("Glow/radius", Value::Number(120.0)),
            ("Output/gamma", Value::Bool(false)),
        ]);
        let profile: Vec<f32> = (0..120).map(|d| at(&glow, d, 0)).collect();
        assert!(profile.windows(2).all(|w| w[1] <= w[0]), "{profile:?}");
    }

    #[test]
    fn hdr_sources_glow_in_proportion_to_their_energy() {
        let linear = [
            ("Glow/radius", Value::Number(60.0)),
            ("Output/gamma", Value::Bool(false)),
        ];
        let dim = glow_of_point_with_brightness(1.0, &linear);
        let hot = glow_of_point_with_brightness(16.0, &linear);
        for d in [4, 15, 40] {
            let ratio = at(&hot, d, 0) / at(&dim, d, 0);
            assert!((ratio - 16.0).abs() < 1e-3, "distance {d}: ratio {ratio}");
        }
    }

    fn disc(size: usize, radius: f32) -> Vec<Pixel> {
        let c = size as f32 / 2.0;
        (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 + 0.5 - c, (i / size) as f32 + 0.5 - c);
                if x * x + y * y < radius * radius {
                    [1.0; 4]
                } else {
                    CLEAR
                }
            })
            .collect()
    }

    #[test]
    fn half_resolution_matches_the_full_resolution_glow() {
        let overrides = [
            ("Glow/radius", Value::Number(80.0)),
            ("Output/view", Value::Number(1.0)),
            ("Output/gamma", Value::Bool(false)),
            ("Output/dither", Value::Bool(false)),
        ];
        let mut full = disc(256, 24.0);
        render(&settings_with(&overrides), &mut full, 256);
        let mut half_settings = settings_with(&overrides);
        half_settings.scale = 0.5;
        let mut half = disc(128, 12.0);
        render(&half_settings, &mut half, 128);
        for (x, y) in [(64, 30), (64, 10), (100, 100), (64, 64), (20, 64)] {
            let averaged: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| full[(2 * y + dy) * 256 + 2 * x + dx][1])
                .sum::<f32>()
                / 4.0;
            let preview = half[y * 128 + x][1];
            assert!(
                (preview / averaged - 1.0).abs() < 0.04,
                "({x}, {y}): half {preview} full {averaged}"
            );
        }
    }
}
