use craft_plugin::{EffectSpec, Param};

use crate::look;

pub const PRESET: &str = "preset";
pub const CORE_TYPE: &str = "Core/type";
pub const CORE_START: &str = "Core/start";
pub const CORE_END: &str = "Core/end";
pub const CORE_SIZE: &str = "Core/size";
pub const CORE_BRIGHTNESS: &str = "Core/brightness";
pub const START_SIZE: &str = "Core/start_size";
pub const END_SIZE: &str = "Core/end_size";
pub const START_OFFSET: &str = "Core/start_offset";
pub const END_OFFSET: &str = "Core/end_offset";
pub const COLOUR_SOURCE: &str = "Glow/colour_source";
pub const GLOW_COLOUR: &str = "Glow/colour";
pub const GLOW_INTENSITY: &str = "Glow/intensity";
pub const GLOW_SPREAD: &str = "Glow/spread";
pub const GLOW_BIAS: &str = "Glow/bias";
pub const FLICKER_INTENSITY: &str = "Flicker/intensity";
pub const FLICKER_SPEED: &str = "Flicker/speed";
pub const DISTORTION_AMOUNT: &str = "Distortion/amount";
pub const DISTORTION_SCALE: &str = "Distortion/scale";
pub const DISTORTION_SPEED: &str = "Distortion/speed";
pub const DISTORTION_CORE: &str = "Distortion/core";
pub const DISTORTION_GLOW: &str = "Distortion/glow";
pub const COMPOSITE: &str = "Render/composite";
pub const ALPHA_BOOST: &str = "Render/alpha_boost";

fn key(id: &str) -> &str {
    id.rsplit_once('/').map_or(id, |(_, key)| key)
}

fn slider(id: &str, name: &str, default: f64, max: f64) -> Param {
    Param::slider(key(id), name, default, 0.0..=max)
}

fn percent(id: &str, name: &str, default: f64) -> Param {
    Param::percent(key(id), name, default)
}

pub fn effect() -> EffectSpec {
    EffectSpec::new("beam", "Plasma Beam", "Generate")
        .param(Param::popup(PRESET, "Preset", &look::names(), 0))
        .group(
            "Core",
            [
                Param::popup(key(CORE_TYPE), "Core Type", &["Two Points", "Layer Masks"], 0),
                Param::point(key(CORE_START), "Core Start", [0.5, 0.8]),
                Param::point(key(CORE_END), "Core End", [0.5, 0.2]),
                slider(CORE_SIZE, "Core Size", 6.0, 200.0),
                slider(CORE_BRIGHTNESS, "Core Brightness", 100.0, 1000.0),
                percent(START_SIZE, "Start Size", 100.0),
                percent(END_SIZE, "End Size", 100.0),
                percent(START_OFFSET, "Start Offset", 0.0),
                percent(END_OFFSET, "End Offset", 100.0),
            ],
        )
        .group(
            "Glow",
            [
                Param::popup(key(COLOUR_SOURCE), "Colour From", &["Preset", "Custom"], 0),
                Param::color(key(GLOW_COLOUR), "Glow Colour", [0.15, 0.45, 1.0, 1.0]),
                slider(GLOW_INTENSITY, "Glow Intensity", 100.0, 1000.0),
                slider(GLOW_SPREAD, "Glow Spread", 100.0, 400.0),
                Param::slider(key(GLOW_BIAS), "Glow Bias", 0.0, -100.0..=100.0),
            ],
        )
        .group("Flicker", [slider(FLICKER_INTENSITY, "Flicker Intensity", 100.0, 1000.0), slider(FLICKER_SPEED, "Flicker Speed", 100.0, 1000.0)])
        .group(
            "Distortion",
            [
                slider(DISTORTION_AMOUNT, "Amount", 100.0, 1000.0),
                slider(DISTORTION_SCALE, "Scale", 100.0, 1000.0),
                slider(DISTORTION_SPEED, "Speed", 100.0, 1000.0),
                slider(DISTORTION_CORE, "Core Amount", 100.0, 400.0),
                slider(DISTORTION_GLOW, "Glow Amount", 100.0, 400.0),
            ],
        )
        .group(
            "Render",
            [Param::popup(key(COMPOSITE), "Composite", &["Add", "Screen", "On Transparent", "On Black"], 0), slider(ALPHA_BOOST, "Alpha Boost", 100.0, 1000.0)],
        )
        .time_dependent()
}
