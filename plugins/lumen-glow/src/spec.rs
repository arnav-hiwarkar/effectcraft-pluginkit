use craft_plugin::prelude::*;

pub const BLEND_MODES: [&str; 2] = ["Add", "Screen"];
pub const VIEWS: [&str; 3] = ["Final", "Glow Only", "Glow Source"];

const WARM: [f64; 4] = [1.0, 0.62, 0.28, 1.0];
const EMBER: [f64; 4] = [1.0, 0.32, 0.06, 1.0];

pub fn spec() -> EffectSpec {
    EffectSpec::new("glow", "Lumen Glow", "Stylize")
        .group(
            "Glow",
            [
                Param::slider("radius", "Radius", 120.0, 0.0..=3000.0),
                Param::slider("exposure", "Exposure", 1.0, -8.0..=8.0),
                Param::slider("threshold", "Threshold", 0.0, 0.0..=4.0),
                Param::percent("smoothness", "Threshold Smoothness", 50.0),
                Param::percent("falloff", "Falloff", 50.0),
                Param::slider("aspect", "Aspect", 0.0, -1.0..=1.0),
            ],
        )
        .group(
            "Colour",
            [
                Param::checkbox("tint", "Tint", false),
                Param::color("inner", "Inner Colour", WARM),
                Param::color("outer", "Outer Colour", EMBER),
                Param::percent("mix", "Tint Mix", 100.0),
            ],
        )
        .group("Aberration", [Param::percent("amount", "Amount", 0.0)])
        .group(
            "Output",
            [
                Param::popup("blend", "Blend", &BLEND_MODES, 0),
                Param::popup("view", "View", &VIEWS, 0),
                Param::percent("source_opacity", "Source Opacity", 100.0),
                Param::percent("glow_opacity", "Glow Opacity", 100.0),
                Param::checkbox("gamma", "Gamma Correct", true),
                Param::checkbox("dither", "Dither", true),
            ],
        )
}
