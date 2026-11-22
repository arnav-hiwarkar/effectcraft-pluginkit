use craft_plugin::Value;
use craft_plugin::prelude::*;

use crate::flare::wgsl_constants;
use crate::looks;

pub const BLENDS: [&str; 2] = ["Add", "Screen"];

const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];

pub struct Own {
    pub enable: bool,
    pub brightness: f64,
    pub size: f64,
    pub colour: [f32; 4],
}

macro_rules! element {
    ($name:ident, $group:literal { $($extra:tt)* }) => {
        params! {
            pub struct $name {
                $group {
                    pub enable: bool = checkbox("Enable", true),
                    pub brightness: f64 = percent("Brightness", 100.0),
                    pub size: f64 = percent("Size", 100.0),
                    pub color: [f32; 4] = color("Colour", WHITE),
                    $($extra)*
                }
            }
        }

        impl $name {
            pub fn own(&self) -> Own {
                Own { enable: self.enable, brightness: self.brightness, size: self.size, colour: self.color }
            }
        }
    };
}

params! {
    pub struct Overall {
        pub look: usize = popup("Look", &looks::NAMES, 1),
        pub light: [f64; 2] = point("Light Position", [0.3, 0.3]),
        pub brightness: f64 = percent("Brightness", 100.0),
        pub scale: f64 = percent("Scale", 100.0),
        pub color: [f32; 4] = color("Colour", WHITE),
        pub rotation: f64 = angle("Rotation", 0.0),
    }
}

element!(Glow, "Glow" { pub falloff: f64 = percent("Falloff", 100.0), });
element!(Rays, "Rays" {
    pub count: f64 = slider("Ray Count (0 = Look)", 0.0, 0.0..=64.0),
    pub sharpness: f64 = percent("Sharpness", 100.0),
});
element!(Streak, "Streak" { pub thickness: f64 = percent("Thickness", 100.0), });
element!(Ring, "Ring" {
    pub width: f64 = percent("Width", 100.0),
    pub fringe: f64 = percent("Chromatic Fringe", 100.0),
});
element!(Ghosts, "Ghosts" {
    pub count: f64 = slider("Ghost Count (0 = Look)", 0.0, 0.0..=16.0),
    pub blades: f64 = slider("Iris Blades (0 = Look, 1 = Round)", 0.0, 0.0..=12.0),
    pub iris_rotation: f64 = angle("Iris Rotation", 0.0),
    pub spread: f64 = percent("Spread", 100.0),
    pub seed: f64 = slider("Seed", 1.0, 0.0..=9999.0),
});
element!(Dirt, "Dirt" {});
element!(Hoop, "Hoop" { pub width: f64 = percent("Width", 100.0), });

params! {
    pub struct Behaviour {
        "Behaviour" {
            pub edge: bool = checkbox("Edge Trigger", true),
            pub edge_width: f64 = slider("Edge Width", 150.0, 0.0..=2000.0).soft(0.0..=500.0),
            pub shimmer: f64 = percent("Shimmer", 30.0),
            pub shimmer_speed: f64 = slider("Shimmer Speed", 1.0, 0.0..=20.0),
            pub occlusion: bool = checkbox("Occlusion", false),
            pub occluder: LayerParam = layer("Occluding Layer"),
            pub occlusion_radius: f64 = slider("Occlusion Radius", 12.0, 0.0..=500.0).soft(1.0..=500.0).log(),
        }
    }
}

params! {
    pub struct Composite {
        "Composite" {
            pub blend: usize = popup("Blend", &BLENDS, 0),
            pub flare_only: bool = checkbox("Flare Only", false),
        }
    }
}

pub fn spec() -> EffectSpec {
    let spec = EffectSpec::new("flare", "Halo Flares", "Generate")
        .params(Overall::params())
        .params(Glow::params())
        .params(Rays::params())
        .params(Streak::params())
        .params(Ring::params())
        .params(Ghosts::params())
        .params(Dirt::params())
        .params(Hoop::params())
        .params(Behaviour::params())
        .params(Composite::params())
        .time_dependent()
        .shader(&format!("{}{}", wgsl_constants(), include_str!("halo.wgsl")));
    with_look_presets(spec)
}

fn with_look_presets(spec: EffectSpec) -> EffectSpec {
    (1..looks::NAMES.len()).fold(spec, |spec, index| {
        let look = looks::look(index);
        let counts = [("look", index as u32), ("Rays/count", look.ray_count), ("Ghosts/count", look.ghost_count), ("Ghosts/blades", look.blades)];
        spec.preset(looks::NAMES[index], counts.map(|(id, count)| (id, Value::Number(f64::from(count)))))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn preset(name: &str) -> BTreeMap<String, Value> {
        spec().presets.into_iter().find(|preset| preset.name == name).map(|preset| preset.values).expect("the preset exists")
    }

    #[test]
    fn a_look_preset_writes_the_look_and_its_counts_and_leaves_the_element_controls_alone() {
        let sun: BTreeMap<String, Value> = [("look", 1.0), ("Rays/count", 14.0), ("Ghosts/count", 7.0), ("Ghosts/blades", 6.0)]
            .into_iter()
            .map(|(id, value)| (id.to_string(), Value::Number(value)))
            .collect();
        assert_eq!(preset("Sun"), sun);
        assert_eq!(preset("Night Street").get("Rays/count"), Some(&Value::Number(16.0)));
        assert_eq!(spec().presets.len(), 6);
    }
}
