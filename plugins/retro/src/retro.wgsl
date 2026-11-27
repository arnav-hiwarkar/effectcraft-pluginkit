const RETRO_TAU: f32 = 6.2831853;

fn retro_luma(p: vec4<f32>) -> f32 {
    return 0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b;
}

fn retro_smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn retro_fract(x: f32) -> f32 {
    return x - floor(x);
}

fn retro_hash(x: u32, y: u32, z: u32) -> f32 {
    var h = (x * 73856093u) ^ (y * 19349663u) ^ (z * 83492791u);
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0;
}

fn retro_bilinear(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let x0 = floor(x);
    let y0 = floor(y);
    let tx = x - x0;
    let ty = y - y0;
    let p = vec2<i32>(i32(x0), i32(y0));
    let a = texel(t, p);
    let b = texel(t, p + vec2<i32>(1, 0));
    let c = texel(t, p + vec2<i32>(0, 1));
    let d = texel(t, p + vec2<i32>(1, 1));
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

fn retro_coarse(t: texture_2d<f32>, factor: f32, x: f32, y: f32) -> vec4<f32> {
    return retro_bilinear(t, (x + 0.5) / factor - 0.5, (y + 0.5) / factor - 0.5);
}

fn retro_gauss(i: i32, sigma: f32) -> f32 {
    let d = f32(i);
    return exp(-(d * d) / (2.0 * sigma * sigma));
}

@compute @workgroup_size(16, 16)
fn retro_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let sigmas = vec3<f32>(pass_f(0u), pass_f(1u), pass_f(2u));
    let radius = i32(ceil(2.5 * max(sigmas.x, max(sigmas.y, sigmas.z))));
    let vertical = pass_u(0u) == 1u;
    let fan = pass_u(1u) == 1u;
    let p = vec2<i32>(id.xy);
    var sum = vec3<f32>(0.0);
    var total = vec3<f32>(0.0);
    for (var i = -radius; i <= radius; i = i + 1) {
        let w = vec3<f32>(retro_gauss(i, sigmas.x), retro_gauss(i, sigmas.y), retro_gauss(i, sigmas.z));
        var offset = vec2<i32>(i, 0);
        if (vertical) { offset = vec2<i32>(0, i); }
        let c = texel(src, p + offset);
        var v = c.rgb;
        if (fan) { v = vec3<f32>(c.r); }
        sum = sum + w * v;
        total = total + w;
    }
    store(id.xy, vec4<f32>(sum / total, 0.0));
}

fn retro_bright(p: vec4<f32>, threshold: f32) -> vec4<f32> {
    let l = retro_luma(p);
    let k = max(l - threshold, 0.0) / max(l, 1e-4);
    return vec4<f32>(p.rgb * k, 0.0);
}

@compute @workgroup_size(16, 16)
fn retro_crt_emit(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let d = i32(pass_u(0u));
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < d; j = j + 1) {
        for (var i = 0; i < d; i = i + 1) {
            sum = sum + retro_bright(texel(src, vec2<i32>(id.xy) * d + vec2<i32>(i, j)), pass_f(0u));
        }
    }
    store(id.xy, sum / f32(d * d));
}

fn retro_parity(v: f32) -> bool {
    return v - 2.0 * floor(v / 2.0) >= 1.0;
}

fn retro_mask(x: f32, y: f32) -> vec3<f32> {
    let p = pass_f(2u);
    let s = pass_f(3u);
    var phase = retro_fract(x / (3.0 * p));
    var rows = 1.0;
    let kind = pass_u(0u);
    if (kind == 1u) {
        var shift = 0.0;
        if (retro_parity(floor(x / (3.0 * p)))) { shift = 0.5; }
        let slot = 0.5 + 0.5 * cos(RETRO_TAU * retro_fract(y / (3.0 * p) + shift));
        rows = 1.0 - s * 0.6 * (slot * slot * slot * slot);
    } else if (kind == 2u) {
        var offset = 0.0;
        if (retro_parity(floor(y / p))) { offset = 0.5; }
        let dot = 0.5 - 0.5 * cos(RETRO_TAU * retro_fract(y / p));
        phase = retro_fract(x / (3.0 * p) + offset);
        rows = 1.0 - s * (1.0 - dot);
    }
    var out = vec3<f32>(0.0);
    for (var c = 0; c < 3; c = c + 1) {
        let w = 0.5 + 0.5 * cos(RETRO_TAU * retro_fract(phase - f32(c) / 3.0));
        out[c] = min(1.0 - s + s * w * w * 1.5, 1.0 + s) * rows;
    }
    return out;
}

