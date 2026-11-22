use std::f32::consts::PI;

pub const FLOATS: usize = 48;
pub const UINTS: usize = 6;

pub const LIGHT: usize = 0;
pub const CENTRE: usize = 2;
pub const ROTATION: usize = 4;
pub const PHASE: usize = 6;
pub const SHIMMER: usize = 7;
pub const GLOW: usize = 8;
pub const GLOW_RADIUS: usize = 11;
pub const GLOW_FALLOFF: usize = 12;
pub const RAYS: usize = 13;
pub const RAYS_LENGTH: usize = 16;
pub const RAYS_SHARPNESS: usize = 17;
pub const STREAK: usize = 18;
pub const STREAK_LENGTH: usize = 21;
pub const STREAK_THICKNESS: usize = 22;
pub const RING: usize = 23;
pub const RING_RADIUS: usize = 26;
pub const RING_WIDTH: usize = 27;
pub const RING_FRINGE: usize = 28;
pub const GHOSTS: usize = 29;
pub const GHOST_SIZE: usize = 32;
pub const IRIS_ROTATION: usize = 33;
pub const GHOST_SPREAD: usize = 34;
pub const DIRT: usize = 35;
pub const DIRT_RADIUS: usize = 38;
pub const DIRT_CELL: usize = 39;
pub const ORIGIN: usize = 40;
pub const HOOP: usize = 42;
pub const HOOP_RADIUS: usize = 45;
pub const HOOP_WIDTH: usize = 46;
pub const RAYS_CUTOFF: usize = 47;

pub const RAY_COUNT: usize = 0;
pub const GHOST_COUNT: usize = 1;
pub const BLADES: usize = 2;
pub const SEED: usize = 3;
pub const BLEND: usize = 4;
pub const FLARE_ONLY: usize = 5;

pub const MAX_GHOSTS: u32 = 16;

pub const NAMES: [(&str, usize); 38] = [
    ("RAYS_CUTOFF", RAYS_CUTOFF),
    ("LIGHT", LIGHT),
    ("CENTRE", CENTRE),
    ("ROTATION", ROTATION),
    ("PHASE", PHASE),
    ("SHIMMER", SHIMMER),
    ("GLOW", GLOW),
    ("GLOW_RADIUS", GLOW_RADIUS),
    ("GLOW_FALLOFF", GLOW_FALLOFF),
    ("RAYS", RAYS),
    ("RAYS_LENGTH", RAYS_LENGTH),
    ("RAYS_SHARPNESS", RAYS_SHARPNESS),
    ("STREAK", STREAK),
    ("STREAK_LENGTH", STREAK_LENGTH),
    ("STREAK_THICKNESS", STREAK_THICKNESS),
    ("RING", RING),
    ("RING_RADIUS", RING_RADIUS),
    ("RING_WIDTH", RING_WIDTH),
    ("RING_FRINGE", RING_FRINGE),
    ("GHOSTS", GHOSTS),
    ("GHOST_SIZE", GHOST_SIZE),
    ("IRIS_ROTATION", IRIS_ROTATION),
    ("GHOST_SPREAD", GHOST_SPREAD),
    ("DIRT", DIRT),
    ("DIRT_RADIUS", DIRT_RADIUS),
    ("DIRT_CELL", DIRT_CELL),
    ("ORIGIN", ORIGIN),
    ("HOOP", HOOP),
    ("HOOP_RADIUS", HOOP_RADIUS),
    ("HOOP_WIDTH", HOOP_WIDTH),
    ("RAY_COUNT", RAY_COUNT),
    ("GHOST_COUNT", GHOST_COUNT),
    ("BLADES", BLADES),
    ("SEED", SEED),
    ("BLEND", BLEND),
    ("FLARE_ONLY", FLARE_ONLY),
    ("MAX_GHOSTS", MAX_GHOSTS as usize),
    ("FLOATS", FLOATS),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub f: [f32; FLOATS],
    pub u: [u32; UINTS],
}

type Rgb = [f32; 3];

fn rgb(pack: &Pack, at: usize) -> Rgb {
    [pack.f[at], pack.f[at + 1], pack.f[at + 2]]
}

fn lit(colour: Rgb) -> bool {
    colour.iter().any(|c| *c != 0.0)
}

