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

const SPIN_SIZE: [u32; 2] = [1024, 1024];
const HEAVY: u32 = 100_000;

fn spin_program(iterations: u32) -> GpuProgram {
    let spin = GpuPass { kernel: "spin".into(), src: GpuSlot::Source, aux: None, out: GpuSlot::Output, f: Vec::new(), u: vec![iterations], draw: None };
    GpuProgram { passes: vec![spin], ..Default::default() }
}

fn run_sized(id: &str, size: [u32; 2]) -> Option<Buf> {
    let g = gpu()?;
    let spec = effectcraft_effects::find(id).expect("registered");
    let params = Params::default();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: size.map(f64::from), seed: 1, adjustment: false, env: Default::default() };
    let buf = Buf { img: pattern(7, size[0], size[1]), offset: [0.0, 0.0], scale: 1.0 };
    g.effects(&[FxStep { spec, ctx }], &buf, None)
}

struct Budget(&'static crate::Gpu);

impl Drop for Budget {
    fn drop(&mut self) {
        self.0.ctx.set_plugin_gpu_budget(std::time::Duration::from_secs(2));
    }
}

#[test]
fn plugin_work_runs_in_bounded_pieces_and_stops_past_its_budget() {
    let Some(g) = gpu() else { return };
    let _restore = Budget(g);
    register("test.gpu.spin.light", SPIN_WGSL, spin_program(4));
    let light = run_sized("test.gpu.spin.light", SPIN_SIZE).expect("a light kernel runs");
    let row = |y: u32| light.img.data[(y * SPIN_SIZE[0] + 3) as usize];
    assert!((0..SPIN_SIZE[1]).all(|y| row(y)[3] == 1.0), "every band of the frame is written");
    register("test.gpu.spin.whole", SPIN_WGSL, spin_program(HEAVY));
    register("test.gpu.spin.heavy", SPIN_WGSL, spin_program(HEAVY));
    g.ctx.set_plugin_gpu_budget(std::time::Duration::from_secs(30));
    let began = std::time::Instant::now();
    assert!(run_sized("test.gpu.spin.whole", SPIN_SIZE).is_some(), "the heavy plan runs within a long budget");
    let whole = began.elapsed();
    g.ctx.set_plugin_gpu_budget(std::time::Duration::from_millis(40));
    let began = std::time::Instant::now();
    assert!(run_sized("test.gpu.spin.heavy", SPIN_SIZE).is_none(), "a plan past its budget is stopped");
    let stopped_after = began.elapsed();
    let reports = g.ctx.plugin_reports("test.gpu.spin.heavy");
    assert!(
        reports.iter().any(|why| why.contains("test.gpu.spin.heavy") && why.contains("pass 0 (`spin`)") && why.contains("40 ms")),
        "the error names the plugin effect, the pass and the budget: {reports:?}"
    );
    assert!(stopped_after < whole / 2, "stopped after {stopped_after:?}; the whole plan takes {whole:?}");
    let began = std::time::Instant::now();
    assert!(run_sized("test.gpu.spin.heavy", SPIN_SIZE).is_none(), "the effect stays on the CPU until the plugin reloads");
    assert!(began.elapsed() < std::time::Duration::from_millis(500), "the next frame declines at once: {:?}", began.elapsed());
    assert!(g.ctx.check_health().is_ok(), "the device stays usable");
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
