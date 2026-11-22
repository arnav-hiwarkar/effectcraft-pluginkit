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
    GpuPass { kernel: kernel.into(), src, aux: None, out, f: TINT.to_vec(), u: Vec::new(), data: Vec::new(), draw: None }
}

fn tint_program() -> GpuProgram {
    GpuProgram { padding: 3, passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() }
}

fn register(id: &str, wgsl: &str, program: GpuProgram) {
    dynamic::register(effect(id, wgsl, program));
}

fn effect(id: &str, wgsl: &str, program: GpuProgram) -> DynamicEffect {
    DynamicEffect {
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
    let flip =
        GpuPass { kernel: "flip".into(), src: GpuSlot::Source, aux: None, out: GpuSlot::Output, f: Vec::new(), u: Vec::new(), data: Vec::new(), draw: None };
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

struct Budget(&'static crate::Gpu);

impl Drop for Budget {
    fn drop(&mut self) {
        self.0.ctx.set_plugin_gpu_budget(std::time::Duration::from_secs(2));
    }
}

#[test]
fn a_frame_past_the_budget_stops_the_registration_until_it_is_registered_again() {
    let Some(g) = gpu() else { return };
    let _restore = Budget(g);
    let id = "test.gpu.zero-budget";
    register(id, TINT_WGSL, GpuProgram { passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() });
    g.ctx.set_plugin_gpu_budget(std::time::Duration::ZERO);
    assert!(run_sized(id, [64, 64]).is_none(), "a 64x64 pass past a zero budget is stopped");
    let stopped = dynamic::get(id).and_then(|r| r.gpu_stopped.get().cloned()).expect("the registration is stopped");
    assert!(stopped.contains(id) && stopped.contains(r#"passes ["tint"]"#) && stopped.contains("past the 0 ms budget"), "{stopped}");
    assert_eq!(g.ctx.plugin_reports(id), vec![stopped]);
    g.ctx.set_plugin_gpu_budget(std::time::Duration::from_secs(2));
    assert!(!g.supports_effect(id), "the next frame is rendered on the CPU");
    assert!(run_sized(id, [64, 64]).is_none(), "a frame already in flight declines too");
    register(id, TINT_WGSL, GpuProgram { passes: vec![pass("tint", GpuSlot::Source, GpuSlot::Output)], ..Default::default() });
    assert!(g.supports_effect(id), "registering the same effect again clears the stop");
    assert!(run_sized(id, [64, 64]).is_some(), "the reloaded effect runs on the GPU");
}

const SPIN_WGSL: &str = r#"
@compute @workgroup_size(16, 16)
fn spin(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    var x = f32(id.x) * 0.001 + f32(id.y) * 0.0007;
    for (var i = 0u; i < pass_u(0); i = i + 1u) {
        x = fract(sin(x * 12.9898 + 78.233) * 43758.547);
    }
    store(id.xy, vec4<f32>(x, x, x, 1.0));
}
"#;

#[test]
fn work_queued_before_a_plugin_is_not_charged_to_it() {
    let Some(g) = gpu() else { return };
    let _restore = Budget(g);
    let spin = g.ctx.plugin_pipelines("test.gpu.spin", 1, SPIN_WGSL, &["spin"]).expect("the spin kernel builds").remove(0);
    let mut p = crate::context::Params::default();
    p.u[0][0] = 20_000;
    let queue_spin = |e: &mut crate::context::Enc| {
        let (src, out) = (e.zeros(512, 512), e.image(512, 512));
        e.dispatch(
            crate::context::Kernel::Plugin { pipeline: &spin, label: "spin" },
            &p,
            crate::context::KernelImages { src: &src, aux: None, out: &out },
            None,
            (32, 32),
        );
    };
    let mut e = crate::context::Enc::new(&g.ctx);
    queue_spin(&mut e);
    let began = std::time::Instant::now();
    e.submit();
    g.ctx.poll_wait(e.last_submission().expect("submitted"), std::time::Duration::from_secs(30));
    let slow = began.elapsed();
    assert!(slow > std::time::Duration::from_millis(40), "the queued work is slow enough to matter: {slow:?}");
    register("test.gpu.cheap", TINT_WGSL, tint_program());
    let cheap = dynamic::get("test.gpu.cheap").expect("registered");
    g.ctx.set_plugin_gpu_budget(slow / 2);
    let mut e = crate::context::Enc::new(&g.ctx);
    queue_spin(&mut e);
    let frame = crate::effects::GBuf { img: g.ctx.upload_image(&pattern(5, 53, 37)).expect("uploads"), offset: [0.0, 0.0], scale: 1.0 };
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [53.0, 37.0], seed: 1, adjustment: false, env: Default::default() };
    assert!(crate::fx_plugin::apply(&mut e, &cheap, &ctx, frame).is_some(), "the cheap plugin runs");
    assert_eq!(cheap.gpu_stopped.get(), None, "the slow work queued before it is not charged to the cheap plugin");
}

fn run_sized(id: &str, size: [u32; 2]) -> Option<Buf> {
    let g = gpu()?;
    let spec = effectcraft_effects::find(id).expect("registered");
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: size.map(f64::from), seed: 1, adjustment: false, env: Default::default() };
    let buf = Buf { img: pattern(7, size[0], size[1]), offset: [0.0, 0.0], scale: 1.0 };
    g.effects(&[FxStep { spec, ctx }], &buf, None)
}

#[test]
fn stopped_plugin_effects_are_left_out_of_gpu_runs_before_any_upload() {
    use effectcraft_render::{Backend, RenderOpts, Renderer};
    let Some(g) = gpu() else { return };
    for id in ["test.gpu.stopped.a", "test.gpu.stopped.b"] {
        register(id, TINT_WGSL, tint_program());
        dynamic::get(id).expect("registered").gpu_stopped.set("stopped by the test".into()).expect("not stopped yet");
    }
    let mut s = super::Scene::new(effectcraft_project::BitDepth::Bpc32);
    let mut layer = s.solid([1.0, 0.5, 0.25], 80, 50);
    for id in ["test.gpu.stopped.a", "test.gpu.stopped.b", "ec.color.levels"] {
        s.effect(&mut layer, id, &[]);
    }
    s.push(layer);
    let mut r = Renderer::new(&s.p, &super::Pattern, RenderOpts { backend: Backend::Gpu, ..super::opts() });
    r.accel = Some(g);
    let ctx = r.eval_ctx(s.cid, effectcraft_time::Tick::ZERO).expect("the comp evaluates");
    let mut e = crate::context::Enc::new(&g.ctx);
    let img = e.image(80, 50);
    let before = g.ctx.transfer_stats();
    let mut fx = crate::effects::GpuFx::new(&mut e, img, [0.0, 0.0], 1.0, None);
    r.run_effects_on(&ctx, &ctx.comp.layers[0], true, &mut fx);
    fx.finish().expect("the stack runs");
    let after = g.ctx.transfer_stats();
    assert_eq!((after.readbacks - before.readbacks, after.uploads - before.uploads), (1, 1), "one readback for the CPU effects, one upload for levels");
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
