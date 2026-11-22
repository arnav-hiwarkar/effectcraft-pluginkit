use effectcraft_effects::dynamic::{self, DynamicEffect, GpuPass, GpuProgram, GpuSlot};
use effectcraft_effects::{Buf, EffectCtx, Params};
use effectcraft_render::{Accelerator, FxStep};

use super::{diff, gpu, pattern};

const TINT: [f32; 3] = [0.5, 1.0, 0.25];

const TINT_WGSL: &str = r#"
@compute @workgroup_size(16, 16)
fn tint(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let c = texel(src, vec2<i32>(id.xy));
    store(id.xy, vec4<f32>(c.rgb * vec3<f32>(pass_f(0u), pass_f(1u), pass_f(2u)), c.a));
}
"#;

fn pass(kernel: &str, src: GpuSlot, out: GpuSlot) -> GpuPass {
    GpuPass { kernel: kernel.into(), src, aux: None, out, f: TINT.to_vec(), u: Vec::new(), draw: None }
}

fn tint_program() -> GpuProgram {
    GpuProgram { padding: 3, passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() }
}

fn register(id: &str, wgsl: &str, program: GpuProgram) {
    dynamic::register(DynamicEffect {
        owner: "test".into(),
        files: None,
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        params: Vec::new(),
        render: Box::new(|_, mut buf: Buf| {
            buf.pad(3);
            for p in &mut buf.img.data {
                for (c, k) in p.iter_mut().zip(TINT) {
                    *c *= k;
                }
            }
            buf
        }),
        shader: Some(wgsl.into()),
        gpu: Some(Box::new(move |_, _| Some(program.clone()))),
        time_dependent: false,
        reads_other_frames: false,
        reads_comp_scene: false,
        revision: 1,
    });
}

fn input() -> Buf {
    Buf { img: pattern(5, 53, 37), offset: [0.0, 0.0], scale: 1.0 }
}

fn run(id: &str) -> Option<(Buf, Buf)> {
    let g = gpu()?;
    let spec = effectcraft_effects::find(id).expect("registered");
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [53.0, 37.0], seed: 1, adjustment: false, env: Default::default() };
    let cpu = effectcraft_effects::apply(spec, &ctx, input());
    let out = g.effects(&[FxStep { spec, ctx }], &input(), None)?;
    Some((cpu, out))
}

fn renders_on_the_gpu_like_the_cpu(id: &str) {
    let Some(g) = gpu() else { return };
    assert!(g.supports_effect(id), "{id}: a registered effect with a shader is offered to the GPU");
    let (cpu, out) = run(id).unwrap_or_else(|| panic!("{id}: the GPU runs the plugin kernel"));
    assert_eq!((out.offset, out.img.width, out.img.height), (cpu.offset, cpu.img.width, cpu.img.height), "{id}: padded geometry");
    let d = diff(&cpu.img, &out.img, 1e-4);
    assert_eq!(d.over, 0, "{id}: worst {:?}", d.worst);
}

#[test]
fn a_plugin_kernel_renders_the_padded_frame_like_the_cpu() {
    register("test.gpu.tint", TINT_WGSL, tint_program());
    renders_on_the_gpu_like_the_cpu("test.gpu.tint");
}

#[test]
fn a_plugin_shader_that_does_not_compile_declines_to_the_cpu() {
    let Some(g) = gpu() else { return };
    let broken = [
        ("test.gpu.syntax", "@compute @workgroup_size(16, 16) fn tint(@builtin(global_invocation_id) id: vec3<u32>) { let x = ; }"),
        ("test.gpu.redeclared", &format!("@group(0) @binding(3) var out: texture_storage_2d<rgba32float, write>;\n{TINT_WGSL}")),
        ("test.gpu.no-entry", "fn helper() {}"),
        (
            "test.gpu.extra-binding",
            &format!("@group(0) @binding(7) var<storage, read> extra: array<f32>;\n{}", TINT_WGSL.replace("c.a));", "c.a + extra[0]));")),
        ),
    ];
    for (id, wgsl) in broken {
        register(id, wgsl, tint_program());
        for frame in 0..2 {
            assert!(run(id).is_none(), "{id} frame {frame}: a shader that fails validation declines");
        }
        assert!(g.ctx.check_health().is_ok(), "{id}: the device stays usable");
    }
    register("test.gpu.after-broken", TINT_WGSL, tint_program());
    renders_on_the_gpu_like_the_cpu("test.gpu.after-broken");
}

#[test]
fn plans_beyond_the_limits_are_rejected() {
    let Some(g) = gpu() else { return };
    let tint = |src, out| pass("tint", src, out);
    let finish = tint(GpuSlot::Source, GpuSlot::Output);
    let plan = |images: Vec<[u32; 2]>, passes: Vec<GpuPass>| GpuProgram { images, passes, ..Default::default() };
    let too_many_floats = GpuPass { f: vec![0.0; 97], ..finish.clone() };
    let too_many_uints = GpuPass { u: vec![0; 17], ..finish.clone() };
    let cases = [
        ("no-passes", plan(vec![], vec![])),
        ("65-passes", plan(vec![], vec![finish.clone(); 65])),
        ("17-images", plan(vec![[8, 8]; 17], vec![finish.clone()])),
        ("empty-image", plan(vec![[0, 8]], vec![finish.clone()])),
        ("huge-image", plan(vec![[2 * 53 + 1, 8]], vec![finish.clone()])),
        ("unwritten-read", plan(vec![[8, 8]], vec![tint(GpuSlot::Scratch(0), GpuSlot::Output)])),
        ("undeclared-image", plan(vec![], vec![tint(GpuSlot::Source, GpuSlot::Scratch(0)), finish.clone()])),
        ("no-output", plan(vec![[8, 8]], vec![tint(GpuSlot::Source, GpuSlot::Scratch(0))])),
        (
            "reads-own-output",
            plan(vec![[8, 8]], vec![tint(GpuSlot::Source, GpuSlot::Scratch(0)), tint(GpuSlot::Scratch(0), GpuSlot::Scratch(0)), finish.clone()]),
        ),
        ("writes-source", plan(vec![], vec![tint(GpuSlot::Source, GpuSlot::Source), finish.clone()])),
        ("too-much-scratch", plan(vec![[2 * 53, 2 * 37]; 3], vec![finish.clone()])),
        ("too-many-floats", plan(vec![], vec![too_many_floats])),
        ("too-many-uints", plan(vec![], vec![too_many_uints])),
    ];
    for (name, program) in cases {
        let id = format!("test.gpu.limit.{name}");
        register(&id, TINT_WGSL, program);
        assert!(run(&id).is_none(), "{name}: the plan is rejected");
    }
    assert!(g.ctx.check_health().is_ok());
    let mut images = vec![[2 * 53, 2 * 37], [53, 37], [53, 37], [53, 37]];
    images.extend([[1, 1]; 12]);
    let at_the_limits = plan(images, std::iter::repeat_n(finish, 64).collect());
    register("test.gpu.limit.ok", TINT_WGSL, at_the_limits);
    assert!(run("test.gpu.limit.ok").is_some(), "a plan at the limits runs");
}
