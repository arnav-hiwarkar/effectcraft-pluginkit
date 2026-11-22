use craft_plugin::prelude::*;

pub const EMITTER_TYPE: &str = "Emitter/type";

pub const EMISSIONS: [&str; 2] = ["Continuous", "Burst"];
pub const EMITTER_TYPES: [&str; 4] = ["Point", "Box", "Sphere", "Layer Masks"];
pub const SHAPES: [&str; 3] = ["Soft Disc", "Streak", "Spark"];
pub const SIZE_CURVES: [&str; 4] = ["Constant", "Shrink", "Grow", "Swell"];
pub const BLENDS: [&str; 2] = ["Add", "Normal"];
pub const VIEWS: [&str; 2] = ["Comp Camera", "Flat"];

params! {
    pub struct Controls {
        "Emitter" {
            pub position: [f64; 3] = point3("Position", [0.5, 0.7, 0.0]),
            pub size_x: f64 = slider("Size X", 100.0, 0.0..=10000.0).soft(0.0..=1000.0),
            pub size_y: f64 = slider("Size Y", 100.0, 0.0..=10000.0).soft(0.0..=1000.0),
            pub size_z: f64 = slider("Size Z", 100.0, 0.0..=10000.0).soft(0.0..=1000.0),
            pub emission: usize = popup("Emission", &EMISSIONS, 0),
            pub rate: f64 = slider("Particles/sec", 300.0, 0.0..=200000.0).soft(1.0..=20000.0).log(),
            pub direction: f64 = angle("Direction", 0.0),
            pub elevation: f64 = angle("Elevation", 0.0),
            pub spread: f64 = percent("Spread", 15.0),
            pub speed: f64 = slider("Velocity", 400.0, 0.0..=20000.0).soft(1.0..=3000.0).log(),
            pub speed_random: f64 = percent("Velocity Random", 30.0),
            pub inherit: f64 = percent("Inherit Emitter Motion", 0.0),
            pub seed: f64 = slider("Random Seed", 1.0, 0.0..=100000.0),
            pub pre_run: f64 = slider("Pre-run (sec)", 0.0, 0.0..=10.0),
            pub max_particles: f64 = slider("Max Particles", 100000.0, 0.0..=500000.0).soft(100.0..=500000.0).log(),
        }
        "Physics" {
            pub gravity: f64 = slider("Gravity", 300.0, -10000.0..=10000.0).soft(-2000.0..=2000.0),
            pub wind_x: f64 = slider("Wind X", 0.0, -10000.0..=10000.0).soft(-2000.0..=2000.0),
            pub wind_y: f64 = slider("Wind Y", 0.0, -10000.0..=10000.0).soft(-2000.0..=2000.0),
            pub wind_z: f64 = slider("Wind Z", 0.0, -10000.0..=10000.0).soft(-2000.0..=2000.0),
            pub drag: f64 = slider("Air Resistance", 0.3, 0.0..=50.0).soft(0.0..=5.0),
            pub turbulence: f64 = slider("Turbulence", 0.0, 0.0..=10000.0).soft(0.0..=1000.0),
            pub turbulence_scale: f64 = slider("Turbulence Scale", 200.0, 1.0..=10000.0).log(),
            pub turbulence_evolution: f64 = slider("Turbulence Evolution", 0.5, 0.0..=50.0).soft(0.0..=5.0),
            pub spin: f64 = slider("Spin (deg/sec)", 0.0, -3600.0..=3600.0).soft(-720.0..=720.0),
            pub spin_random: f64 = percent("Spin Random", 50.0),
        }
        "Particle" {
            pub life: f64 = slider("Life (sec)", 2.0, 0.01..=60.0).log(),
            pub life_random: f64 = percent("Life Random", 30.0),
            pub shape: usize = popup("Shape", &SHAPES, 0),
            pub size: f64 = slider("Size", 6.0, 0.0..=1000.0).soft(0.5..=200.0).log(),
            pub size_random: f64 = percent("Size Random", 40.0),
            pub size_over_life: usize = popup("Size Over Life", &SIZE_CURVES, 1),
            pub streak: f64 = percent("Streak Length", 100.0),
            pub colour_birth: [f32; 4] = color("Colour at Birth", [1.0, 0.95, 0.7, 1.0]),
            pub colour_middle: [f32; 4] = color("Colour at Middle", [1.0, 0.45, 0.08, 1.0]),
            pub colour_death: [f32; 4] = color("Colour at Death", [0.35, 0.03, 0.0, 1.0]),
            pub intensity: f64 = slider("Intensity", 1.0, 0.0..=100.0).soft(0.0..=10.0),
            pub opacity: f64 = percent("Opacity", 100.0),
            pub fade_in: f64 = percent("Fade In", 5.0),
            pub fade_out: f64 = percent("Fade Out", 40.0),
        }
        "Render" {
            pub blend: usize = popup("Blend", &BLENDS, 0),
            pub view: usize = popup("View", &VIEWS, 0),
            pub motion_blur: bool = checkbox("Motion Blur", true),
            pub shutter: f64 = slider("Shutter Angle", 180.0, 0.0..=720.0),
            pub samples: f64 = slider("Samples", 6.0, 1.0..=32.0),
        }
    }
}

pub fn effect() -> EffectSpec {
    let mut controls = Controls::params();
    controls.insert(0, Param::popup(EMITTER_TYPE, "Emitter Type", &EMITTER_TYPES, 0));
    EffectSpec::new("ember", "Ember", "Simulation").params(controls).time_dependent().reads_param_history().reads_scene()
}
