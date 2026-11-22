fn craft_data(i: u32) -> f32 {
    return data[i];
}

@compute @workgroup_size(16, 16)
fn craft_convolve_x(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let n = i32(pass_u(0u));
    let p = vec2<i32>(id.xy);
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < n; k++) {
        sum += texel(src, vec2<i32>(p.x + k - n / 2, p.y)) * craft_data(u32(k));
    }
    store(id.xy, sum);
}

@compute @workgroup_size(16, 16)
fn craft_convolve_y(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let n = i32(pass_u(0u));
    let p = vec2<i32>(id.xy);
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < n; k++) {
        sum += texel(src, vec2<i32>(p.x, p.y + k - n / 2)) * craft_data(u32(k));
    }
    store(id.xy, sum);
}

@compute @workgroup_size(16, 16)
fn craft_convolve_full(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let w = i32(pass_u(0u));
    let h = i32(pass_u(1u));
    let p = vec2<i32>(id.xy);
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < h; j++) {
        for (var i = 0; i < w; i++) {
            sum += texel(src, p + vec2<i32>(i - w / 2, j - h / 2)) * craft_data(u32(j * w + i));
        }
    }
    store(id.xy, sum);
}

fn craft_filter(bicubic: bool, d: f32) -> f32 {
    let a = abs(d);
    if (!bicubic) {
        return max(1.0 - a, 0.0);
    }
    if (a < 1.0) {
        return (1.5 * a - 2.5) * a * a + 1.0;
    }
    if (a < 2.0) {
        return ((-0.5 * a + 2.5) * a - 4.0) * a + 2.0;
    }
    return 0.0;
}

fn craft_resampled(t: texture_2d<f32>, p: vec2<i32>, scale: vec2<f32>, bicubic: bool) -> vec4<f32> {
    let centre = (vec2<f32>(p) + 0.5) * scale - 0.5;
    let widen = max(scale, vec2<f32>(1.0));
    let reach = select(1.0, 2.0, bicubic) * widen;
    let first = vec2<i32>(floor(centre - reach)) + 1;
    let last = vec2<i32>(ceil(centre + reach)) - 1;
    var total = vec2<f32>(0.0);
    for (var i = first.x; i <= last.x; i++) {
        total.x += craft_filter(bicubic, (f32(i) - centre.x) / widen.x);
    }
    var sum = vec4<f32>(0.0);
    for (var j = first.y; j <= last.y; j++) {
        let wy = craft_filter(bicubic, (f32(j) - centre.y) / widen.y);
        total.y += wy;
        var row = vec4<f32>(0.0);
        for (var i = first.x; i <= last.x; i++) {
            row += texel(t, vec2<i32>(i, j)) * craft_filter(bicubic, (f32(i) - centre.x) / widen.x);
        }
        sum += row * wy;
    }
    let c = sum / (total.x * total.y);
    if (bicubic) {
        return select(vec4<f32>(0.0), max(c, vec4<f32>(0.0)), c.a > 0.0);
    }
    return c;
}

@compute @workgroup_size(16, 16)
fn craft_resample(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let scale = vec2<f32>(pass_f(0u), pass_f(1u));
    store(id.xy, craft_resampled(src, vec2<i32>(id.xy), scale, pass_u(0u) == 1u));
}

fn craft_blend_channel(mode: u32, b: f32, t: f32) -> f32 {
    switch mode {
        case 2u: { return b - t; }
        case 3u: { return b * t; }
        case 4u: { return b + t - b * t; }
        case 5u: { return select(1.0 - 2.0 * (1.0 - b) * (1.0 - t), 2.0 * b * t, b <= 0.5); }
        case 6u: { return min(b, t); }
        case 7u: { return max(b, t); }
        case 8u: { return abs(b - t); }
        default: { return b + t; }
    }
}

fn craft_curve(at: u32, n: u32, channel: u32, x: f32) -> f32 {
    let last = n - 1u;
    let t = clamp(x, 0.0, 1.0) * f32(last);
    let i = min(u32(t), last - 1u);
    let a = craft_data(at + i * 4u + channel);
    let b = craft_data(at + (i + 1u) * 4u + channel);
    return a + (b - a) * (t - f32(i));
}

fn craft_curves(at: u32, p: vec4<f32>) -> vec4<f32> {
    let n = u32(craft_data(at));
    let alpha = craft_curve(at + 1u, n, 3u, p.a);
    var straight = vec3<f32>(0.0);
    if (p.a > 0.0) {
        straight = p.rgb / p.a;
    }
    let colour = vec3<f32>(craft_curve(at + 1u, n, 0u, straight.r), craft_curve(at + 1u, n, 1u, straight.g), craft_curve(at + 1u, n, 2u, straight.b));
    return vec4<f32>(colour * alpha, alpha);
}

fn craft_blend(at: u32, base: vec4<f32>, top: vec4<f32>) -> vec4<f32> {
    let mode = u32(craft_data(at));
    var blended = top + base * (1.0 - top.a);
    if (mode != 0u) {
        blended = vec4<f32>(
            craft_blend_channel(mode, base.r, top.r),
            craft_blend_channel(mode, base.g, top.g),
            craft_blend_channel(mode, base.b, top.b),
            craft_blend_channel(mode, base.a, top.a),
        );
    }
    let amount = craft_data(at + 1u);
    if (amount == 0.0) {
        return base;
    }
    if (amount == 1.0) {
        return blended;
    }
    return base + (blended - base) * amount;
}

