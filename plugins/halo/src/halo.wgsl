const HALO_PI: f32 = 3.14159265358979;

fn halo_f(i: u32) -> f32 { return pass_f(i); }

fn halo_rgb(i: u32) -> vec3<f32> { return vec3<f32>(pass_f(i), pass_f(i + 1u), pass_f(i + 2u)); }

fn halo_lit(c: vec3<f32>) -> bool { return any(c != vec3<f32>(0.0)); }

fn halo_smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn halo_hash(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn halo_unit(v: u32) -> f32 { return f32(halo_hash(v) >> 8u) / 16777216.0; }

fn halo_exp(x: f32) -> f32 {
    let t = clamp(x, -80.0, 80.0) * 1.442695;
    let i = floor(t + 0.5);
    let f = (t - i) * 0.6931472;
    let p = 1.0 + f * (1.0 + f * (0.5 + f * (1.0 / 6.0 + f * (1.0 / 24.0 + f * (1.0 / 120.0 + f / 720.0)))));
    return p * bitcast<f32>(u32(i32(i) + 127) << 23u);
}

fn halo_ln(x: f32) -> f32 {
    let bits = bitcast<u32>(x);
    var e = i32((bits >> 23u) & 255u) - 127;
    var m = bitcast<f32>((bits & 0x007fffffu) | 0x3f800000u);
    if (m > 1.4142135) {
        m = m * 0.5;
        e = e + 1;
    }
    let t = (m - 1.0) / (m + 1.0);
    let s = t * t;
    return f32(e) * 0.6931472 + 2.0 * t * (1.0 + s * (1.0 / 3.0 + s * (1.0 / 5.0 + s * (1.0 / 7.0 + s / 9.0))));
}

fn halo_sin(x: f32) -> f32 {
    let k = floor(x / HALO_PI + 0.5);
    let y = x - k * HALO_PI;
    let s = y * y;
    let p = y * (1.0 + s * (-1.0 / 6.0 + s * (1.0 / 120.0 + s * (-1.0 / 5040.0 + s / 362880.0))));
    if ((i32(k) & 1) == 1) { return -p; }
    return p;
}

fn halo_cos(x: f32) -> f32 { return halo_sin(x + 0.5 * HALO_PI); }

fn halo_atan2(y: f32, x: f32) -> f32 {
    let ax = abs(x);
    let ay = abs(y);
    let hi = max(ax, ay);
    let lo = min(ax, ay);
    if (hi == 0.0) { return 0.0; }
    let a = lo / hi;
    let s = a * a;
    var r = a * (0.99997726 + s * (-0.33262347 + s * (0.19354346 + s * (-0.11643287 + s * (0.05265332 - 0.0117212 * s)))));
    if (ay > ax) { r = 0.5 * HALO_PI - r; }
    if (x < 0.0) { r = HALO_PI - r; }
    if (y < 0.0) { return -r; }
    return r;
}

fn halo_glow(r: f32, radius: f32, falloff: f32) -> f32 {
    let q = (r / radius) * (r / radius);
    var core = 0.0;
    if (q < 1.0) { core = 1.5 * halo_exp(-25.0 * q); }
    return core + halo_exp(-falloff * halo_ln(1.0 + q));
}

fn halo_shimmer(theta: f32, phase: f32, amount: f32) -> f32 {
    let wave = 0.5 * halo_sin(5.0 * theta + 1.7 * phase) + 0.3 * halo_sin(11.0 * theta - 2.3 * phase) + 0.2 * halo_sin(17.0 * theta + 3.1 * phase);
    return 1.0 + amount * wave;
}

fn halo_rays(r: f32, turned: f32, count: u32, length: f32, sharpness: f32, cutoff: f32) -> f32 {
    if (count == 0u || r > 12.0 * length) { return 0.0; }
    let spoke = abs(halo_cos(0.5 * f32(count) * turned));
    if (spoke <= cutoff) { return 0.0; }
    let reach = length * (0.55 + 0.45 * (0.5 + 0.5 * halo_sin(7.0 * turned + 2.0)));
    return halo_exp(sharpness * halo_ln(spoke)) * halo_exp(-r / reach);
}

fn halo_streak(u: f32, v: f32, length: f32, thickness: f32) -> f32 {
    if (abs(v) > 16.0 * thickness) { return 0.0; }
    let core = halo_exp(-abs(u) / length) * halo_exp(-(v / thickness) * (v / thickness));
    let wide = halo_exp(-abs(u) / (0.5 * length)) * halo_exp(-(v / (4.0 * thickness)) * (v / (4.0 * thickness)));
    return core + 0.3 * wide;
}

fn halo_ring(r: f32, radius: f32, width: f32, fringe: f32) -> vec3<f32> {
    let spread = radius * fringe * 0.03;
    if (abs(r - radius) > spread + 4.0 * width) { return vec3<f32>(0.0); }
    let at = vec3<f32>(radius + spread, radius, radius - spread);
    let x = (vec3<f32>(r) - at) / width;
    return vec3<f32>(halo_exp(-x.x * x.x), halo_exp(-x.y * x.y), halo_exp(-x.z * x.z));
}

fn halo_iris(q: vec2<f32>, size: f32, blades: u32, rotation: f32) -> f32 {
    let rq2 = q.x * q.x + q.y * q.y;
    if (rq2 > 1.3225 * size * size) { return 0.0; }
    let rq = sqrt(rq2);
    var rho = rq / size;
    if (blades >= 3u) {
        let sector = 2.0 * HALO_PI / f32(blades);
        let angle = halo_atan2(q.y, q.x) - rotation;
        let a = angle - sector * floor(angle / sector) - 0.5 * sector;
        rho = rq * halo_cos(a) / (halo_cos(HALO_PI / f32(blades)) * size);
    }
    let body = 1.0 - halo_smooth(0.82, 1.0, rho);
    let x = (rho - 0.93) / 0.05;
    return 0.6 * body + 0.4 * halo_exp(-x * x);
}

fn halo_hue(h: f32) -> vec3<f32> {
    let t = vec3<f32>(h) + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs((t - floor(t)) * 6.0 - 3.0), vec3<f32>(1.0), vec3<f32>(2.0)) - 1.0;
}

