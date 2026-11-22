use craft_plugin::{EffectSpec, Param};

pub const EMITTER_TYPE: &str = "Emitter/type";
pub const POSITION: &str = "Emitter/position";
pub const EMITTER_SIZE_X: &str = "Emitter/size_x";
pub const EMITTER_SIZE_Y: &str = "Emitter/size_y";
pub const EMITTER_SIZE_Z: &str = "Emitter/size_z";
pub const EMITTER_SPEED_X: &str = "Emitter/motion_x";
pub const EMITTER_SPEED_Y: &str = "Emitter/motion_y";
pub const EMITTER_SPEED_Z: &str = "Emitter/motion_z";
pub const RATE: &str = "Emitter/rate";
pub const EMISSION: &str = "Emitter/emission";
pub const DIRECTION: &str = "Emitter/direction";
pub const ELEVATION: &str = "Emitter/elevation";
pub const SPREAD: &str = "Emitter/spread";
pub const SPEED: &str = "Emitter/speed";
pub const SPEED_RANDOM: &str = "Emitter/speed_random";
pub const INHERIT: &str = "Emitter/inherit";
pub const SEED: &str = "Emitter/seed";
pub const PRE_RUN: &str = "Emitter/pre_run";
pub const MAX_PARTICLES: &str = "Emitter/max_particles";

pub const GRAVITY: &str = "Physics/gravity";
pub const WIND_X: &str = "Physics/wind_x";
pub const WIND_Y: &str = "Physics/wind_y";
pub const WIND_Z: &str = "Physics/wind_z";
pub const DRAG: &str = "Physics/drag";
pub const TURBULENCE: &str = "Physics/turbulence";
pub const TURBULENCE_SCALE: &str = "Physics/turbulence_scale";
pub const TURBULENCE_EVOLUTION: &str = "Physics/turbulence_evolution";
pub const SPIN: &str = "Physics/spin";
pub const SPIN_RANDOM: &str = "Physics/spin_random";

pub const LIFE: &str = "Particle/life";
pub const LIFE_RANDOM: &str = "Particle/life_random";
pub const SHAPE: &str = "Particle/shape";
pub const SIZE: &str = "Particle/size";
pub const SIZE_RANDOM: &str = "Particle/size_random";
pub const SIZE_OVER_LIFE: &str = "Particle/size_over_life";
pub const STREAK: &str = "Particle/streak";
pub const COLOUR_BIRTH: &str = "Particle/colour_birth";
pub const COLOUR_MIDDLE: &str = "Particle/colour_middle";
pub const COLOUR_DEATH: &str = "Particle/colour_death";
pub const INTENSITY: &str = "Particle/intensity";
pub const OPACITY: &str = "Particle/opacity";
pub const FADE_IN: &str = "Particle/fade_in";
pub const FADE_OUT: &str = "Particle/fade_out";

pub const BLEND: &str = "Render/blend";
pub const VIEW: &str = "Render/view";
pub const MOTION_BLUR: &str = "Render/motion_blur";
pub const SHUTTER: &str = "Render/shutter";
pub const SAMPLES: &str = "Render/samples";

pub const EMISSIONS: [&str; 2] = ["Continuous", "Burst"];
pub const EMITTER_TYPES: [&str; 4] = ["Point", "Box", "Sphere", "Layer Masks"];
pub const SHAPES: [&str; 3] = ["Soft Disc", "Streak", "Spark"];
pub const SIZE_CURVES: [&str; 4] = ["Constant", "Shrink", "Grow", "Swell"];
pub const BLENDS: [&str; 2] = ["Add", "Normal"];
pub const VIEWS: [&str; 2] = ["Comp Camera", "Flat"];

fn key(id: &str) -> &str {
    id.rsplit_once('/').map_or(id, |(_, key)| key)
}

fn slider(id: &str, name: &str, default: f64, min: f64, max: f64) -> Param {
    Param::slider(key(id), name, default, min..=max)
}

fn percent(id: &str, name: &str, default: f64) -> Param {
    Param::percent(key(id), name, default)
}

