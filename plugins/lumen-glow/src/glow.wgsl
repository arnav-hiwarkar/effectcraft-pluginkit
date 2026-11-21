fn lumen_down_tap(k: i32) -> f32 {
    return select(0.375, 0.125, k == 0 || k == 3);
}

fn lumen_to_linear(x: f32) -> f32 {
    if (x <= 0.04045) {
        return x / 12.92;
    }
    return pow((x + 0.055) / 1.055, 2.4);
}

fn lumen_to_srgb(x: f32) -> f32 {
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn lumen_decode(p: vec4<f32>, srgb: bool) -> vec4<f32> {
    if (!srgb) {
        return p;
    }
    return vec4<f32>(lumen_to_linear(p.r), lumen_to_linear(p.g), lumen_to_linear(p.b), p.a);
}

fn lumen_encode(c: vec3<f32>, srgb: bool) -> vec3<f32> {
    if (!srgb) {
        return c;
    }
    return vec3<f32>(lumen_to_srgb(c.r), lumen_to_srgb(c.g), lumen_to_srgb(c.b));
}

// knee.rs: Knee::response and Knee::emit.
fn lumen_response(b: f32, threshold: f32, width: f32) -> f32 {
    let above = b - threshold;
    if (width <= 0.0) {
        return max(above, 0.0);
    }
    let soft = clamp(above + width, 0.0, 2.0 * width);
    return max(above, soft * soft / (4.0 * width));
}

fn lumen_knee(p: vec4<f32>, threshold: f32, width: f32) -> vec4<f32> {
    let b = max(max(p.r, p.g), p.b);
    if (b <= 0.0) {
        return vec4<f32>(0.0);
    }
    return p * (lumen_response(b, threshold, width) / b);
}

// pixel.rs: Mix::apply, its three columns in floats 0..12.
fn lumen_mix(p: vec4<f32>) -> vec4<f32> {
    let r = vec4<f32>(pass_f(0u), pass_f(1u), pass_f(2u), pass_f(3u));
    let g = vec4<f32>(pass_f(4u), pass_f(5u), pass_f(6u), pass_f(7u));
    let b = vec4<f32>(pass_f(8u), pass_f(9u), pass_f(10u), pass_f(11u));
    return r * p.r + g * p.g + b * p.b;
}

// resample.rs: upsample_row, 3/4 of the coarse texel under `p` and 1/4 of its outer neighbour on each halved axis.
fn lumen_upsample(t: texture_2d<f32>, p: vec2<i32>, halve: vec2<bool>) -> vec4<f32> {
    let near = select(p, p / 2, halve);
    let even = (p & vec2<i32>(1)) == vec2<i32>(0);
    let far = select(p, select(near + 1, near - 1, even), halve);
    let wn = select(vec2<f32>(1.0), vec2<f32>(0.75), halve);
    let wf = select(vec2<f32>(0.0), vec2<f32>(0.25), halve);
    let top = texel(t, near) * wn.x + texel(t, vec2<i32>(far.x, near.y)) * wf.x;
    let bottom = texel(t, vec2<i32>(near.x, far.y)) * wn.x + texel(t, far) * wf.x;
    return top * wn.y + bottom * wf.y;
}

// Pass::Emit (passes/emit.rs): decode, soft-knee threshold, then the 4-tap 2x downsample into level 0.
@compute @workgroup_size(16, 16)
fn lumen_emit(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) {
        return;
    }
    let base = vec2<i32>(id.xy) * 2 - 1;
    let srgb = pass_u(0u) != 0u;
    var sum = vec4<f32>(0.0);
    for (var ky = 0; ky < 4; ky++) {
        var row = vec4<f32>(0.0);
        for (var kx = 0; kx < 4; kx++) {
            let p = lumen_decode(texel(src, base + vec2<i32>(kx, ky)), srgb);
            row += lumen_knee(p, pass_f(0u), pass_f(1u)) * lumen_down_tap(kx);
        }
        sum += row * lumen_down_tap(ky);
    }
    store(id.xy, sum);
}