@compute @workgroup_size(16, 16)
fn retro_crt(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let size = vec2<f32>(out_size());
    let w = size.x;
    let h = size.y;
    let fx = f32(id.x) + 0.5;
    let fy = f32(id.y) + 0.5;
    let curvature = pass_f(0u);
    let r = pass_f(1u);
    let u = fx / w * 2.0 - 1.0;
    let v = fy / h * 2.0 - 1.0;
    let su = u * (1.0 + curvature * v * v);
    let sv = v * (1.0 + curvature * u * u);
    let aspect = w / h;
    let qx = (abs(su) - 1.0) * aspect + r;
    let qy = abs(sv) - 1.0 + r;
    let outside = sqrt(max(qx, 0.0) * max(qx, 0.0) + max(qy, 0.0) * max(qy, 0.0));
    let distance = outside + min(max(qx, qy), 0.0) - r;
    let shape = clamp(0.5 - distance * h * 0.5, 0.0, 1.0) * max(1.0 - pass_f(9u) * 0.5 * (su * su + sv * sv), 0.0);
    if (shape <= 0.0) {
        store(id.xy, vec4<f32>(0.0));
        return;
    }
    let sx = (su + 1.0) * 0.5 * w - 0.5;
    let sy = (sv + 1.0) * 0.5 * h - 0.5;
    let conv = pass_f(7u);
    let red = retro_bilinear(src, sx + conv, sy);
    let green = retro_bilinear(src, sx, sy);
    let blue = retro_bilinear(src, sx - conv, sy);
    let halo = retro_coarse(aux, f32(pass_u(2u)), sx, sy);
    let mask = retro_mask(fx, fy);
    let scan_w = 0.5 + 0.5 * cos(RETRO_TAU * retro_fract((fy + pass_f(6u)) / pass_f(5u)));
    let lines = 1.0 - pass_f(4u) * (1.0 - scan_w);
    let average = (1.0 - pass_f(3u) * 0.5) * (1.0 - pass_f(4u) * 0.5);
    let gain = 1.0 + pass_f(12u) * (1.0 / max(average, 0.05) - 1.0);
    let alpha = green.a;
    let frame = pass_u(1u);
    let flick = 1.0 + pass_f(11u) * (retro_hash(frame, 7u, 3u) - 0.5) * 2.0;
    let grain = (retro_hash(id.x, id.y, frame) - 0.5) * pass_f(10u);
    let base = vec3<f32>(red.r, green.g, blue.b);
    let lit = base * mask * lines * gain + halo.rgb * pass_f(8u);
    let contrasted = (lit - 0.18 * alpha) * pass_f(13u) + 0.18 * alpha;
    let rgb = max((contrasted + grain * alpha) * flick, vec3<f32>(0.0)) * shape;
    store(id.xy, vec4<f32>(rgb, alpha * shape));
}

fn retro_signal(p: vec4<f32>) -> f32 {
    if (pass_u(0u) == 1u) { return p.a; }
    return retro_luma(p);
}

fn retro_line(x: i32, y: i32) -> f32 {
    let t = i32(pass_u(1u));
    let a = retro_signal(texel(src, vec2<i32>(x - t, y - t)));
    let b = retro_signal(texel(src, vec2<i32>(x, y - t)));
    let c = retro_signal(texel(src, vec2<i32>(x + t, y - t)));
    let d = retro_signal(texel(src, vec2<i32>(x - t, y)));
    let f = retro_signal(texel(src, vec2<i32>(x + t, y)));
    let g = retro_signal(texel(src, vec2<i32>(x - t, y + t)));
    let k = retro_signal(texel(src, vec2<i32>(x, y + t)));
    let l = retro_signal(texel(src, vec2<i32>(x + t, y + t)));
    let gx = c + 2.0 * f + l - a - 2.0 * d - g;
    let gy = g + 2.0 * k + l - a - 2.0 * b - c;
    let magnitude = sqrt(gx * gx + gy * gy) * 0.25;
    return retro_smoothstep(pass_f(0u) - pass_f(1u), pass_f(0u) + pass_f(1u), magnitude);
}

@compute @workgroup_size(16, 16)
fn retro_neon_emit(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let d = i32(pass_u(2u));
    var sum = 0.0;
    for (var j = 0; j < d; j = j + 1) {
        for (var i = 0; i < d; i = i + 1) {
            let p = vec2<i32>(id.xy) * d + vec2<i32>(i, j);
            sum = sum + retro_line(p.x, p.y);
        }
    }
    store(id.xy, vec4<f32>(sum / f32(d * d), 0.0, 0.0, 0.0));
}

@compute @workgroup_size(16, 16)
fn retro_neon(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let edge = retro_line(p.x, p.y);
    let blurred = retro_coarse(aux, f32(pass_u(2u)), f32(id.x), f32(id.y));
    let falloff = pass_f(3u);
    let bloom = (blurred.r + falloff * blurred.g + falloff * falloff * blurred.b) / (1.0 + falloff + falloff * falloff) * pass_f(2u);
    let flick = 1.0 - pass_f(4u) * retro_hash(pass_u(4u), 11u, 5u);
    let lit = edge * flick;
    let halo = bloom * flick;
    let core = vec3<f32>(pass_f(5u), pass_f(6u), pass_f(7u));
    let glow = vec4<f32>(pass_f(9u), pass_f(10u), pass_f(11u), pass_f(12u));
    let rgb = core * lit + glow.rgb * halo;
    let cover = min(lit + halo * retro_luma(glow), 1.0);
    if (pass_u(3u) == 1u) {
        store(id.xy, vec4<f32>(rgb, cover));
        return;
    }
    let s = texel(src, p);
    store(id.xy, vec4<f32>(s.rgb * (1.0 - lit) + rgb, max(s.a, cover)));
}