fn craft_matrix(at: u32, p: vec4<f32>) -> vec4<f32> {
    let r = vec4<f32>(craft_data(at), craft_data(at + 1u), craft_data(at + 2u), craft_data(at + 3u));
    let g = vec4<f32>(craft_data(at + 4u), craft_data(at + 5u), craft_data(at + 6u), craft_data(at + 7u));
    let b = vec4<f32>(craft_data(at + 8u), craft_data(at + 9u), craft_data(at + 10u), craft_data(at + 11u));
    let a = vec4<f32>(craft_data(at + 12u), craft_data(at + 13u), craft_data(at + 14u), craft_data(at + 15u));
    return vec4<f32>(dot(r, p), dot(g, p), dot(b, p), dot(a, p));
}

fn craft_to_linear(x: f32) -> f32 {
    let a = abs(x);
    var linear = a / 12.92;
    if (a > 0.04045) {
        linear = pow((a + 0.055) / 1.055, 2.4);
    }
    return select(linear, -linear, x < 0.0);
}

fn craft_to_srgb(x: f32) -> f32 {
    let a = abs(x);
    var encoded = a * 12.92;
    if (a > 0.0031308) {
        encoded = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(encoded, -encoded, x < 0.0);
}

fn craft_brightness_curve(at: u32, p: vec4<f32>) -> vec4<f32> {
    var m = max(max(p.r, p.g), p.b);
    if (craft_data(at) != 0.0) {
        m = dot(p.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    }
    if (m <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, p.a);
    }
    let last = u32(craft_data(at + 1u)) - 1u;
    let t = m * craft_data(at + 2u);
    let i = min(u32(t), last - 1u);
    let a = craft_data(at + 3u + i);
    let b = craft_data(at + 4u + i);
    let gain = (a + (b - a) * (t - f32(i))) / m;
    return vec4<f32>(p.rgb * gain, p.a);
}

fn craft_word(at: u32) -> u32 {
    return u32(craft_data(at)) | (u32(craft_data(at + 1u)) << 16u);
}

fn craft_hash(x: u32, y: u32, seed: u32) -> u32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    return h ^ (h >> 16u);
}

fn craft_dither(at: u32, p: vec4<f32>, at_pixel: vec2<u32>) -> vec4<f32> {
    let room = clamp(p.rgb * 255.0, vec3<f32>(0.0), vec3<f32>(1.0)) * clamp((1.0 - p.rgb) * 255.0, vec3<f32>(0.0), vec3<f32>(1.0));
    if (all(room == vec3<f32>(0.0))) {
        return p;
    }
    let h = craft_hash(at_pixel.x - craft_word(at + 2u), at_pixel.y - craft_word(at + 4u), craft_word(at));
    let n = (f32(h & 0xffffu) - f32(h >> 16u)) * (1.0 / 65536.0) * (1.0 / 255.0);
    return vec4<f32>(p.rgb + n * room, p.a);
}

fn craft_program(s0: vec4<f32>, s1: vec4<f32>, at_pixel: vec2<u32>) -> vec4<f32> {
    var held: array<vec4<f32>, 16>;
    held[0] = s0;
    held[1] = s1;
    let n = u32(craft_data(0u));
    for (var k = 0u; k < n; k++) {
        let head = 1u + k * 4u;
        let code = u32(craft_data(head));
        let v = held[u32(craft_data(head + 1u))];
        let top = held[u32(craft_data(head + 2u))];
        let at = u32(craft_data(head + 3u));
        var r = v;
        switch code {
            case 0u: { r = craft_blend(at, v, top); }
            case 1u: { r = craft_curves(at, v); }
            case 2u: { r = craft_matrix(at, v); }
            case 3u: { r = vec4<f32>(craft_to_linear(v.r), craft_to_linear(v.g), craft_to_linear(v.b), v.a); }
            case 4u: { r = vec4<f32>(craft_to_srgb(v.r), craft_to_srgb(v.g), craft_to_srgb(v.b), v.a); }
            case 5u: { r = craft_brightness_curve(at, v); }
            case 6u: { r = craft_dither(at, v, at_pixel); }
            case 7u: { r = vec4<f32>(v.rgb, max(v.a, clamp(max(max(v.r, v.g), v.b), 0.0, 1.0))); }
            default: {}
        }
        held[2u + k] = r;
    }
    return held[1u + n];
}

@compute @workgroup_size(16, 16)
fn craft_pointwise(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let resampled = pass_u(0u);
    let scale = vec2<f32>(pass_f(0u), pass_f(1u));
    let bicubic = pass_u(1u) == 1u;
    var s0 = texel(src, p);
    var s1 = texel(aux, p);
    if (resampled == 1u) {
        s0 = craft_resampled(src, p, scale, bicubic);
    }
    if (resampled == 2u) {
        s1 = craft_resampled(aux, p, scale, bicubic);
    }
    store(id.xy, craft_program(s0, s1, id.xy));
}

@compute @workgroup_size(16, 16)
fn craft_warp(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let d = texel(aux, vec2<i32>(id.xy));
    let at = vec2<f32>(id.xy) + vec2<f32>(pass_f(0u) * d.r, pass_f(1u) * d.g);
    if (!(all(at > vec2<f32>(-1.0)) && all(at < vec2<f32>(src_size())))) {
        store(id.xy, vec4<f32>(0.0));
        return;
    }
    let base = floor(at);
    let f = at - base;
    let b = vec2<i32>(base);
    let top = mix(texel(src, b), texel(src, b + vec2<i32>(1, 0)), f.x);
    let bottom = mix(texel(src, b + vec2<i32>(0, 1)), texel(src, b + vec2<i32>(1, 1)), f.x);
    store(id.xy, mix(top, bottom, f.y));
}