// Pass::Downsample (passes/resample.rs downsample): 4-tap on each halved axis, a copy on the others.
// The level is pass_u(2..4) wide and high; the rest of `out` is cleared.
@compute @workgroup_size(16, 16)
fn lumen_downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) {
        return;
    }
    if (any(id.xy >= vec2<u32>(pass_u(2u), pass_u(3u)))) {
        store(id.xy, vec4<f32>(0.0));
        return;
    }
    let p = vec2<i32>(id.xy);
    let halve = vec2<bool>(pass_u(0u) != 0u, pass_u(1u) != 0u);
    let taps = select(vec2<i32>(1), vec2<i32>(4), halve);
    let base = select(p, p * 2 - 1, halve);
    var sum = vec4<f32>(0.0);
    for (var ky = 0; ky < taps.y; ky++) {
        var row = vec4<f32>(0.0);
        for (var kx = 0; kx < taps.x; kx++) {
            row += texel(src, base + vec2<i32>(kx, ky)) * select(1.0, lumen_down_tap(kx), halve.x);
        }
        sum += row * select(1.0, lumen_down_tap(ky), halve.y);
    }
    store(id.xy, sum);
}

// Pass::Blur then Pass::Accumulate for one level: the separable kernels of Level::kernels applied
// as one 2-D sum (rows, then columns), the level's Mix, plus the coarser accumulated level (aux)
// upsampled. Floats 12.. hold the row kernel from its centre out, then the column kernel.
@compute @workgroup_size(16, 16)
fn lumen_level(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let r = vec2<i32>(i32(pass_u(0u)), i32(pass_u(1u)));
    let column = 13u + pass_u(0u);
    var blurred = vec4<f32>(0.0);
    for (var dy = -r.y; dy <= r.y; dy++) {
        var row = vec4<f32>(0.0);
        for (var dx = -r.x; dx <= r.x; dx++) {
            row += texel(src, p + vec2<i32>(dx, dy)) * pass_f(12u + u32(abs(dx)));
        }
        blurred += row * pass_f(column + u32(abs(dy)));
    }
    var acc = lumen_mix(blurred);
    if (pass_u(2u) != 0u) {
        acc += lumen_upsample(aux, p, vec2<bool>(pass_u(3u) != 0u, pass_u(4u) != 0u));
    }
    store(id.xy, acc);
}

// dither.rs: hash, triangular_noise and Dither::apply.
fn lumen_hash(x: u32, y: u32, seed: u32) -> u32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    return h ^ (h >> 16u);
}

fn lumen_dither(c: vec4<f32>, p: vec2<u32>) -> vec4<f32> {
    let room = clamp(c.rgb * 255.0, vec3<f32>(0.0), vec3<f32>(1.0)) * clamp((1.0 - c.rgb) * 255.0, vec3<f32>(0.0), vec3<f32>(1.0));
    if (all(room == vec3<f32>(0.0))) {
        return c;
    }
    let h = lumen_hash(p.x - pass_u(5u), p.y - pass_u(6u), pass_u(4u));
    let n = (f32(h & 0xffffu) - f32(h >> 16u)) * (1.0 / 65536.0) * (1.0 / 255.0);
    return vec4<f32>(c.rgb + n * room, c.a);
}

// Pass::Composite (passes/composite.rs): source over glow per View and Blend, then encode and dither.
@compute @workgroup_size(16, 16)
fn lumen_composite(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let source = texel(src, p);
    var spread = vec4<f32>(0.0);
    if (pass_u(7u) != 0u) {
        spread = lumen_upsample(aux, p, vec2<bool>(true));
    }
    let view = pass_u(0u);
    let srgb = pass_u(2u) != 0u;
    var light = vec3<f32>(0.0);
    var alpha = 0.0;
    if (all(source == vec4<f32>(0.0))) {
        if (view != 2u) {
            light = spread.rgb * pass_f(14u);
        }
    } else {
        let linear = lumen_decode(source, srgb);
        let emission = lumen_knee(linear, pass_f(12u), pass_f(13u));
        let glow = (lumen_mix(emission) + spread) * pass_f(14u);
        if (view == 2u) {
            light = emission.rgb;
        } else if (view == 1u) {
            light = glow.rgb;
        } else {
            let base = linear * pass_f(15u);
            light = base.rgb + glow.rgb;
            if (pass_u(1u) == 1u) {
                light = base.rgb + glow.rgb * (1.0 - clamp(base.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
            }
            alpha = base.a;
        }
    }
    var c = vec4<f32>(lumen_encode(light, srgb), alpha);
    if (pass_u(3u) != 0u) {
        c = lumen_dither(c, id.xy);
    }
    store(id.xy, vec4<f32>(c.rgb, max(c.a, clamp(max(max(c.r, c.g), c.b), 0.0, 1.0))));
}