fn add(sum: &mut Rgb, colour: Rgb, amount: f32) {
    for (s, c) in sum.iter_mut().zip(colour) {
        *s += c * amount;
    }
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn hash(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

pub fn unit(v: u32) -> f32 {
    (hash(v) >> 8) as f32 / 16_777_216.0
}

pub fn exp(x: f32) -> f32 {
    let t = x.clamp(-80.0, 80.0) * std::f32::consts::LOG2_E;
    let i = (t + 0.5).floor();
    let f = (t - i) * std::f32::consts::LN_2;
    let p = 1.0 + f * (1.0 + f * (0.5 + f * (1.0 / 6.0 + f * (1.0 / 24.0 + f * (1.0 / 120.0 + f / 720.0)))));
    p * f32::from_bits(((i as i32 + 127) as u32) << 23)
}

pub fn ln(x: f32) -> f32 {
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 255) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > std::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let t = (m - 1.0) / (m + 1.0);
    let s = t * t;
    e as f32 * std::f32::consts::LN_2 + 2.0 * t * (1.0 + s * (1.0 / 3.0 + s * (1.0 / 5.0 + s * (1.0 / 7.0 + s / 9.0))))
}

pub fn sin(x: f32) -> f32 {
    let k = (x / PI + 0.5).floor();
    let y = x - k * PI;
    let s = y * y;
    let p = y * (1.0 + s * (-1.0 / 6.0 + s * (1.0 / 120.0 + s * (-1.0 / 5040.0 + s / 362_880.0))));
    if (k as i32) & 1 == 1 { -p } else { p }
}

pub fn cos(x: f32) -> f32 {
    sin(x + 0.5 * PI)
}

pub fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (x.abs(), y.abs());
    let (hi, lo) = (ax.max(ay), ax.min(ay));
    if hi == 0.0 {
        return 0.0;
    }
    let a = lo / hi;
    let s = a * a;
    let mut r = a * (0.999_977_26 + s * (-0.332_623_47 + s * (0.193_543_46 + s * (-0.116_432_87 + s * (0.052_653_32 - 0.011_721_2 * s)))));
    if ay > ax {
        r = 0.5 * PI - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}

pub fn glow(r: f32, radius: f32, falloff: f32) -> f32 {
    let q = (r / radius) * (r / radius);
    let core = if q < 1.0 { 1.5 * exp(-25.0 * q) } else { 0.0 };
    core + exp(-falloff * ln(1.0 + q))
}

pub fn shimmer(theta: f32, phase: f32, amount: f32) -> f32 {
    let wave = 0.5 * sin(5.0 * theta + 1.7 * phase) + 0.3 * sin(11.0 * theta - 2.3 * phase) + 0.2 * sin(17.0 * theta + 3.1 * phase);
    1.0 + amount * wave
}

pub fn rays(r: f32, turned: f32, count: u32, length: f32, sharpness: f32, cutoff: f32) -> f32 {
    if count == 0 || r > 12.0 * length {
        return 0.0;
    }
    let spoke = cos(0.5 * count as f32 * turned).abs();
    if spoke <= cutoff {
        return 0.0;
    }
    let reach = length * (0.55 + 0.45 * (0.5 + 0.5 * sin(7.0 * turned + 2.0)));
    exp(sharpness * ln(spoke)) * exp(-r / reach)
}

pub fn rays_cutoff(sharpness: f32) -> f32 {
    if sharpness <= 0.0 { 0.0 } else { exp(-12.0 / sharpness) }
}

pub fn streak(u: f32, v: f32, length: f32, thickness: f32) -> f32 {
    if v.abs() > 16.0 * thickness {
        return 0.0;
    }
    let core = exp(-u.abs() / length) * exp(-(v / thickness) * (v / thickness));
    let wide = exp(-u.abs() / (0.5 * length)) * exp(-(v / (4.0 * thickness)) * (v / (4.0 * thickness)));
    core + 0.3 * wide
}

pub fn ring(r: f32, radius: f32, width: f32, fringe: f32) -> Rgb {
    let spread = radius * fringe * 0.03;
    if (r - radius).abs() > spread + 4.0 * width {
        return [0.0; 3];
    }
    std::array::from_fn(|c| {
        let at = radius + spread * (1.0 - c as f32);
        let x = (r - at) / width;
        exp(-x * x)
    })
}

pub fn iris(qx: f32, qy: f32, size: f32, blades: u32, rotation: f32) -> f32 {
    let rq2 = qx * qx + qy * qy;
    if rq2 > 1.3225 * size * size {
        return 0.0;
    }
    let rq = rq2.sqrt();
    let rho = if blades < 3 {
        rq / size
    } else {
        let sector = 2.0 * PI / blades as f32;
        let angle = atan2(qy, qx) - rotation;
        let a = angle - sector * (angle / sector).floor() - 0.5 * sector;
        rq * cos(a) / (cos(PI / blades as f32) * size)
    };
    let body = 1.0 - smoothstep(0.82, 1.0, rho);
    let x = (rho - 0.93) / 0.05;
    0.6 * body + 0.4 * exp(-x * x)
}

pub fn hue(h: f32) -> Rgb {
    [0.0, 2.0 / 3.0, 1.0 / 3.0].map(|o: f32| {
        let t = h + o;
        ((t - t.floor()) * 6.0 - 3.0).abs().clamp(1.0, 2.0) - 1.0
    })
}

pub struct Ghost {
    pub along: f32,
    pub size: f32,
    pub brightness: f32,
    pub tint: Rgb,
}

pub fn ghost(seed: u32, index: u32) -> Ghost {
    let base = hash(seed).wrapping_add(index.wrapping_mul(4));
    let [u1, u2, u3, u4] = [0, 1, 2, 3].map(|k| unit(base.wrapping_add(k)));
    let shade = hue(u4);
    Ghost { along: -1.4 + 2.2 * u1, size: 0.25 + 1.1 * u2 * u2, brightness: 0.25 + 0.75 * u3, tint: std::array::from_fn(|c| 0.45 + 0.55 * shade[c]) }
}

pub fn noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (cx, cy) = (ix as i32 as u32, iy as i32 as u32);
    let corner = |dx: u32, dy: u32| unit(cx.wrapping_add(dx) ^ hash(cy.wrapping_add(dy)));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * ux;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * ux;
    top + (bottom - top) * uy
}

