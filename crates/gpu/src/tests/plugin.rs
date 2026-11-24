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
        version: "1.0.0".into(),
        files: None,
        margin: None,
        state_version: 0,
        migrate: None,
        param_state: None,
        aux: Vec::new(),
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
        gpu: Some(Box::new(move |_, _, _| Some(program.clone()))),
        time_dependent: false,
        reads_other_frames: false,
        reads_comp_scene: false,
        reads_param_history: false,
        presets: Vec::new(),
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
            let image = crate::plugin_draw::Stage::new("test.gpu.reshape", 1, "", p, None).texture(&g.ctx, 0).expect("the texture uploads");
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

const SPIN_WGSL: &str = r#"
@compute @workgroup_size(16, 16)
fn spin(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    var n = 0.0;
    loop {
        n = n + 1.0;
        if (pass_f(0u) < 0.0) { break; }
    }
    store(id.xy, vec4<f32>(n / 1048576.0));
}
"#;

const COUNT_WGSL: &str = r#"
fn count(k: u32) -> f32 {
    var n = 0.0;
    for (var i = 0u; i < k; i++) {
        for (var j = 0u; j < k; j++) { n += 1.0; }
    }
    return n;
}

@compute @workgroup_size(16, 16)
fn spin(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    store(id.xy, vec4<f32>(count(pass_u(0u)) / 1000000.0));
}
"#;

fn small(id: &str) -> Option<Buf> {
    let g = gpu()?;
    let spec = effectcraft_effects::find(id).expect("registered");
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [4.0, 4.0], seed: 1, adjustment: false, env: Default::default() };
    g.effects(&[FxStep { spec, ctx }], &Buf { img: pattern(2, 4, 4), offset: [0.0, 0.0], scale: 1.0 }, None)
}

fn spin_program(u: Vec<u32>) -> GpuProgram {
    GpuProgram { padding: [3; 4], passes: vec![GpuPass { u, ..pass("spin", GpuSlot::Source, GpuSlot::Output) }], ..Default::default() }
}

#[test]
fn a_loop_that_never_ends_stops_at_the_cap_then_its_shader_is_turned_off_and_reported() {
    let Some(g) = gpu() else { return };
    register("test.gpu.spin", SPIN_WGSL, spin_program(Vec::new()));
    assert!(small("test.gpu.spin").is_none(), "the first frame, whose loop was cut at the cap, renders on the CPU");
    assert!(small("test.gpu.spin").is_none(), "the next frame declines before it records");
    let registered = dynamic::get("test.gpu.spin").expect("registered");
    let stopped = registered.gpu_stopped.get().expect("the effect carries the reason");
    assert!(stopped.starts_with("a loop in this shader ran past 1048576 iterations in one invocation"), "{stopped}");
    assert!(g.ctx.plugin_reports("test.gpu.spin").contains(stopped), "the GPU log names it");
    assert!(g.ctx.check_health().is_ok(), "the device stays usable");
    register("test.gpu.after-spin", TINT_WGSL, tint_program());
    renders_on_the_gpu_like_the_cpu("test.gpu.after-spin");
}

pub(crate) fn register_spin(id: &str, variant: &str) {
    let mut spin = effect(id, &format!("{SPIN_WGSL}{variant}"), spin_program(Vec::new()));
    spin.time_dependent = true;
    dynamic::register(spin);
}

#[test]
fn a_capped_frame_declines_while_another_encoder_holds_a_flag_read_of_the_same_shader() {
    let Some(g) = gpu() else { return };
    let wgsl = format!("{SPIN_WGSL}\n");
    register("test.gpu.spin-shared", &wgsl, spin_program(Vec::new()));
    let guard = g.ctx.plugin_guard("test.gpu.spin-shared", 1, &wgsl).expect("the shader has a loop guard");
    let mut other = crate::context::Enc::new(&g.ctx);
    other.read_loop_flag(&guard);
    assert!(small("test.gpu.spin-shared").is_none(), "the capped frame renders on the CPU");
}

