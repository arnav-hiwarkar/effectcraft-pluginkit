
const KERNELS: &str = r#"
@compute @workgroup_size(16, 16)
fn {{snake}}_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    store(id.xy, vec4<f32>(c.rgb * pass_f(0u), c.a));
}
"#;
