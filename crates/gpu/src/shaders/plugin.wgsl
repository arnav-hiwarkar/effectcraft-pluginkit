// Plugin kernel prelude (docs/plugin-gpu.md). Compiled after common.wgsl, which declares the
// bindings `P`, `src`, `aux` and `data`; this file declares `out`. A plugin's WGSL follows it and
// must not redeclare any of them. Everything below is stable API for plugin kernels.
//
// Each pass runs its kernel over the output image in 16x16 workgroups: declare
// `@compute @workgroup_size(16, 16)` and return early for ids outside `out_size()`.

@group(0) @binding(3) var out: texture_storage_2d<rgba32float, write>;

fn out_size() -> vec2<u32> {
    return textureDimensions(out);
}

fn src_size() -> vec2<u32> {
    return textureDimensions(src);
}

// A pass without `aux` binds a 1x1 transparent image.
fn aux_size() -> vec2<u32> {
    return textureDimensions(aux);
}

fn inside_out(p: vec2<u32>) -> bool {
    return all(p < out_size());
}

// Value `i` of the pass's `floats` / `uints`; 0 past the values the pass gave.
fn pass_f(i: u32) -> f32 {
    return select(0.0, P.f[min(i, 95u) / 4u][i % 4u], i < 96u);
}

fn pass_u(i: u32) -> u32 {
    return select(0u, P.u[min(i, 15u) / 4u][i % 4u], i < 16u);
}

// Texel `p` of `t`; transparent outside it.
fn texel(t: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
    return tex_get(t, p.x, p.y);
}

// Centre of output texel `p` in 0..1 coordinates, which address the same spot in an image of any size.
fn out_uv(p: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(p) + 0.5) / vec2<f32>(out_size());
}

// Bilinear sample of `t` at 0..1 coordinates, whatever its size; edge texels repeat.
fn sample_uv(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<f32>(textureDimensions(t));
    return sample_bilinear_clamped(t, uv.x * d.x, uv.y * d.y);
}

fn store(p: vec2<u32>, c: vec4<f32>) {
    textureStore(out, vec2<i32>(p), c);
}

@compute @workgroup_size(16, 16)
fn craft_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    store(id.xy, texel(src, vec2<i32>(id.xy)));
}

@vertex
fn vs_craft_blit(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1), 0.0, 1.0);
}

@fragment
fn fs_craft_blit(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return texel(src, vec2<i32>(p.xy));
}
