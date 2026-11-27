use crate::random::Stream;
use crate::settings::{Look, Shape, SizeCurve, Vec3};
use crate::sim::Particle;

pub const BIN_ROWS: usize = 16;
const MIN_RADIUS: f32 = 0.75;
const FLOATS: usize = 12;
pub const SPARK_LENGTH: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub centre: [f32; 2],
    pub blur: [f32; 2],
    pub streak: [f32; 2],
    pub radius: f32,
    pub angle: f32,
    pub colour: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub sprites: Vec<Sprite>,
    pub bin_starts: Vec<u32>,
    pub bin_items: Vec<u32>,
    pub samples: u32,
    pub shape: Shape,
    pub additive: bool,
}

pub struct View {
    pub projection: Option<[[f64; 4]; 3]>,
    pub scale: f32,
    pub offset: [f32; 2],
    pub size: [usize; 2],
    pub frame_rate: f32,
}

struct Projected {
    xy: [f32; 2],
    depth: f32,
    zoom: f32,
}

impl View {
    fn project(&self, p: Vec3) -> Option<Projected> {
        let Some(m) = self.projection else {
            return Some(Projected { xy: [p[0] * self.scale + self.offset[0], p[1] * self.scale + self.offset[1]], depth: p[2], zoom: self.scale });
        };
        let q = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        let row = |r: [f64; 4], d: f64| r[0] * (q[0] + d) + r[1] * q[1] + r[2] * q[2] + r[3];
        let w = row(m[2], 0.0);
        if w <= 1e-6 {
            return None;
        }
        let x = row(m[0], 0.0) / w;
        let y = row(m[1], 0.0) / w;
        let w1 = row(m[2], 1.0);
        let (x1, y1) = (row(m[0], 1.0) / w1, row(m[1], 1.0) / w1);
        let zoom = ((x1 - x).powi(2) + (y1 - y).powi(2)).sqrt() as f32 * self.scale;
        Some(Projected { xy: [x as f32 * self.scale + self.offset[0], y as f32 * self.scale + self.offset[1]], depth: w as f32, zoom })
    }
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

pub fn colour_at(look: &Look, t: f32) -> Vec3 {
    if t < 0.5 { lerp3(look.colours[0], look.colours[1], t * 2.0) } else { lerp3(look.colours[1], look.colours[2], t * 2.0 - 1.0) }
}

pub fn opacity_at(look: &Look, t: f32) -> f32 {
    let fade_in = if look.fade_in > 0.0 { (t / look.fade_in).min(1.0) } else { 1.0 };
    let fade_out = if look.fade_out > 0.0 { ((1.0 - t) / look.fade_out).min(1.0) } else { 1.0 };
    look.opacity * fade_in.min(fade_out).max(0.0)
}

pub fn size_at(curve: SizeCurve, t: f32) -> f32 {
    match curve {
        SizeCurve::Constant => 1.0,
        SizeCurve::Shrink => 1.0 - t,
        SizeCurve::Grow => t,
        SizeCurve::Swell => 1.0 - (2.0 * t - 1.0).powi(2),
    }
}

fn sprite(look: &Look, view: &View, p: &Particle) -> Option<(f32, Sprite)> {
    let t = (p.age / p.life).clamp(0.0, 1.0);
    let mut rng = Stream::new(p.seed);
    let size = look.size * (1.0 + look.size_random * rng.signed()) * size_at(look.size_curve, t);
    let alpha = opacity_at(look, t);
    if size <= 0.0 || alpha <= 0.0 {
        return None;
    }
    let spin = look.spin * (1.0 + look.spin_random * rng.signed());
    let angle = rng.next() * std::f32::consts::TAU + spin * p.age;
    let here = view.project(p.position)?;
    let frame_time = 1.0 / view.frame_rate;
    let back = |seconds: f32| {
        let q = [0, 1, 2].map(|a| p.position[a] - p.velocity[a] * seconds);
        view.project(q).map_or([0.0; 2], |b| [b.xy[0] - here.xy[0], b.xy[1] - here.xy[1]])
    };
    let blur = if look.shutter > 0.0 && look.samples > 1 { back(look.shutter * frame_time) } else { [0.0; 2] };
    let streak = if look.shape == Shape::Disc { [0.0; 2] } else { back(look.streak * frame_time) };
    let mut radius = 0.5 * size * here.zoom;
    let mut gain = alpha;
    if radius < MIN_RADIUS {
        gain *= (radius / MIN_RADIUS).powi(2);
        radius = MIN_RADIUS;
    }
    let colour = colour_at(look, t);
    let premultiplied = [colour[0] * gain, colour[1] * gain, colour[2] * gain, gain];
    if look.shape == Shape::Spark {
        radius *= SPARK_LENGTH;
    }
    Some((here.depth, Sprite { centre: here.xy, blur, streak, radius, angle, colour: premultiplied }))
}

impl Sprite {
    pub fn rows(&self) -> (f32, f32) {
        let ys = [0.0, self.blur[1], self.streak[1], self.blur[1] + self.streak[1]].map(|d| self.centre[1] + d);
        let reach = self.radius + 1.0;
        (ys.iter().copied().fold(f32::MAX, f32::min) - reach, ys.iter().copied().fold(f32::MIN, f32::max) + reach)
    }

