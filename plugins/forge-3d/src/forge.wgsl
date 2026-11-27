struct ForgeVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tint: vec4<f32>,
}

fn forge_vec3(first: u32) -> vec3<f32> {
    return vec3<f32>(pass_f(first), pass_f(first + 1u), pass_f(first + 2u));
}

fn forge_vec4(first: u32) -> vec4<f32> {
    return vec4<f32>(forge_vec3(first), pass_f(first + 3u));
}

@vertex
fn vs_forge(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) tint: vec4<f32>,
) -> ForgeVertex {
    let model = mat4x4<f32>(m0, m1, m2, m3);
    let world = model * vec4<f32>(position, 1.0);
    let view_projection = mat4x4<f32>(forge_vec4(0u), forge_vec4(4u), forge_vec4(8u), forge_vec4(12u));
    var v: ForgeVertex;
    v.clip = view_projection * world;
    v.world = world.xyz;
    v.normal = (model * vec4<f32>(normal, 0.0)).xyz;
    v.tint = tint;
    return v;
}

@fragment
fn fs_forge(v: ForgeVertex) -> @location(0) vec4<f32> {
    let view = normalize(forge_vec3(16u) - v.world);
    var n = normalize(v.normal);
    if (dot(n, view) < 0.0) {
        n = -n;
    }
    var diffuse = vec3<f32>(pass_f(19u));
    var shine = vec3<f32>(0.0);
    let lights = min(pass_u(0u), 6u);
    for (var i = 0u; i < lights; i++) {
        let at = 24u + i * 12u;
        let kind = pass_f(at + 3u);
        let colour = forge_vec3(at + 8u);
        if (kind < 0.5) {
            diffuse += colour;
            continue;
        }
        let direction = forge_vec3(at + 4u);
        var to_light = -direction;
        var reach = 1.0;
        if (kind < 2.5) {
            to_light = normalize(forge_vec3(at) - v.world);
            if (kind > 1.5) {
                reach = smoothstep(pass_f(at + 7u), pass_f(at + 11u), dot(-to_light, direction));
            }
        }
        let facing = max(dot(n, to_light), 0.0);
        diffuse += colour * facing * reach;
        let half_way = normalize(to_light + view);
        shine += colour * pow(max(dot(n, half_way), 0.0), pass_f(21u)) * pass_f(20u) * reach * step(0.0001, facing);
    }
    let rim = pass_f(22u) * pow(1.0 - max(dot(n, view), 0.0), 3.0);
    let rgb = v.tint.rgb * (diffuse + rim) + shine;
    return vec4<f32>(rgb * v.tint.a, v.tint.a);
}

@compute @workgroup_size(16, 16)
fn forge_over(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let below = texel(src, vec2<i32>(id.xy)) * pass_f(0u);
    let model = sample_uv(aux, out_uv(id.xy));
    store(id.xy, model + below * (1.0 - model.a));
}
