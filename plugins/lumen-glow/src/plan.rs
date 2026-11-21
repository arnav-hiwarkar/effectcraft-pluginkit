use crate::pixel::{Mix, luma};
use crate::profile::Profile;
use crate::settings::{Settings, View};

const MIN_LEVEL_SIGMA: f32 = 2.0;
const DOWNSAMPLE_VARIANCE: f32 = 0.75;
const UPSAMPLE_VARIANCE: f32 = 0.1875;
const FIRST_FACTOR: usize = 2;
const COARSE_SIGMA: f32 = 16.0;
const COARSE_TEXEL_SIGMA: f32 = 2.0;
const TINT_LUMA_FLOOR: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub factor: [usize; 2],
    pub halve: [bool; 2],
    pub blur: [f32; 2],
    pub mix: Mix,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub core: Mix,
    pub levels: Vec<Level>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Emit,
    Downsample { level: usize },
    Blur { level: usize },
    Accumulate { level: usize },
    Composite,
}

impl Plan {
    pub fn new(settings: &Settings) -> Plan {
        if settings.view == View::GlowSource {
            return Plan {
                core: Mix::ZERO,
                levels: Vec::new(),
            };
        }
        let radius_scale = settings.channel_radius_scale();
        let octaves = radius_scale
            .iter()
            .map(|m| Profile::octave_count(settings.profile.radius * m))
            .max()
            .unwrap_or(1);
        let [red, green, blue] = radius_scale.map(|m| {
            Profile {
                radius: settings.profile.radius * m,
                ..settings.profile
            }
            .weights(octaves)
        });
        let mut core = Mix::ZERO;
        let mut levels: Vec<Level> = Vec::new();
        let mut chain = LevelChain::start();
        for (octave, ((r, g), b)) in red.into_iter().zip(green).zip(blue).enumerate() {
            let channel_weights = [r, g, b].map(|w| w * settings.gain);
            let mix = octave_colour(settings, octave, octaves).weighted(channel_weights);
            let sigma = (octave as f32).exp2() * settings.scale;
            if sigma < MIN_LEVEL_SIGMA {
                core = core.plus(mix);
                continue;
            }
            let axis_sigma = [sigma * settings.stretch[0], sigma * settings.stretch[1]];
            let previous = levels.last().map(|l| l.factor);
            levels.push(chain.level(previous, axis_sigma, mix));
        }
        Plan { core, levels }
    }

    pub fn sizes(&self, frame: [usize; 2]) -> Vec<[usize; 2]> {
        self.levels
            .iter()
            .map(|l| {
                [
                    frame[0].div_ceil(l.factor[0]),
                    frame[1].div_ceil(l.factor[1]),
                ]
            })
            .collect()
    }

    pub fn passes(&self) -> Vec<Pass> {
        let n = self.levels.len();
        if n == 0 {
            return vec![Pass::Composite];
        }
        let mut passes = vec![Pass::Emit];
        passes.extend((1..n).map(|level| Pass::Downsample { level }));
        passes.extend((0..n).map(|level| Pass::Blur { level }));
        passes.extend((0..n).rev().map(|level| Pass::Accumulate { level }));
        passes.push(Pass::Composite);
        passes
    }
}

struct LevelChain {
    extra_variance: [f32; 2],
}

impl LevelChain {
    fn start() -> LevelChain {
        let first = FIRST_FACTOR as f32;
        let edge = DOWNSAMPLE_VARIANCE + UPSAMPLE_VARIANCE * first * first;
        LevelChain {
            extra_variance: [edge; 2],
        }
    }

    fn level(&mut self, previous: Option<[usize; 2]>, axis_sigma: [f32; 2], mix: Mix) -> Level {
        let factor = [0, 1].map(|a| match previous {
            Some(p) => {
                power_of_two_floor(axis_sigma[a] / texel_sigma(axis_sigma[a])).clamp(p[a], 2 * p[a])
            }
            None => FIRST_FACTOR,
        });
        let halve = [0, 1].map(|a| previous.is_some_and(|p| factor[a] > p[a]));
        for a in 0..2 {
            if halve[a] {
                let f = factor[a] as f32;
                self.extra_variance[a] +=
                    DOWNSAMPLE_VARIANCE * (f / 2.0).powi(2) + UPSAMPLE_VARIANCE * f * f;
            }
        }
        let blur = [0, 1].map(|a| {
            (axis_sigma[a].powi(2) - self.extra_variance[a])
                .max(0.0)
                .sqrt()
                / factor[a] as f32
        });
        Level {
            factor,
            halve,
            blur,
            mix,
        }
    }
}

fn texel_sigma(sigma: f32) -> f32 {
    if sigma < COARSE_SIGMA {
        1.0
    } else {
        COARSE_TEXEL_SIGMA
    }
}

fn power_of_two_floor(x: f32) -> usize {
    1usize << (x.max(1.0).log2().floor() as u32).min(30)
}

fn octave_colour(settings: &Settings, octave: usize, octaves: usize) -> Mix {
    match settings.tint {
        None => Mix::identity(),
        Some(tint) => {
            let t = if octaves > 1 {
                octave as f32 / (octaves - 1) as f32
            } else {
                0.0
            };
            let colour: [f32; 3] =
                std::array::from_fn(|c| tint.inner[c] + (tint.outer[c] - tint.inner[c]) * t);
            let norm = luma(colour).max(TINT_LUMA_FLOOR);
            Mix::tint(colour.map(|c| c / norm), tint.mix)
        }
    }
}

#[cfg(test)]
mod tests {
    use craft_plugin::Value;

    use super::*;
    use crate::settings::tests::settings_with;

    #[test]
    fn levels_double_in_resolution_step_and_keep_small_blurs() {
        let plan = Plan::new(&settings_with(&[("Glow/radius", Value::Number(400.0))]));
        let factors: Vec<usize> = plan.levels.iter().map(|l| l.factor[0]).collect();
        assert_eq!(factors, [2, 4, 8, 8, 16, 32, 64]);
        assert!(
            plan.levels
                .iter()
                .all(|l| l.blur.iter().all(|b| (0.5..=2.0).contains(b))),
            "{:?}",
            plan.levels
        );
    }

    #[test]
    fn half_resolution_uses_one_level_fewer_at_the_same_layer_sigmas() {
        let full = Plan::new(&settings_with(&[("Glow/radius", Value::Number(400.0))]));
        let mut half_settings = settings_with(&[("Glow/radius", Value::Number(400.0))]);
        half_settings.scale = 0.5;
        let half = Plan::new(&half_settings);
        assert_eq!(half.levels.len() + 1, full.levels.len());
        assert_eq!(
            &half.levels[1..]
                .iter()
                .map(|l| l.factor)
                .collect::<Vec<_>>()[..],
            &[[4, 4], [8, 8], [8, 8], [16, 16], [32, 32]]
        );
    }

    #[test]
    fn horizontal_aspect_keeps_vertical_resolution() {
        let plan = Plan::new(&settings_with(&[
            ("Glow/radius", Value::Number(400.0)),
            ("Glow/aspect", Value::Number(0.6)),
        ]));
        let factors: Vec<[usize; 2]> = plan.levels.iter().map(|l| l.factor).collect();
        assert_eq!(
            factors,
            [[2, 2], [4, 2], [8, 2], [8, 2], [16, 4], [32, 8], [64, 8]]
        );
    }
}
