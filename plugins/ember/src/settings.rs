use craft_plugin::Cx;

use crate::spec::*;

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
    pub motion: Vec3,
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

struct Reader<'a>(&'a Cx);

impl Reader<'_> {
    fn number(&self, id: &str) -> f32 {
        finite(self.0.f(id))
    }

    fn ratio(&self, id: &str) -> f32 {
        self.number(id) / 100.0
    }

    fn to_layer(&self, frame_px: [f64; 2]) -> [f32; 2] {
        let scale = self.0.scale.max(1e-6);
        [0, 1].map(|axis| quantise(finite((frame_px[axis] - self.0.offset[axis]) / scale)))
    }

    fn vec3(&self, ids: [&str; 3]) -> Vec3 {
        ids.map(|id| self.number(id))
    }

    fn colour(&self, id: &str) -> Vec3 {
        let c = self.0.color(id);
        [c[0], c[1], c[2]].map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
    }
}

pub fn direction(azimuth_deg: f32, elevation_deg: f32) -> Vec3 {
    let (a, e) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    [a.sin() * e.cos(), -a.cos() * e.cos(), -e.sin()]
}

impl Settings {
    pub fn read(cx: &Cx, masks: Vec<Vec<[f64; 2]>>) -> Settings {
        let read = Reader(cx);
        let frame_rate = if cx.frame_rate.is_finite() && cx.frame_rate > 0.0 { cx.frame_rate } else { DEFAULT_FRAME_RATE };
        let position3 = cx.point3(POSITION);
        let xy = read.to_layer([position3[0], position3[1]]);
        let emitter = match cx.index(EMITTER_TYPE) {
            0 => Emitter::Point,
            1 => Emitter::Box,
            2 => Emitter::Sphere,
            _ => Emitter::Masks(masks.into_iter().map(|points| points.into_iter().map(|p| read.to_layer(p)).collect()).collect()),
        };
        let pre_run = read.number(PRE_RUN).clamp(0.0, 10.0);
        let sim = Sim {
            emitter,
            position: [xy[0], xy[1], quantise(finite(position3[2]))],
            extent: read.vec3([EMITTER_SIZE_X, EMITTER_SIZE_Y, EMITTER_SIZE_Z]).map(f32::abs),
            motion: read.vec3([EMITTER_SPEED_X, EMITTER_SPEED_Y, EMITTER_SPEED_Z]),
            rate: read.number(RATE).clamp(0.0, 200_000.0),
            burst: cx.index(EMISSION) == 1,
            direction: direction(read.number(DIRECTION), read.number(ELEVATION)),
            spread: read.ratio(SPREAD).clamp(0.0, 1.0),
            speed: read.number(SPEED).max(0.0),
            speed_random: read.ratio(SPEED_RANDOM).clamp(0.0, 1.0),
            inherit: read.ratio(INHERIT),
            seed: read.number(SEED).max(0.0) as u32,
            pre_run_frames: (f64::from(pre_run) * frame_rate).round() as u32,
            max_particles: read.number(MAX_PARTICLES).clamp(0.0, 500_000.0) as u32,
            gravity: read.number(GRAVITY),
            wind: read.vec3([WIND_X, WIND_Y, WIND_Z]),
            drag: read.number(DRAG).max(0.0),
            turbulence: read.number(TURBULENCE).max(0.0),
            turbulence_scale: read.number(TURBULENCE_SCALE).max(1.0),
            turbulence_evolution: read.number(TURBULENCE_EVOLUTION).max(0.0),
            life: read.number(LIFE).max(0.01),
            life_random: read.ratio(LIFE_RANDOM).clamp(0.0, 1.0),
            frame_rate: frame_rate as f32,
        };
        let look = Look {
            shape: match cx.index(SHAPE) {
                0 => Shape::Disc,
                1 => Shape::Streak,
                _ => Shape::Spark,
            },
            size: read.number(SIZE).max(0.0),
            size_random: read.ratio(SIZE_RANDOM).clamp(0.0, 1.0),
            size_curve: match cx.index(SIZE_OVER_LIFE) {
                0 => SizeCurve::Constant,
                1 => SizeCurve::Shrink,
                2 => SizeCurve::Grow,
                _ => SizeCurve::Swell,
            },
            streak: read.ratio(STREAK).max(0.0),
            colours: [COLOUR_BIRTH, COLOUR_MIDDLE, COLOUR_DEATH].map(|id| read.colour(id).map(|c| c * read.number(INTENSITY).max(0.0))),
            opacity: read.ratio(OPACITY).clamp(0.0, 1.0),
            fade_in: read.ratio(FADE_IN).clamp(0.0, 1.0),
            fade_out: read.ratio(FADE_OUT).clamp(0.0, 1.0),
            additive: cx.index(BLEND) == 0,
            camera: cx.index(VIEW) == 0,
            shutter: if cx.bool(MOTION_BLUR) { read.number(SHUTTER).clamp(0.0, 720.0) / 360.0 } else { 0.0 },
            samples: if cx.bool(MOTION_BLUR) { read.number(SAMPLES).clamp(1.0, 32.0) as u32 } else { 1 },
            spin: read.number(SPIN).to_radians(),
            spin_random: read.ratio(SPIN_RANDOM).clamp(0.0, 1.0),
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
    pub fn hash(&self) -> u64 {
        let mut h = Hash::new();
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
                &mut h
            }
        };
        h.f32s(&self.position).f32s(&self.extent).f32s(&self.motion).f32(self.rate).u32(u32::from(self.burst)).f32s(&self.direction);
        h.f32(self.spread).f32(self.speed).f32(self.speed_random).f32(self.inherit).u32(self.seed);
        h.u32(self.max_particles).u32(self.pre_run_frames).f32(self.gravity).f32s(&self.wind).f32(self.drag);
        h.f32(self.turbulence).f32(self.turbulence_scale).f32(self.turbulence_evolution);
        h.f32(self.life).f32(self.life_random).f32(self.frame_rate);
        h.finish()
    }
}