pub fn effect() -> EffectSpec {
    EffectSpec::new("ember", "Ember", "Simulation")
        .group(
            "Emitter",
            [
                Param::popup(key(EMITTER_TYPE), "Emitter Type", &EMITTER_TYPES, 0),
                Param::point3(key(POSITION), "Position", [0.5, 0.7, 0.0]),
                slider(EMITTER_SIZE_X, "Size X", 100.0, 0.0, 10000.0),
                slider(EMITTER_SIZE_Y, "Size Y", 100.0, 0.0, 10000.0),
                slider(EMITTER_SIZE_Z, "Size Z", 100.0, 0.0, 10000.0),
                slider(EMITTER_SPEED_X, "Emitter Motion X", 0.0, -10000.0, 10000.0),
                slider(EMITTER_SPEED_Y, "Emitter Motion Y", 0.0, -10000.0, 10000.0),
                slider(EMITTER_SPEED_Z, "Emitter Motion Z", 0.0, -10000.0, 10000.0),
                Param::popup(key(EMISSION), "Emission", &EMISSIONS, 0),
                slider(RATE, "Particles/sec", 300.0, 0.0, 200000.0),
                Param::angle(key(DIRECTION), "Direction", 0.0),
                Param::angle(key(ELEVATION), "Elevation", 0.0),
                percent(SPREAD, "Spread", 15.0),
                slider(SPEED, "Velocity", 400.0, 0.0, 20000.0),
                percent(SPEED_RANDOM, "Velocity Random", 30.0),
                percent(INHERIT, "Inherit Emitter Motion", 0.0),
                slider(SEED, "Random Seed", 1.0, 0.0, 100000.0),
                slider(PRE_RUN, "Pre-run (sec)", 0.0, 0.0, 10.0),
                slider(MAX_PARTICLES, "Max Particles", 100000.0, 0.0, 500000.0),
            ],
        )
        .group(
            "Physics",
            [
                slider(GRAVITY, "Gravity", 300.0, -10000.0, 10000.0),
                slider(WIND_X, "Wind X", 0.0, -10000.0, 10000.0),
                slider(WIND_Y, "Wind Y", 0.0, -10000.0, 10000.0),
                slider(WIND_Z, "Wind Z", 0.0, -10000.0, 10000.0),
                slider(DRAG, "Air Resistance", 0.3, 0.0, 50.0),
                slider(TURBULENCE, "Turbulence", 0.0, 0.0, 10000.0),
                slider(TURBULENCE_SCALE, "Turbulence Scale", 200.0, 1.0, 10000.0),
                slider(TURBULENCE_EVOLUTION, "Turbulence Evolution", 0.5, 0.0, 50.0),
                slider(SPIN, "Spin (deg/sec)", 0.0, -3600.0, 3600.0),
                percent(SPIN_RANDOM, "Spin Random", 50.0),
            ],
        )
        .group(
            "Particle",
            [
                slider(LIFE, "Life (sec)", 2.0, 0.01, 60.0),
                percent(LIFE_RANDOM, "Life Random", 30.0),
                Param::popup(key(SHAPE), "Shape", &SHAPES, 0),
                slider(SIZE, "Size", 6.0, 0.0, 1000.0),
                percent(SIZE_RANDOM, "Size Random", 40.0),
                Param::popup(key(SIZE_OVER_LIFE), "Size Over Life", &SIZE_CURVES, 1),
                percent(STREAK, "Streak Length", 100.0),
                Param::color(key(COLOUR_BIRTH), "Colour at Birth", [1.0, 0.95, 0.7, 1.0]),
                Param::color(key(COLOUR_MIDDLE), "Colour at Middle", [1.0, 0.45, 0.08, 1.0]),
                Param::color(key(COLOUR_DEATH), "Colour at Death", [0.35, 0.03, 0.0, 1.0]),
                slider(INTENSITY, "Intensity", 1.0, 0.0, 100.0),
                percent(OPACITY, "Opacity", 100.0),
                percent(FADE_IN, "Fade In", 5.0),
                percent(FADE_OUT, "Fade Out", 40.0),
            ],
        )
        .group(
            "Render",
            [
                Param::popup(key(BLEND), "Blend", &BLENDS, 0),
                Param::popup(key(VIEW), "View", &VIEWS, 0),
                Param::checkbox(key(MOTION_BLUR), "Motion Blur", true),
                slider(SHUTTER, "Shutter Angle", 180.0, 0.0, 720.0),
                slider(SAMPLES, "Samples", 6.0, 1.0, 32.0),
            ],
        )
        .time_dependent()
        .reads_scene()
}
