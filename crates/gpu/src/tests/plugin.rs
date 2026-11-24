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
    GpuPass { kernel: kernel.into(), src, aux: None, out, f: TINT.to_vec(), u: Vec::new(), data: Vec::new(), draw: None, host: None }
}

fn tint_program() -> GpuProgram {
    GpuProgram { padding: [3; 4], passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() }
}

fn register(id: &str, wgsl: &str, program: GpuProgram) {
    dynamic::register(effect(id, wgsl, program));
}

fn effect(id: &str, wgsl: &str, program: GpuProgram) -> DynamicEffect {
    DynamicEffect {
        owner: "test".into(),
        files: None,
        margin: None,
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        params: Vec::new(),
        render: Box::new(|ctx: &EffectCtx, mut buf: Buf| {
            buf.pad(3);
            for p in &mut buf.img.data {
                for (c, k) in p.iter_mut().zip(TINT) {
                    *c *= k;
                }
            }
            ctx.quantized(buf)
        }),
        shader: Some(wgsl.into()),
        gpu: Some(Box::new(move |_, _| Some(program.clone()))),
        time_dependent: false,
        reads_other_frames: false,
        reads_comp_scene: false,
        reads_param_history: false,
        presets: Vec::new(),
        groups: Vec::new(),
        revision: 1,
    }
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
fn a_plain_pass_cannot_run_a_built_in_kernel() {
    let Some(g) = gpu() else { return };
    let mut raw = pass("craft_convolve_x", GpuSlot::Source, GpuSlot::Output);
    raw.u = vec![1025];
    register("test.gpu.raw-host", TINT_WGSL, GpuProgram { passes: vec![raw], ..Default::default() });
    assert!(run("test.gpu.raw-host").is_none(), "a plain pass naming a built-in kernel declines");
    let reports = g.ctx.plugin_reports("test.gpu.raw-host");
    assert!(reports.iter().any(|why| why.contains("craft_convolve_x")), "{reports:?}");
}

#[test]
fn plans_the_gpu_cannot_hold_are_declined() {
    let Some(g) = gpu() else { return };
    let finish = pass("tint", GpuSlot::Source, GpuSlot::Output);
    let limit = g.ctx.device.limits().max_texture_dimension_2d;
    let odd_buffer = dynamic::GpuBytes { name: "odd".into(), bytes: vec![0u8; 6].into() };
    let short_texture = dynamic::GpuPicture { name: "short".into(), size: [4, 4], bytes: vec![0u8; 15 * 16].into() };
    let cases = [
        ("too-wide-for-the-device", GpuProgram { images: vec![[limit + 1, 8]], passes: vec![finish.clone()], ..Default::default() }),
        ("odd-buffer", GpuProgram { buffers: vec![odd_buffer], passes: vec![finish.clone()], ..Default::default() }),
        ("short-texture", GpuProgram { textures: vec![short_texture], passes: vec![finish.clone()], ..Default::default() }),
    ];
    for (name, program) in cases {
        let id = format!("test.gpu.limit.{name}");
        register(&id, TINT_WGSL, program);
        assert!(run(&id).is_none(), "{name}: the plan is declined");
    }
    assert!(g.ctx.check_health().is_ok());
    register("test.gpu.limit.ok", TINT_WGSL, GpuProgram { images: vec![[2 * 53, 2 * 37]], passes: vec![finish], ..Default::default() });
    assert!(run("test.gpu.limit.ok").is_some(), "a plan within the device's limits runs");
}

#[test]
fn compile_errors_quote_lines_of_the_plugin_source() {
    let Some(g) = gpu() else { return };
    let wgsl =
        "fn helper() -> f32 {\n    return 1.0;\n}\n@compute @workgroup_size(16, 16) fn tint(@builtin(global_invocation_id) id: vec3<u32>) { let x = ; }\n";
    register("test.gpu.lines", wgsl, tint_program());
    assert!(run("test.gpu.lines").is_none());
    let reports = g.ctx.plugin_reports("test.gpu.lines");
    let report = reports.first().unwrap_or_else(|| panic!("the failure is reported: {reports:?}"));
    assert!(report.contains("wgsl:4:"), "the error points at line 4 of the plugin's source: {report}");
    assert!(report.lines().any(|line| line.trim_start().starts_with("4 │")), "the quoted line is numbered 4: {report}");
}

#[test]
fn the_shader_cache_forgets_an_unloaded_plugin() {
    let Some(g) = gpu() else { return };
    dynamic::register(DynamicEffect { owner: "test.unload".into(), ..effect("test.gpu.unloaded", TINT_WGSL, tint_program()) });
    assert!(run("test.gpu.unloaded").is_some());
    assert!(g.ctx.plugin_shader_cached("test.gpu.unloaded"));
    dynamic::unregister("test.unload");
    register("test.gpu.after-unload", TINT_WGSL, tint_program());
    assert!(run("test.gpu.after-unload").is_some());
    assert!(!g.ctx.plugin_shader_cached("test.gpu.unloaded"), "the unloaded plugin's shader is dropped");
    assert!(g.ctx.plugin_shader_cached("test.gpu.after-unload"));
}

const FLIP_WGSL: &str = r#"
@compute @workgroup_size(16, 16)
fn flip(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    store(vec2<u32>(id.x, out_size().y - 1u - id.y), texel(src, vec2<i32>(id.xy)));
}
"#;

#[test]
fn a_kernel_may_store_each_pixel_anywhere_in_the_output() {
    let Some(g) = gpu() else { return };
    let flip = GpuPass { f: Vec::new(), ..pass("flip", GpuSlot::Source, GpuSlot::Output) };
    register("test.gpu.flip", FLIP_WGSL, GpuProgram { passes: vec![flip], ..Default::default() });
    let spec = effectcraft_effects::find("test.gpu.flip").expect("registered");
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [256.0, 512.0], seed: 1, adjustment: false, env: Default::default() };
    let mut img = effectcraft_raster::Image::new(256, 512);
    for (i, p) in img.data.iter_mut().enumerate() {
        *p = [(i % 256) as f32, (i / 256) as f32, 0.0, 1.0];
    }
    let out = g.effects(&[FxStep { spec, ctx }], &Buf { img, offset: [0.0, 0.0], scale: 1.0 }, None).expect("the flip runs on the GPU");
    let at = |x: usize, y: usize| out.img.data[y * 256 + x];
    assert_eq!(
        [at(0, 0), at(255, 0), at(7, 300), at(255, 511)],
        [[0.0, 511.0, 0.0, 1.0], [255.0, 511.0, 0.0, 1.0], [7.0, 211.0, 0.0, 1.0], [255.0, 0.0, 0.0, 1.0]]
    );
    assert!(out.img.data.iter().enumerate().all(|(i, p)| *p == [(i % 256) as f32, (511 - i / 256) as f32, 0.0, 1.0]), "every row lands flipped");
}

