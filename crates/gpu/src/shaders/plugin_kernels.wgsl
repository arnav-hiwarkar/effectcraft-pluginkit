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
    for (var j = first.y; j <= last.y; j++) {
        total.y += craft_filter(bicubic, (f32(j) - centre.y) / widen.y);
    }
    let share = 1.0 / total;
    var c = vec4<f32>(0.0);
    for (var j = first.y; j <= last.y; j++) {
        var row = vec4<f32>(0.0);
        for (var i = first.x; i <= last.x; i++) {
            row += texel(t, vec2<i32>(i, j)) * (craft_filter(bicubic, (f32(i) - centre.x) / widen.x) * share.x);
        }
        c += row * (craft_filter(bicubic, (f32(j) - centre.y) / widen.y) * share.y);
    }
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

fn craft_amount(base: vec4<f32>, blended: vec4<f32>, amount: f32) -> vec4<f32> {
    if (amount == 0.0) {
        return base;
    }
    if (amount == 1.0) {
        return blended;
    }
    return base + (blended - base) * amount;
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
        linear = exp2(clamp(2.4 * log2((a + 0.055) / 1.055), -126.0, 126.0));
    }
    return select(linear, -linear, x < 0.0);
}

fn craft_to_srgb(x: f32) -> f32 {
    let a = abs(x);
    var encoded = a * 12.92;
    if (a > 0.0031308) {
        encoded = 1.055 * exp2(clamp(log2(a) / 2.4, -126.0, 126.0)) - 0.055;
    }
    return select(encoded, -encoded, x < 0.0);
}

fn craft_linear(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(craft_to_linear(v.r), craft_to_linear(v.g), craft_to_linear(v.b), v.a);
}

fn craft_srgb(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(craft_to_srgb(v.r), craft_to_srgb(v.g), craft_to_srgb(v.b), v.a);
}

fn craft_linear_below_zero(v: vec4<f32>, transferred: vec4<f32>, slope: f32) -> vec4<f32> {
    return vec4<f32>(select(transferred.rgb, v.rgb * slope, v.rgb < vec3<f32>(0.0)), v.a);
}

fn craft_light_alpha(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(v.rgb, max(v.a, clamp(max(max(v.r, v.g), v.b), 0.0, 1.0)));
}

fn craft_ratio(a: f32, b: f32) -> f32 {
    let parts = frexp(b);
    return ldexp(a, -parts.exp) / parts.fract;
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
    let range = craft_data(at + 2u);
    let parts = frexp(m);
    let per_m = 1.0 / parts.fract;
    if (m > range) {
        let value = craft_data(at + 3u + last);
        let before = craft_data(at + 2u + last);
        var gain = ldexp(value, -parts.exp) * per_m;
        if (value != before) {
            gain += (gain - ldexp(before, -parts.exp) * per_m) * (craft_ratio(m - range, range) * f32(last));
        }
        return vec4<f32>(p.rgb * gain, p.a);
    }
    let t = craft_ratio(m, range) * f32(last);
    let i = min(u32(t), last - 1u);
    let a = ldexp(craft_data(at + 3u + i), -parts.exp) * per_m;
    let b = ldexp(craft_data(at + 4u + i), -parts.exp) * per_m;
    return vec4<f32>(p.rgb * (a + (b - a) * (t - f32(i))), p.a);
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

fn craft_bilinear(t: texture_2d<f32>, at: vec2<f32>) -> vec4<f32> {
    if (!(all(at > vec2<f32>(-1.0)) && all(at < vec2<f32>(textureDimensions(t))))) {
        return vec4<f32>(0.0);
    }
    let base = floor(at);
    let f = at - base;
    let b = vec2<i32>(base);
    let top = mix(texel(t, b), texel(t, b + vec2<i32>(1, 0)), f.x);
    let bottom = mix(texel(t, b + vec2<i32>(0, 1)), texel(t, b + vec2<i32>(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

@compute @workgroup_size(16, 16)
fn craft_warp(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let d = texel(aux, vec2<i32>(id.xy));
    store(id.xy, craft_bilinear(src, vec2<f32>(id.xy) + vec2<f32>(pass_f(0u) * d.r, pass_f(1u) * d.g)));
}

@compute @workgroup_size(16, 16)
fn craft_morphology(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let r = i32(pass_u(0u));
    let erode = pass_u(1u) == 1u;
    let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), pass_u(2u) == 1u);
    let p = vec2<i32>(id.xy);
    let along = dot(p, axis);
    let first = max(along - r, 0) - along;
    let last = min(along + r, dot(vec2<i32>(src_size()), axis) - 1) - along;
    var v = texel(src, p + axis * first);
    for (var k = first + 1; k <= last; k++) {
        let q = texel(src, p + axis * k);
        v = select(max(v, q), min(v, q), erode);
    }
    store(id.xy, v);
}

@compute @workgroup_size(16, 16)
fn craft_directional_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let n = pass_u(0u);
    let step = vec2<f32>(pass_f(0u), pass_f(1u));
    let middle = f32(n - 1u) * 0.5;
    var sum = vec4<f32>(0.0);
    for (var s = 0u; s < n; s++) {
        sum += craft_bilinear(src, vec2<f32>(id.xy) + step * (f32(s) - middle));
    }
    store(id.xy, sum * pass_f(2u));
}

@compute @workgroup_size(16, 16)
fn craft_bilateral(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let r = i32(pass_u(0u));
    let range = pass_f(0u);
    let p = vec2<i32>(id.xy);
    let size = vec2<i32>(src_size());
    let centre = texel(src, p);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var dy = -r; dy <= r; dy++) {
        let y = p.y + dy;
        if (y < 0 || y >= size.y) { continue; }
        for (var dx = -r; dx <= r; dx++) {
            let x = p.x + dx;
            if (x < 0 || x >= size.x) { continue; }
            let q = texel(src, vec2<i32>(x, y));
            let d = q.rgb - centre.rgb;
            let w = craft_data(u32(dx + r)) * craft_data(u32(dy + r)) * exp(-dot(d, d) * range);
            sum += q * w;
            total += w;
        }
    }
    store(id.xy, sum / total);
}

fn craft_lattice(x: u32, y: u32, seed: u32) -> f32 {
    return f32(craft_hash(x, y, seed) & 0xffffffu) / 16777215.0;
}

fn craft_value_noise(at: vec2<f32>, seed: u32) -> f32 {
    let base = floor(at);
    let f = at - base;
    let s = f * f * (3.0 - 2.0 * f);
    let i = bitcast<vec2<u32>>(vec2<i32>(base));
    let top = mix(craft_lattice(i.x, i.y, seed), craft_lattice(i.x + 1u, i.y, seed), s.x);
    let bottom = mix(craft_lattice(i.x, i.y + 1u, seed), craft_lattice(i.x + 1u, i.y + 1u, seed), s.x);
    return mix(top, bottom, s.y);
}

fn craft_noise(at: u32, at_pixel: vec2<u32>) -> vec4<f32> {
    let p = vec2<f32>(bitcast<vec2<i32>>(at_pixel - vec2<u32>(craft_word(at + 4u), craft_word(at + 6u))));
    let seed = craft_word(at + 2u);
    var frequency = craft_data(at);
    var amplitude = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var o = 0u; o < u32(craft_data(at + 1u)); o++) {
        sum += amplitude * craft_value_noise(p * frequency, seed + o * 0x9e3779b9u);
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    let n = sum / total;
    return vec4<f32>(n, n, n, 1.0);
}