pub fn dirt(x: f32, y: f32) -> f32 {
    let n = (noise(x, y) + 0.5 * noise(2.13 * x + 17.0, 2.13 * y + 5.0)) / 1.5;
    0.5 + 0.5 * smoothstep(0.3, 0.9, n)
}

pub fn hoop(r: f32, facing: f32, radius: f32, width: f32) -> f32 {
    if (r - radius).abs() > 4.0 * width {
        return 0.0;
    }
    let x = (r - radius) / width;
    let side = 0.5 + 0.5 * facing;
    exp(-x * x) * side * side * side
}

pub struct Placed {
    pub at: [f32; 2],
    pub size: f32,
    pub colour: Rgb,
}

pub fn place(pack: &Pack) -> Vec<Placed> {
    let f = &pack.f;
    let ghost_rgb = rgb(pack, GHOSTS);
    if !lit(ghost_rgb) {
        return Vec::new();
    }
    let axis = [f[LIGHT] - f[CENTRE], f[LIGHT + 1] - f[CENTRE + 1]];
    (0..pack.u[GHOST_COUNT].min(MAX_GHOSTS))
        .map(|i| {
            let g = ghost(pack.u[SEED], i);
            let along = g.along * f[GHOST_SPREAD];
            Placed {
                at: [f[CENTRE] + axis[0] * along, f[CENTRE + 1] + axis[1] * along],
                size: g.size * f[GHOST_SIZE],
                colour: std::array::from_fn(|c| ghost_rgb[c] * g.tint[c] * g.brightness),
            }
        })
        .collect()
}

pub fn flare(pack: &Pack, ghosts: &[Placed], px: f32, py: f32) -> Rgb {
    let f = &pack.f;
    let (dx, dy) = (px - f[LIGHT], py - f[LIGHT + 1]);
    let r = (dx * dx + dy * dy).sqrt();
    let (rc, rs) = (f[ROTATION], f[ROTATION + 1]);
    let mut sum = [0.0; 3];
    let glow_rgb = rgb(pack, GLOW);
    if lit(glow_rgb) {
        add(&mut sum, glow_rgb, glow(r, f[GLOW_RADIUS], f[GLOW_FALLOFF]));
    }
    let rays_rgb = rgb(pack, RAYS);
    if lit(rays_rgb) {
        let theta = atan2(dy, dx);
        let turned = theta - atan2(rs, rc);
        let amount = rays(r, turned, pack.u[RAY_COUNT], f[RAYS_LENGTH], f[RAYS_SHARPNESS], f[RAYS_CUTOFF]);
        if amount > 0.0 {
            add(&mut sum, rays_rgb, amount * shimmer(theta, f[PHASE], f[SHIMMER]));
        }
    }
    let streak_rgb = rgb(pack, STREAK);
    if lit(streak_rgb) {
        let (u, v) = (dx * rc + dy * rs, dy * rc - dx * rs);
        add(&mut sum, streak_rgb, streak(u, v, f[STREAK_LENGTH], f[STREAK_THICKNESS]));
    }
    let ring_rgb = rgb(pack, RING);
    if lit(ring_rgb) {
        let band = ring(r, f[RING_RADIUS], f[RING_WIDTH], f[RING_FRINGE]);
        for c in 0..3 {
            sum[c] += ring_rgb[c] * band[c];
        }
    }
    let (ax, ay) = (f[LIGHT] - f[CENTRE], f[LIGHT + 1] - f[CENTRE + 1]);
    for g in ghosts {
        let amount = iris(px - g.at[0], py - g.at[1], g.size, pack.u[BLADES], f[IRIS_ROTATION]);
        if amount > 0.0 {
            add(&mut sum, g.colour, amount);
        }
    }
    let dirt_rgb = rgb(pack, DIRT);
    if lit(dirt_rgb) && r < 2.6 * f[DIRT_RADIUS] {
        let q = r / f[DIRT_RADIUS];
        let cell = f[DIRT_CELL];
        let smudge = dirt((px - f[ORIGIN]) / cell, (py - f[ORIGIN + 1]) / cell);
        add(&mut sum, dirt_rgb, exp(-q * q) * smudge);
    }
    let hoop_rgb = rgb(pack, HOOP);
    if lit(hoop_rgb) {
        let reach = (ax * ax + ay * ay).sqrt();
        let facing = if reach < 1.0 || r < 1e-3 { 1.0 } else { -(dx * ax + dy * ay) / (r * reach) };
        add(&mut sum, hoop_rgb, hoop(r, facing, f[HOOP_RADIUS], f[HOOP_WIDTH]));
    }
    sum
}

