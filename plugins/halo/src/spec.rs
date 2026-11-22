use craft_plugin::prelude::*;

use crate::flare::wgsl_constants;
use crate::looks;

pub const BLENDS: [&str; 2] = ["Add", "Screen"];
pub const ELEMENTS: [&str; 7] = ["Glow", "Rays", "Streak", "Ring", "Ghosts", "Dirt", "Hoop"];

const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];

fn element(extra: Vec<Param>) -> Vec<Param> {
    let mut params = vec![
        Param::checkbox("enable", "Enable", true),
        Param::percent("brightness", "Brightness", 100.0),
        Param::percent("size", "Size", 100.0),
        Param::color("color", "Colour", WHITE),
    ];
    params.extend(extra);
    params
}

pub fn spec() -> EffectSpec {
    let extras: [Vec<Param>; 7] = [
        vec![Param::percent("falloff", "Falloff", 100.0)],
        vec![Param::slider("count", "Ray Count (0 = Look)", 0.0, 0.0..=64.0), Param::percent("sharpness", "Sharpness", 100.0)],
        vec![Param::percent("thickness", "Thickness", 100.0)],
        vec![Param::percent("width", "Width", 100.0), Param::percent("fringe", "Chromatic Fringe", 100.0)],
        vec![
            Param::slider("count", "Ghost Count (0 = Look)", 0.0, 0.0..=16.0),
            Param::slider("blades", "Iris Blades (0 = Look, 1 = Round)", 0.0, 0.0..=12.0),
            Param::angle("iris_rotation", "Iris Rotation", 0.0),
            Param::percent("spread", "Spread", 100.0),
            Param::slider("seed", "Seed", 1.0, 0.0..=9999.0),
        ],
        vec![],
        vec![Param::percent("width", "Width", 100.0)],
    ];
    let spec = EffectSpec::new("flare", "Halo Flares", "Generate")
        .param(Param::popup("look", "Look", &looks::NAMES, 1))
        .param(Param::point("light", "Light Position", [0.3, 0.3]))
        .param(Param::percent("brightness", "Brightness", 100.0))
        .param(Param::percent("scale", "Scale", 100.0))
        .param(Param::color("color", "Colour", WHITE))
        .param(Param::angle("rotation", "Rotation", 0.0));
    let spec = ELEMENTS.iter().zip(extras).fold(spec, |spec, (name, extra)| spec.group(name, element(extra)));
    spec.group(
        "Behaviour",
        [
            Param::checkbox("edge", "Edge Trigger", true),
            Param::slider("edge_width", "Edge Width", 150.0, 0.0..=2000.0),
            Param::percent("shimmer", "Shimmer", 30.0),
            Param::slider("shimmer_speed", "Shimmer Speed", 1.0, 0.0..=20.0),
            Param::checkbox("occlusion", "Occlusion", false),
            Param::layer("occluder", "Occluding Layer"),
            Param::slider("occlusion_radius", "Occlusion Radius", 12.0, 0.0..=500.0),
        ],
    )
    .group("Composite", [Param::popup("blend", "Blend", &BLENDS, 0), Param::checkbox("flare_only", "Flare Only", false)])
    .time_dependent()
    .shader(&format!("{}{}", wgsl_constants(), include_str!("halo.wgsl")))
}
