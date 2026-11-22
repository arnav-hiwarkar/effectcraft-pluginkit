use craft_plugin::prelude::*;

use crate::look;

pub const PRESET: &str = "preset";
pub const CORE_TYPE: &str = "Core/type";
pub const CORE_START: &str = "Core/start";
pub const CORE_END: &str = "Core/end";

params! {
    pub struct CoreControls {
        "Core" {
            pub start: [f64; 2] = point("Core Start", [0.5, 0.8]),
            pub end: [f64; 2] = point("Core End", [0.5, 0.2]),
            pub size: f64 = slider("Core Size", 6.0, 0.0..=200.0).soft(0.5..=60.0).log(),
            pub brightness: f64 = slider("Core Brightness", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
            pub start_size: f64 = percent("Start Size", 100.0),
            pub end_size: f64 = percent("End Size", 100.0),
            pub start_offset: f64 = percent("Start Offset", 0.0),
            pub end_offset: f64 = percent("End Offset", 100.0),
        }
    }
}

params! {
    pub struct GlowControls {
        "Glow" {
            pub colour_source: usize = popup("Colour From", &["Preset", "Custom"], 0),
            pub colour: [f32; 4] = color("Glow Colour", [0.15, 0.45, 1.0, 1.0]),
            pub intensity: f64 = slider("Glow Intensity", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
            pub spread: f64 = slider("Glow Spread", 100.0, 0.0..=400.0),
            pub bias: f64 = slider("Glow Bias", 0.0, -100.0..=100.0),
        }
    }
}

params! {
    pub struct FlickerControls {
        "Flicker" {
            pub intensity: f64 = slider("Flicker Intensity", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
            pub speed: f64 = slider("Flicker Speed", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
        }
    }
}

params! {
    pub struct DistortionControls {
        "Distortion" {
            pub amount: f64 = slider("Amount", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
            pub scale: f64 = slider("Scale", 100.0, 0.0..=1000.0).soft(1.0..=1000.0).log(),
            pub speed: f64 = slider("Speed", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
            pub core: f64 = slider("Core Amount", 100.0, 0.0..=400.0),
            pub glow: f64 = slider("Glow Amount", 100.0, 0.0..=400.0),
        }
    }
}

params! {
    pub struct RenderControls {
        "Render" {
            pub composite: usize = popup("Composite", &["Add", "Screen", "On Transparent", "On Black"], 0),
            pub alpha_boost: f64 = slider("Alpha Boost", 100.0, 0.0..=1000.0).soft(0.0..=400.0),
        }
    }
}

pub fn effect() -> EffectSpec {
    let mut core = CoreControls::params();
    core.insert(0, Param::popup(CORE_TYPE, "Core Type", &["Two Points", "Layer Masks"], 0));
    EffectSpec::new("beam", "Plasma Beam", "Generate")
        .param(Param::popup(PRESET, "Preset", &look::names(), 0))
        .params(core)
        .params(GlowControls::params())
        .params(FlickerControls::params())
        .params(DistortionControls::params())
        .params(RenderControls::params())
        .time_dependent()
}
