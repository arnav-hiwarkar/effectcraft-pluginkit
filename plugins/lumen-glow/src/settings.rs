use craft_plugin::Cx;

use crate::knee::Knee;
use crate::profile::Profile;
use crate::transfer::Transfer;

const MAX_STRETCH_OCTAVES: f32 = 5.0;
const MAX_ABERRATION_OCTAVES: f32 = 1.0;
const MAX_PADDING: f32 = 4096.0;
const MIN_PIXEL_BUDGET: f64 = 16_777_216.0;
const AREA_BUDGET: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Add,
    Screen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Final,
    GlowOnly,
    GlowSource,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub inner: [f32; 3],
    pub outer: [f32; 3],
    pub mix: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dither {
    pub seed: u32,
    pub origin: [i64; 2],
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub profile: Profile,
    pub scale: f32,
    pub gain: f32,
    pub knee: Knee,
    pub stretch: [f32; 2],
    pub aberration: f32,
    pub tint: Option<Tint>,
    pub blend: Blend,
    pub view: View,
    pub source_opacity: f32,
    pub glow_opacity: f32,
    pub transfer: Transfer,
    pub dither: Option<Dither>,
}

impl Settings {
    pub fn from_cx(cx: &Cx) -> Settings {
        let unit = |id: &str| (cx.f(id) / 100.0) as f32;
        let transfer = if cx.bool("Output/gamma") {
            Transfer::srgb()
        } else {
            Transfer::Linear
        };
        let colour = |id: &str| {
            let c = cx.color(id);
            [c[0], c[1], c[2]].map(|v| transfer.decode_value(v))
        };
        let threshold = transfer.decode_value(cx.f("Glow/threshold") as f32);
        Settings {
            profile: Profile {
                radius: cx.f("Glow/radius").max(0.0) as f32,
                falloff: unit("Glow/falloff"),
            },
            scale: cx.scale.max(1e-3) as f32,
            gain: (cx.f("Glow/exposure") as f32).exp2(),
            knee: Knee::new(threshold, unit("Glow/smoothness")),
            stretch: stretch(cx.f("Glow/aspect") as f32),
            aberration: unit("Aberration/amount").clamp(0.0, 1.0) * MAX_ABERRATION_OCTAVES,
            tint: cx.bool("Colour/tint").then(|| Tint {
                inner: colour("Colour/inner"),
                outer: colour("Colour/outer"),
                mix: unit("Colour/mix").clamp(0.0, 1.0),
            }),
            blend: if cx.index("Output/blend") == 1 {
                Blend::Screen
            } else {
                Blend::Add
            },
            view: match cx.index("Output/view") {
                1 => View::GlowOnly,
                2 => View::GlowSource,
                _ => View::Final,
            },
            source_opacity: unit("Output/source_opacity").clamp(0.0, 1.0),
            glow_opacity: unit("Output/glow_opacity").clamp(0.0, 1.0),
            transfer,
            dither: (cx.bool("Output/dither") && !cx.float).then(|| Dither {
                seed: (cx.instance ^ (cx.instance >> 32)) as u32,
                origin: [cx.offset[0].round() as i64, cx.offset[1].round() as i64],
            }),
        }
    }

    pub fn channel_radius_scale(&self) -> [f32; 3] {
        [self.aberration.exp2(), 1.0, (-self.aberration).exp2()]
    }

    pub fn padding(&self) -> u32 {
        let widest = self.channel_radius_scale()[0];
        (Profile::tail_extent(self.profile.radius * widest) * self.scale)
            .ceil()
            .clamp(0.0, MAX_PADDING) as u32
    }

    pub fn frame_padding(&self, size: [u32; 2]) -> u32 {
        let pad = self.padding();
        let (w, h) = (f64::from(size[0]), f64::from(size[1]));
        let budget = MIN_PIXEL_BUDGET.max(AREA_BUDGET * w * h);
        let padded = |p: f64| (w + 2.0 * p) * (h + 2.0 * p);
        if padded(f64::from(pad)) <= budget {
            return pad;
        }
        let fit = (((w - h).powi(2) + 4.0 * budget).sqrt() - (w + h)) / 4.0;
        (fit.floor().max(0.0) as u32).min(pad)
    }
}

fn stretch(aspect: f32) -> [f32; 2] {
    let squeeze = (-aspect.abs().min(1.0) * MAX_STRETCH_OCTAVES).exp2();
    if aspect >= 0.0 {
        [1.0, squeeze]
    } else {
        [squeeze, 1.0]
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;

    use craft_plugin::{ParamKind, RenderCx, Value};

    use super::*;

    pub fn settings_with(overrides: &[(&str, Value)]) -> Settings {
        let mut params: BTreeMap<String, Value> = crate::spec::spec()
            .params
            .into_iter()
            .filter_map(|p| {
                let value = match p.kind {
                    ParamKind::Slider { default, .. } | ParamKind::Percent { default } => {
                        Value::Number(default)
                    }
                    ParamKind::Checkbox { default } => Value::Bool(default),
                    ParamKind::Popup { default, .. } => Value::Number(f64::from(default)),
                    ParamKind::Color { default } => Value::List(default.to_vec()),
                    _ => return None,
                };
                Some((p.id, value))
            })
            .collect();
        for (id, value) in overrides {
            assert!(params.contains_key(*id), "no parameter {id}");
            params.insert(id.to_string(), value.clone());
        }
        Settings::from_cx(&Cx::new(RenderCx {
            params,
            scale: 1.0,
            ..Default::default()
        }))
    }

    #[test]
    fn padding_grows_with_radius_and_is_capped() {
        let pad = |r: f64| settings_with(&[("Glow/radius", Value::Number(r))]).padding();
        assert_eq!(
            [pad(10.0), pad(120.0), pad(400.0), pad(2000.0)],
            [14, 168, 560, 2800]
        );
        let wide = settings_with(&[
            ("Glow/radius", Value::Number(3000.0)),
            ("Aberration/amount", Value::Number(100.0)),
        ]);
        assert_eq!(wide.padding(), 4096);
    }

    #[test]
    fn frame_padding_keeps_the_padded_frame_within_the_pixel_budget() {
        let settings = settings_with(&[("Glow/radius", Value::Number(3000.0))]);
        for size in [[1u32, 1], [1920, 1080], [3840, 2160], [16000, 100]] {
            let pad = settings.frame_padding(size);
            let area = |p: u32| f64::from(size[0] + 2 * p) * f64::from(size[1] + 2 * p);
            let budget = MIN_PIXEL_BUDGET.max(AREA_BUDGET * area(0));
            assert!(
                pad <= settings.padding() && area(pad) <= budget,
                "{size:?}: {pad}"
            );
            assert!(area(pad + 1) > budget, "{size:?}: {pad} could be larger");
        }
        let small = settings_with(&[]);
        assert_eq!(small.frame_padding([1920, 1080]), small.padding());
    }

    #[test]
    fn aspect_squeezes_the_minor_axis() {
        assert_eq!(
            [stretch(0.0), stretch(0.2), stretch(-1.0)],
            [[1.0, 1.0], [1.0, 0.5], [1.0 / 32.0, 1.0]]
        );
    }
}
