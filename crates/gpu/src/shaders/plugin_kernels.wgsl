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

@compute @workgroup_size(16, 16)
fn craft_resample(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let bicubic = pass_u(0u) == 1u;
    let scale = vec2<f32>(pass_f(0u), pass_f(1u));
    let centre = (vec2<f32>(id.xy) + 0.5) * scale - 0.5;
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
            row += texel(src, vec2<i32>(i, j)) * craft_filter(bicubic, (f32(i) - centre.x) / widen.x);
        }
        sum += row * wy;
    }
    var c = sum / (total.x * total.y);
    if (bicubic) {
        c = max(c, vec4<f32>(0.0));
    }
    store(id.xy, c);
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

@compute @workgroup_size(16, 16)
fn craft_blend(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let mode = pass_u(0u);
    let base = texel(src, vec2<i32>(id.xy));
    let top = texel(aux, vec2<i32>(id.xy));
    var blended = top + base * (1.0 - top.a);
    if (mode != 0u) {
        blended = vec4<f32>(
            craft_blend_channel(mode, base.r, top.r),
            craft_blend_channel(mode, base.g, top.g),
            craft_blend_channel(mode, base.b, top.b),
            craft_blend_channel(mode, base.a, top.a),
        );
    }
    store(id.xy, base + (blended - base) * pass_f(0u));
}

fn craft_curve(n: u32, channel: u32, x: f32) -> f32 {
    let last = n - 1u;
    let t = clamp(x, 0.0, 1.0) * f32(last);
    let i = min(u32(t), last - 1u);
    let a = craft_data(i * 4u + channel);
    let b = craft_data((i + 1u) * 4u + channel);
    return a + (b - a) * (t - f32(i));
}

@compute @workgroup_size(16, 16)
fn craft_curves(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let n = pass_u(0u);
    let p = texel(src, vec2<i32>(id.xy));
    let alpha = craft_curve(n, 3u, p.a);
    var straight = vec3<f32>(0.0);
    if (p.a > 0.0) {
        straight = p.rgb / p.a;
    }
    let colour = vec3<f32>(craft_curve(n, 0u, straight.r), craft_curve(n, 1u, straight.g), craft_curve(n, 2u, straight.b));
    store(id.xy, vec4<f32>(colour * alpha, alpha));
}

@compute @workgroup_size(16, 16)
fn craft_warp(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let d = texel(aux, vec2<i32>(id.xy));
    let at = vec2<f32>(id.xy) + vec2<f32>(pass_f(0u) * d.r, pass_f(1u) * d.g);
    let base = floor(at);
    let f = at - base;
    let b = vec2<i32>(base);
    let top = mix(texel(src, b), texel(src, b + vec2<i32>(1, 0)), f.x);
    let bottom = mix(texel(src, b + vec2<i32>(0, 1)), texel(src, b + vec2<i32>(1, 1)), f.x);
    store(id.xy, mix(top, bottom, f.y));
}

@compute @workgroup_size(16, 16)
fn craft_matrix(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = texel(src, vec2<i32>(id.xy));
    let r = vec4<f32>(pass_f(0u), pass_f(1u), pass_f(2u), pass_f(3u));
    let g = vec4<f32>(pass_f(4u), pass_f(5u), pass_f(6u), pass_f(7u));
    let b = vec4<f32>(pass_f(8u), pass_f(9u), pass_f(10u), pass_f(11u));
    let a = vec4<f32>(pass_f(12u), pass_f(13u), pass_f(14u), pass_f(15u));
    store(id.xy, vec4<f32>(dot(r, p), dot(g, p), dot(b, p), dot(a, p)));
}