#[test]
fn a_capped_frame_goes_on_screen_at_once_then_is_turned_away_and_never_read_back() {
    let Some(g) = gpu() else { return };
    register_spin("test.gpu.spin-display", "\n\n");
    crate::fx_plugin::warm(&g.ctx, &dynamic::get("test.gpu.spin-display").expect("registered"));
    let mut scene = crate::tests::Scene::new(effectcraft_project::BitDepth::Bpc32);
    let mut layer = scene.footage(8, 8);
    scene.effect(&mut layer, "test.gpu.spin-display", &[]);
    scene.push(layer);
    let mut r = effectcraft_render::Renderer::new(
        &scene.p,
        &crate::tests::Pattern,
        effectcraft_render::RenderOpts { backend: effectcraft_render::Backend::Gpu, ..crate::tests::opts() },
    );
    r.accel = Some(g);
    let shown = g.render_display(&r, scene.cid, effectcraft_time::Tick::ZERO).expect("the frame goes on screen without waiting for its loop flag");
    let capped = (0..50).find_map(|_| {
        g.poll();
        let read = shown.loop_check.capped();
        if read.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        read
    });
    assert_eq!(capped, Some(true), "its loop flag arrives within a few steps and says capped");
    assert_eq!(g.read_display(&shown), None, "no readback publishes the capped frame");
    let again = g.render_display(&r, scene.cid, effectcraft_time::Tick::ZERO);
    assert!(
        again.is_none_or(|f| {
            g.wait();
            f.loop_check.capped() == Some(false)
        }),
        "the frame renders again with the shader turned off"
    );
}

#[test]
fn loops_under_the_cap_run_every_iteration_frame_after_frame() {
    register("test.gpu.count", COUNT_WGSL, spin_program(vec![1000]));
    let frames: Vec<Buf> = (0..3).map_while(|_| small("test.gpu.count")).collect();
    assert_eq!(frames.len(), 3, "every frame runs on the GPU");
    assert!(frames.iter().all(|frame| frame.img.data.iter().all(|p| *p == [1.0; 4])), "a million nested iterations all ran: {:?}", frames[0].img.data.first());
}

#[test]
fn a_plugin_effect_the_project_uses_is_built_before_its_first_frame_and_an_unused_one_is_not() {
    gpu();
    let Some(g) = crate::Gpu::headless() else { return };
    let flip = || GpuProgram { passes: vec![pass("flip", GpuSlot::Source, GpuSlot::Output)], ..Default::default() };
    register("test.gpu.warm", FLIP_WGSL, flip());
    register("test.gpu.unused", FLIP_WGSL, flip());
    let mut scene = crate::tests::Scene::new(effectcraft_project::BitDepth::Bpc32);
    let mut layer = scene.solid([1.0; 3], 8, 8);
    scene.effect(&mut layer, "test.gpu.warm", &[]);
    scene.push(layer);
    assert!(!g.ctx.plugin_shader_cached("test.gpu.warm"), "a fresh context has built nothing");
    g.warm_plugins(&scene.p).expect("a newly used plugin effect starts a warm-up").join().expect("the warm-up finishes");
    assert!(g.ctx.plugin_shader_cached("test.gpu.warm"), "the shader was built off the frame path");
    assert!(!g.ctx.plugin_shader_cached("test.gpu.unused"), "an effect the project does not use was built");
    assert!(g.warm_plugins(&scene.p).is_none(), "an unchanged project warms again");
}

const TRIANGLE_WGSL: &str = r#"
@vertex
fn vs_tri(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(p, 0.0, 1.0);
}

@fragment
fn fs_tri() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0);
}
"#;

#[test]
fn warming_up_leaves_the_buffers_of_a_live_render_alone() {
    let Some(g) = gpu() else { return };
    let triangles = |n: usize| [-1.0f32, -1.0, 3.0, -1.0, -1.0, 3.0].repeat(n).iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let draw = dynamic::GpuDraw { layout: vec![2], clear: true, ..Default::default() };
    let mut effect = effect("test.gpu.warm-draw", TRIANGLE_WGSL, GpuProgram::default());
    effect.gpu = Some(Box::new(move |ctx: &EffectCtx, _, _| {
        Some(GpuProgram {
            buffers: vec![dynamic::GpuBytes { name: "mesh".into(), bytes: triangles(if ctx.seed == 1 { 1 } else { 2 }).into() }],
            passes: vec![GpuPass { draw: Some(draw.clone()), ..pass("tri", GpuSlot::Source, GpuSlot::Output) }],
            ..Default::default()
        })
    }));
    dynamic::register(effect);
    assert!(small("test.gpu.warm-draw").is_some(), "the live frame draws on the GPU");
    assert_eq!(g.ctx.plugin_draw.kept_bytes("test.gpu.warm-draw", "mesh"), Some(24), "the live render keeps its one triangle");
    crate::fx_plugin::warm(&g.ctx, &dynamic::get("test.gpu.warm-draw").expect("registered"));
    assert_eq!(g.ctx.plugin_draw.kept_bytes("test.gpu.warm-draw", "mesh"), Some(24), "the warm-up's two triangles do not replace it");
}