#[test]
fn a_plan_that_fails_leaves_no_work_of_its_own_and_keeps_the_work_before_it() {
    let Some(g) = gpu() else { return };
    let broken = pass("tint", GpuSlot::Scratch(3), GpuSlot::Output);
    register(
        "test.gpu.part-way",
        TINT_WGSL,
        GpuProgram { images: vec![[53, 37]], passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Scratch(0)), broken], ..Default::default() },
    );
    register(
        "test.gpu.padded-missing-entry",
        TINT_WGSL,
        GpuProgram { padding: [3; 4], passes: vec![pass("absent", GpuSlot::Source, GpuSlot::Output)], ..Default::default() },
    );
    register("test.gpu.earlier", TINT_WGSL, GpuProgram { passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() });
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [53.0, 37.0], seed: 1, adjustment: false, env: Default::default() };
    let frame = || crate::effects::GBuf { img: g.ctx.upload_image(&pattern(5, 53, 37)).expect("uploads"), offset: [0.0, 0.0], scale: 1.0 };
    let apply = |e: &mut crate::context::Enc, id: &str, b| crate::fx_plugin::apply(e, &dynamic::get(id).expect("registered"), &ctx, b);
    let mut e = crate::context::Enc::new(&g.ctx);
    assert!(apply(&mut e, "test.gpu.padded-missing-entry", frame()).is_none(), "a plan naming a kernel its shader lacks declines");
    e.submit();
    assert!(e.last_submission().is_none(), "the padding copy is not recorded before the plan is known to run");
    let mut e = crate::context::Enc::new(&g.ctx);
    let tinted = apply(&mut e, "test.gpu.earlier", frame()).expect("the earlier effect runs");
    let earlier = e.recorded();
    assert!(apply(&mut e, "test.gpu.part-way", tinted).is_none(), "a pass reading an image the plan lacks declines");
    assert_eq!((earlier > 0, e.recorded()), (true, earlier), "the earlier effect's work stays and the failed plan's first pass is not recorded");
}

#[test]
fn a_plugin_texture_cached_under_one_size_is_uploaded_again_under_another() {
    let Some(g) = gpu() else { return };
    let bytes: std::sync::Arc<[u8]> = vec![0u8; 64 * 16].into();
    let program = |size: [u32; 2]| GpuProgram { textures: vec![dynamic::GpuPicture { name: "lut".into(), size, bytes: bytes.clone() }], ..Default::default() };
    let sizes: Vec<[u32; 2]> = [program([16, 4]), program([8, 8])]
        .iter()
        .map(|p| {
            let image = crate::plugin_draw::Stage::new("test.gpu.reshape", 1, "", p).texture(&g.ctx, 0).expect("the texture uploads");
            [image.width, image.height]
        })
        .collect();
    assert_eq!(sizes, [[16, 4], [8, 8]]);
}

#[test]
fn the_sixty_fifth_host_module_evicts_only_the_least_recently_used() {
    gpu();
    let Some(g) = crate::Gpu::headless() else { return };
    let source = |i: usize| format!("@compute @workgroup_size(16, 16)\nfn craft_probe_{i}(@builtin(global_invocation_id) id: vec3<u32>) {{}}\n");
    let build = |i: usize| g.ctx.host_pipeline(&source(i), &format!("craft_probe_{i}")).expect("the module builds");
    (0..64).for_each(|i| drop(build(i)));
    drop(build(0));
    drop(build(64));
    let kept: Vec<bool> = [0, 1, 2, 64].iter().map(|i| g.ctx.host_module_kept(&source(*i))).collect();
    assert_eq!(kept, [true, false, true, true], "module 1 was used least recently");
}