pub fn composite(pack: &Pack, source: [f32; 4], flare: Rgb) -> [f32; 4] {
    let alpha = flare.iter().fold(0.0f32, |m, c| m.max(*c)).clamp(0.0, 1.0);
    if pack.u[FLARE_ONLY] != 0 {
        return [flare[0], flare[1], flare[2], alpha];
    }
    let mixed: Rgb = std::array::from_fn(|c| {
        let (s, l) = (source[c], flare[c]);
        match pack.u[BLEND] {
            1 => s + l - s.clamp(0.0, 1.0) * l.clamp(0.0, 1.0),
            _ => s + l,
        }
    });
    [mixed[0], mixed[1], mixed[2], source[3] + alpha * (1.0 - source[3])]
}

pub fn wgsl_constants() -> String {
    NAMES.iter().map(|(name, value)| format!("const HALO_{name}: u32 = {value}u;\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn fast_maths_tracks_the_standard_library() {
        for i in -400..400 {
            let x = i as f32 * 0.05;
            assert!((exp(x) - x.exp()).abs() <= 2e-6 * x.exp().max(1.0), "exp {x}");
            assert!((sin(x) - x.sin()).abs() < 2e-5, "sin {x}");
            assert!((cos(x) - x.cos()).abs() < 2e-5, "cos {x}");
            let y = (i as f32 * 0.37).cos() * 3.0;
            assert!((atan2(y, x) - y.atan2(x)).abs() < 5e-6, "atan2 {y} {x}");
            let z = 1e-3 + (i + 400) as f32 * 0.25;
            assert!((ln(z) - z.ln()).abs() < 2e-6 * z.ln().abs().max(1.0), "ln {z}");
        }
        assert_eq!(atan2(0.0, 0.0), 0.0);
    }

    #[test]
    fn glow_peaks_at_the_light_and_falls_off_with_the_falloff() {
        assert!(close(glow(0.0, 50.0, 1.0), 2.5));
        assert!(close(glow(50.0, 50.0, 1.0), 0.5));
        assert!(close(glow(50.0, 50.0, 2.0), 0.25));
        assert!(close(glow(150.0, 50.0, 1.0), 0.1));
    }

    #[test]
    fn rays_point_along_spokes_and_vanish_between_them() {
        assert!(close(rays(10.0, 0.0, 6, 10.0, 8.0, 0.0), (-10.0 / (10.0 * (0.55 + 0.45 * (0.5 + 0.5 * 2.0f32.sin())))).exp()));
        assert!(rays(10.0, PI / 6.0, 6, 10.0, 8.0, 0.0) < 1e-6);
        assert!(rays(10.0, PI / 3.0, 6, 10.0, 8.0, 0.0) > 0.2);
        assert_eq!(rays(10.0, 0.0, 0, 10.0, 8.0, 0.0), 0.0);
        assert_eq!(rays(130.0, 0.0, 6, 10.0, 8.0, 0.0), 0.0);
        assert_eq!(rays(10.0, 0.5, 6, 10.0, 8.0, rays_cutoff(8.0)), 0.0);
        assert!(close(rays_cutoff(12.0), (-1.0f32).exp()));
    }

    #[test]
    fn streak_is_long_and_thin() {
        assert!(close(streak(0.0, 0.0, 100.0, 2.0), 1.3));
        assert!(close(streak(100.0, 0.0, 100.0, 2.0), (-1.0f32).exp() + 0.3 * (-2.0f32).exp()));
        assert!(streak(0.0, 6.0, 100.0, 2.0) < 0.3);
        assert_eq!(streak(0.0, 40.0, 100.0, 2.0), 0.0);
    }

    #[test]
    fn ring_fringe_puts_red_outside_and_blue_inside() {
        let [r, g, b] = ring(103.0, 100.0, 0.5, 1.0);
        assert!(close(r, 1.0) && g < 1e-6 && b < 1e-6);
        let [r, g, b] = ring(97.0, 100.0, 0.5, 1.0);
        assert!(close(b, 1.0) && r < 1e-6 && g < 1e-6);
        assert_eq!(ring(100.0, 100.0, 2.0, 0.0), [1.0; 3]);
    }

    #[test]
    fn iris_is_a_polygon_with_a_bright_rim() {
        assert!(close(iris(0.0, 0.0, 20.0, 6, 0.0), 0.6));
        assert!(close(iris(18.6, 0.0, 20.0, 6, 0.0), 0.4 + 0.6 * (1.0 - smoothstep(0.82, 1.0, 0.93))));
        let edge_mid = 20.0 * (PI / 6.0).cos();
        let corner_dir = iris(19.5 * (PI / 6.0).cos(), 19.5 * (PI / 6.0).sin(), 20.0, 6, PI / 6.0);
        assert!(corner_dir > 0.0);
        assert!(iris(edge_mid * 1.12, 0.0, 20.0, 6, PI / 6.0) < 1e-3);
        assert_eq!(iris(24.0, 0.0, 20.0, 6, 0.0), 0.0);
    }

    #[test]
    fn hue_wheel_hits_the_primaries() {
        assert_eq!(hue(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(hue(1.0 / 3.0), [0.0, 1.0, 0.0]);
        assert_eq!(hue(2.0 / 3.0), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn ghosts_are_deterministic_from_the_seed() {
        let a = ghost(7, 3);
        let b = ghost(7, 3);
        let c = ghost(8, 3);
        assert_eq!((a.along, a.size, a.brightness), (b.along, b.size, b.brightness));
        assert_ne!(a.along, c.along);
        for i in 0..MAX_GHOSTS {
            let g = ghost(42, i);
            assert!((-1.4..=0.8).contains(&g.along) && (0.25..=1.35).contains(&g.size) && (0.25..=1.0).contains(&g.brightness));
        }
    }

    #[test]
    fn noise_is_continuous_across_cells() {
        assert!((noise(2.9999, 1.5) - noise(3.0001, 1.5)).abs() < 1e-3);
        assert!(close(noise(3.0, 4.0), unit(3 ^ hash(4))));
        assert!((0.5..=1.0).contains(&dirt(1.7, -3.2)));
    }

    #[test]
    fn hoop_shows_on_the_side_that_faces_the_centre() {
        assert!(close(hoop(200.0, 1.0, 200.0, 5.0), 1.0));
        assert!(close(hoop(200.0, 0.0, 200.0, 5.0), 0.125));
        assert_eq!(hoop(200.0, -1.0, 200.0, 5.0), 0.0);
        assert_eq!(hoop(230.0, 1.0, 200.0, 5.0), 0.0);
    }

    #[test]
    fn composite_adds_screens_or_shows_the_flare_alone() {
        let mut pack = Pack { f: [0.0; FLOATS], u: [0; UINTS] };
        assert_eq!(composite(&pack, [0.5, 0.2, 0.0, 1.0], [0.5, 0.5, 0.25]), [1.0, 0.7, 0.25, 1.0]);
        pack.u[BLEND] = 1;
        let screened = composite(&pack, [0.5, 0.2, 0.0, 1.0], [0.5, 0.5, 0.25]);
        assert!(screened.iter().zip([0.75, 0.6, 0.25, 1.0]).all(|(a, b)| close(*a, b)), "{screened:?}");
        pack.u[FLARE_ONLY] = 1;
        assert_eq!(composite(&pack, [0.5, 0.2, 0.0, 0.0], [0.5, 0.25, 0.0]), [0.5, 0.25, 0.0, 0.5]);
        pack.u = [0; UINTS];
        assert_eq!(composite(&pack, [0.0; 4], [2.0, 0.0, 0.0]), [2.0, 0.0, 0.0, 1.0]);
    }
}