    pub fn columns(&self) -> (f32, f32) {
        let xs = [0.0, self.blur[0], self.streak[0], self.blur[0] + self.streak[0]].map(|d| self.centre[0] + d);
        let reach = self.radius + 1.0;
        (xs.iter().copied().fold(f32::MAX, f32::min) - reach, xs.iter().copied().fold(f32::MIN, f32::max) + reach)
    }
}

pub fn build(look: &Look, view: &View, particles: &[Particle]) -> DrawList {
    let mut placed: Vec<(f32, u32, Sprite)> =
        particles.iter().enumerate().filter_map(|(i, p)| sprite(look, view, p).map(|(depth, s)| (depth, i as u32, s))).collect();
    let height = view.size[1] as f32;
    let width = view.size[0] as f32;
    placed.retain(|(_, _, s)| {
        let (top, bottom) = s.rows();
        let (left, right) = s.columns();
        bottom >= 0.0 && top < height && right >= 0.0 && left < width && s.centre.iter().all(|v| v.is_finite())
    });
    if !look.additive {
        placed.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    let sprites: Vec<Sprite> = placed.into_iter().map(|(_, _, s)| s).collect();
    let bins = view.size[1].div_ceil(BIN_ROWS).max(1);
    let span = |s: &Sprite| {
        let (top, bottom) = s.rows();
        let first = (top.max(0.0) as usize / BIN_ROWS).min(bins - 1);
        let last = (bottom.clamp(0.0, height - 1.0).max(0.0) as usize / BIN_ROWS).min(bins - 1);
        first..=last
    };
    let mut counts = vec![0u32; bins + 1];
    sprites.iter().for_each(|s| span(s).for_each(|b| counts[b + 1] += 1));
    for b in 0..bins {
        counts[b + 1] += counts[b];
    }
    let mut fill = counts.clone();
    let mut items = vec![0u32; counts[bins] as usize];
    for (i, s) in sprites.iter().enumerate() {
        for b in span(s) {
            items[fill[b] as usize] = i as u32;
            fill[b] += 1;
        }
    }
    DrawList { sprites, bin_starts: counts, bin_items: items, samples: look.samples.max(1), shape: look.shape, additive: look.additive }
}

impl DrawList {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.sprites.len() * FLOATS * 4 + (self.bin_starts.len() + self.bin_items.len()) * 4);
        let shape = match self.shape {
            Shape::Disc => 0u32,
            Shape::Streak => 1,
            Shape::Spark => 2,
        };
        for word in
            [self.sprites.len() as u32, self.bin_starts.len() as u32, self.bin_items.len() as u32, self.samples, shape | (u32::from(self.additive) << 8)]
        {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for s in &self.sprites {
            for v in s.centre.iter().chain(&s.blur).chain(&s.streak).chain([&s.radius, &s.angle]).chain(&s.colour) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for w in self.bin_starts.iter().chain(&self.bin_items) {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<DrawList> {
        let words: Vec<u32> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let [count, starts, items, samples, flags] = *words.get(..5)? else { return None };
        let (count, starts, items) = (count as usize, starts as usize, items as usize);
        if !bytes.len().is_multiple_of(4) || words.len() != 5 + count * FLOATS + starts + items {
            return None;
        }
        let f = |i: usize| f32::from_bits(words[i]);
        let sprites = (0..count)
            .map(|n| {
                let b = 5 + n * FLOATS;
                Sprite {
                    centre: [f(b), f(b + 1)],
                    blur: [f(b + 2), f(b + 3)],
                    streak: [f(b + 4), f(b + 5)],
                    radius: f(b + 6),
                    angle: f(b + 7),
                    colour: [f(b + 8), f(b + 9), f(b + 10), f(b + 11)],
                }
            })
            .collect();
        let at = 5 + count * FLOATS;
        let shape = match flags & 0xff {
            0 => Shape::Disc,
            1 => Shape::Streak,
            _ => Shape::Spark,
        };
        Some(DrawList {
            sprites,
            bin_starts: words[at..at + starts].to_vec(),
            bin_items: words[at + starts..].to_vec(),
            samples,
            shape,
            additive: flags >> 8 != 0,
        })
    }
}
