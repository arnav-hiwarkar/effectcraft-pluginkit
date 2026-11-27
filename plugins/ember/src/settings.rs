use craft_plugin::Cx;

use crate::spec::Controls;

pub type Vec3 = [f32; 3];

const QUANTUM: f32 = 1.0 / 256.0;
const DEFAULT_FRAME_RATE: f64 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Emitter {
    Point,
    Box,
    Sphere,
    Masks(Vec<Vec<[f32; 2]>>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sim {
    pub emitter: Emitter,
    pub position: Vec3,
    pub extent: Vec3,
    pub rate: f32,
    pub burst: bool,
    pub direction: Vec3,
    pub spread: f32,
    pub speed: f32,
    pub speed_random: f32,
    pub inherit: f32,
    pub seed: u32,
    pub pre_run_frames: u32,
    pub max_particles: u32,
    pub gravity: f32,
    pub wind: Vec3,
    pub drag: f32,
    pub turbulence: f32,
    pub turbulence_scale: f32,
    pub turbulence_evolution: f32,
    pub life: f32,
    pub life_random: f32,
    pub frame_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Disc,
    Streak,
    Spark,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeCurve {
    Constant,
    Shrink,
    Grow,
    Swell,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub shape: Shape,
    pub size: f32,
    pub size_random: f32,
    pub size_curve: SizeCurve,
    pub streak: f32,
    pub colours: [Vec3; 3],
    pub opacity: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub additive: bool,
    pub camera: bool,
    pub shutter: f32,
    pub samples: u32,
    pub spin: f32,
    pub spin_random: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub sim: Sim,
    pub look: Look,
    pub frame: u32,
}

fn finite(value: f64) -> f32 {
    if value.is_finite() { value as f32 } else { 0.0 }
}

fn quantise(value: f32) -> f32 {
    (value / QUANTUM).round() * QUANTUM
}

fn ratio(value: f64) -> f32 {
    finite(value) / 100.0
}

fn to_layer(cx: &Cx, frame_px: [f64; 2]) -> [f32; 2] {
    let scale = cx.scale.max(1e-6);
    [0, 1].map(|axis| quantise(finite((frame_px[axis] - cx.offset[axis]) / scale)))
}

fn colour(c: [f32; 4]) -> Vec3 {
    [c[0], c[1], c[2]].map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
}

pub fn direction(azimuth_deg: f32, elevation_deg: f32) -> Vec3 {
    let (a, e) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    [a.sin() * e.cos(), -a.cos() * e.cos(), -e.sin()]
}

impl Settings {
    pub fn read(cx: &Cx, masks: Vec<Vec<[f64; 2]>>) -> Settings {
        let c = Controls::read(cx);
        let frame_rate = if cx.frame_rate.is_finite() && cx.frame_rate > 0.0 { cx.frame_rate } else { DEFAULT_FRAME_RATE };
        let xy = to_layer(cx, [c.position[0], c.position[1]]);
        let emitter = match c.kind {
            0 => Emitter::Point,
            1 => Emitter::Box,
            2 => Emitter::Sphere,
            _ => Emitter::Masks(masks.into_iter().map(|points| points.into_iter().map(|p| to_layer(cx, p)).collect()).collect()),
        };
        let pre_run = finite(c.pre_run).clamp(0.0, 10.0);
        let sim = Sim {
            emitter,
            position: [xy[0], xy[1], quantise(finite(c.position[2]))],
            extent: [c.size_x, c.size_y, c.size_z].map(|v| finite(v).abs()),
            rate: finite(c.rate).clamp(0.0, 200_000.0),
            burst: c.emission == 1,
            direction: direction(finite(c.direction), finite(c.elevation)),
            spread: ratio(c.spread).clamp(0.0, 1.0),
            speed: finite(c.speed).max(0.0),
            speed_random: ratio(c.speed_random).clamp(0.0, 1.0),
            inherit: ratio(c.inherit),
            seed: finite(c.seed).max(0.0) as u32,
            pre_run_frames: (f64::from(pre_run) * frame_rate).round() as u32,
            max_particles: finite(c.max_particles).clamp(0.0, 500_000.0) as u32,
            gravity: finite(c.gravity),
            wind: [c.wind_x, c.wind_y, c.wind_z].map(finite),
            drag: finite(c.drag).max(0.0),
            turbulence: finite(c.turbulence).max(0.0),
            turbulence_scale: finite(c.turbulence_scale).max(1.0),
            turbulence_evolution: finite(c.turbulence_evolution).max(0.0),
            life: finite(c.life).max(0.01),
            life_random: ratio(c.life_random).clamp(0.0, 1.0),
            frame_rate: frame_rate as f32,
        };
        let look = Look {
            shape: match c.shape {
                0 => Shape::Disc,
                1 => Shape::Streak,
                _ => Shape::Spark,
            },
            size: finite(c.size).max(0.0),
            size_random: ratio(c.size_random).clamp(0.0, 1.0),
            size_curve: match c.size_over_life {
                0 => SizeCurve::Constant,
                1 => SizeCurve::Shrink,
                2 => SizeCurve::Grow,
                _ => SizeCurve::Swell,
            },
            streak: ratio(c.streak).max(0.0),
            colours: [c.colour_birth, c.colour_middle, c.colour_death].map(|rgba| colour(rgba).map(|v| v * finite(c.intensity).max(0.0))),
            opacity: ratio(c.opacity).clamp(0.0, 1.0),
            fade_in: ratio(c.fade_in).clamp(0.0, 1.0),
            fade_out: ratio(c.fade_out).clamp(0.0, 1.0),
            additive: c.blend == 0,
            camera: c.view == 0,
            shutter: if c.motion_blur { finite(c.shutter).clamp(0.0, 720.0) / 360.0 } else { 0.0 },
            samples: if c.motion_blur { finite(c.samples).clamp(1.0, 32.0) as u32 } else { 1 },
            spin: finite(c.spin).to_radians(),
            spin_random: ratio(c.spin_random).clamp(0.0, 1.0),
        };
        let frame = (cx.time * frame_rate).round();
        let frame = if frame.is_finite() { frame.max(-1.0) } else { -1.0 };
        let sim_frame = frame + f64::from(sim.pre_run_frames);
        Settings { sim, look, frame: if sim_frame < 0.0 { u32::MAX } else { sim_frame.min(f64::from(u32::MAX - 1)) as u32 } }
    }
}

pub struct Hash(u64);

impl Hash {
    pub fn new() -> Hash {
        Hash(0xcbf2_9ce4_8422_2325)
    }

    pub fn u32(&mut self, value: u32) -> &mut Hash {
        for byte in value.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        self
    }

    pub fn f32(&mut self, value: f32) -> &mut Hash {
        self.u32(value.to_bits())
    }

    pub fn f32s(&mut self, values: &[f32]) -> &mut Hash {
        values.iter().for_each(|v| {
            self.f32(*v);
        });
        self
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Sim {
    pub fn fixed_hash(&self, history: u64) -> u64 {
        let mut h = Hash::new();
        h.u32((history >> 32) as u32).u32(history as u32);
        self.hash_emitter(&mut h);
        h.u32(u32::from(self.burst)).u32(self.seed).u32(self.max_particles).u32(self.pre_run_frames).f32(self.frame_rate);
        h.finish()
    }

    fn hash_emitter(&self, h: &mut Hash) {
        match &self.emitter {
            Emitter::Point => h.u32(0),
            Emitter::Box => h.u32(1),
            Emitter::Sphere => h.u32(2),
            Emitter::Masks(paths) => {
                h.u32(3).u32(paths.len() as u32);
                paths.iter().for_each(|path| {
                    h.u32(path.len() as u32);
                    path.iter().for_each(|p| {
                        h.f32s(p);
                    });
                });
                h
            }
        };
    }

    pub fn hash(&self) -> u64 {
        let mut h = Hash::new();
        self.hash_emitter(&mut h);
        h.f32s(&self.position).f32s(&self.extent).f32(self.rate).u32(u32::from(self.burst)).f32s(&self.direction);
        h.f32(self.spread).f32(self.speed).f32(self.speed_random).f32(self.inherit).u32(self.seed);
        h.u32(self.max_particles).u32(self.pre_run_frames).f32(self.gravity).f32s(&self.wind).f32(self.drag);
        h.f32(self.turbulence).f32(self.turbulence_scale).f32(self.turbulence_evolution);
        h.f32(self.life).f32(self.life_random).f32(self.frame_rate);
        h.finish()
    }
}