fn halo_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = bitcast<vec2<u32>>(vec2<i32>(i));
    let a = halo_unit(c.x ^ halo_hash(c.y));
    let b = halo_unit((c.x + 1u) ^ halo_hash(c.y));
    let d = halo_unit(c.x ^ halo_hash(c.y + 1u));
    let e = halo_unit((c.x + 1u) ^ halo_hash(c.y + 1u));
    let top = a + (b - a) * u.x;
    let bottom = d + (e - d) * u.x;
    return top + (bottom - top) * u.y;
}

fn halo_dirt(p: vec2<f32>) -> f32 {
    let n = (halo_noise(p) + 0.5 * halo_noise(vec2<f32>(2.13 * p.x + 17.0, 2.13 * p.y + 5.0))) / 1.5;
    return 0.5 + 0.5 * halo_smooth(0.3, 0.9, n);
}

fn halo_hoop(r: f32, facing: f32, radius: f32, width: f32) -> f32 {
    if (abs(r - radius) > 4.0 * width) { return 0.0; }
    let x = (r - radius) / width;
    let side = 0.5 + 0.5 * facing;
    return halo_exp(-x * x) * side * side * side;
}

fn halo_flare(p: vec2<f32>) -> vec3<f32> {
    let light = vec2<f32>(halo_f(HALO_LIGHT), halo_f(HALO_LIGHT + 1u));
    let centre = vec2<f32>(halo_f(HALO_CENTRE), halo_f(HALO_CENTRE + 1u));
    let d = p - light;
    let r = sqrt(d.x * d.x + d.y * d.y);
    let cs = halo_f(HALO_ROTATION);
    let sn = halo_f(HALO_ROTATION + 1u);
    var sum = vec3<f32>(0.0);
    let glow_rgb = halo_rgb(HALO_GLOW);
    if (halo_lit(glow_rgb)) {
        sum += glow_rgb * halo_glow(r, halo_f(HALO_GLOW_RADIUS), halo_f(HALO_GLOW_FALLOFF));
    }
    let rays_rgb = halo_rgb(HALO_RAYS);
    if (halo_lit(rays_rgb)) {
        let theta = halo_atan2(d.y, d.x);
        let turned = theta - halo_atan2(sn, cs);
        let amount = halo_rays(r, turned, pass_u(HALO_RAY_COUNT), halo_f(HALO_RAYS_LENGTH), halo_f(HALO_RAYS_SHARPNESS), halo_f(HALO_RAYS_CUTOFF));
        if (amount > 0.0) {
            sum += rays_rgb * amount * halo_shimmer(theta, halo_f(HALO_PHASE), halo_f(HALO_SHIMMER));
        }
    }
    let streak_rgb = halo_rgb(HALO_STREAK);
    if (halo_lit(streak_rgb)) {
        let u = d.x * cs + d.y * sn;
        let v = d.y * cs - d.x * sn;
        sum += streak_rgb * halo_streak(u, v, halo_f(HALO_STREAK_LENGTH), halo_f(HALO_STREAK_THICKNESS));
    }
    let ring_rgb = halo_rgb(HALO_RING);
    if (halo_lit(ring_rgb)) {
        sum += ring_rgb * halo_ring(r, halo_f(HALO_RING_RADIUS), halo_f(HALO_RING_WIDTH), halo_f(HALO_RING_FRINGE));
    }
    let axis = light - centre;
    let ghost_rgb = halo_rgb(HALO_GHOSTS);
    if (halo_lit(ghost_rgb)) {
        let seed = halo_hash(pass_u(HALO_SEED));
        let count = min(pass_u(HALO_GHOST_COUNT), HALO_MAX_GHOSTS);
        for (var i = 0u; i < count; i++) {
            let base = seed + i * 4u;
            let along = (-1.4 + 2.2 * halo_unit(base)) * halo_f(HALO_GHOST_SPREAD);
            let u2 = halo_unit(base + 1u);
            let size = (0.25 + 1.1 * u2 * u2) * halo_f(HALO_GHOST_SIZE);
            let brightness = 0.25 + 0.75 * halo_unit(base + 2u);
            let tint = vec3<f32>(0.45) + 0.55 * halo_hue(halo_unit(base + 3u));
            let q = p - (centre + axis * along);
            let amount = halo_iris(q, size, pass_u(HALO_BLADES), halo_f(HALO_IRIS_ROTATION));
            if (amount > 0.0) {
                sum += ghost_rgb * tint * brightness * amount;
            }
        }
    }
    let dirt_rgb = halo_rgb(HALO_DIRT);
    if (halo_lit(dirt_rgb) && r < 2.6 * halo_f(HALO_DIRT_RADIUS)) {
        let q = r / halo_f(HALO_DIRT_RADIUS);
        let origin = vec2<f32>(halo_f(HALO_ORIGIN), halo_f(HALO_ORIGIN + 1u));
        sum += dirt_rgb * halo_exp(-q * q) * halo_dirt((p - origin) / halo_f(HALO_DIRT_CELL));
    }
    let hoop_rgb = halo_rgb(HALO_HOOP);
    if (halo_lit(hoop_rgb)) {
        let reach = sqrt(axis.x * axis.x + axis.y * axis.y);
        var facing = 1.0;
        if (!(reach < 1.0 || r < 1e-3)) { facing = -(d.x * axis.x + d.y * axis.y) / (r * reach); }
        sum += hoop_rgb * halo_hoop(r, facing, halo_f(HALO_HOOP_RADIUS), halo_f(HALO_HOOP_WIDTH));
    }
    return sum;
}

@compute @workgroup_size(16, 16)
fn halo_render(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let source = texel(src, vec2<i32>(id.xy));
    let flare = halo_flare(vec2<f32>(id.xy) + vec2<f32>(0.5));
    let alpha = clamp(max(flare.r, max(flare.g, flare.b)), 0.0, 1.0);
    if (pass_u(HALO_FLARE_ONLY) != 0u) {
        store(id.xy, vec4<f32>(flare, alpha));
        return;
    }
    var mixed = source.rgb + flare;
    if (pass_u(HALO_BLEND) == 1u) {
        mixed = source.rgb + flare - clamp(source.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * clamp(flare, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    store(id.xy, vec4<f32>(mixed, source.a + alpha * (1.0 - source.a)));
}
