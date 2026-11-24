use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectSpec, Params};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::raster::Image;
use effectcraft_engine::render::{Accelerator, FxStep};
use effectcraft_gpu::Gpu;

use crate::app;

const SIZE: [u32; 2] = [61, 43];

pub(crate) fn gpu() -> Option<&'static Gpu> {
    static GPU: std::sync::OnceLock<Result<Gpu, String>> = std::sync::OnceLock::new();
    match GPU.get_or_init(Gpu::try_headless) {
        Ok(gpu) => Some(gpu),
        Err(why) if std::env::var("EFFECTCRAFT_REQUIRE_GPU").is_ok_and(|v| v == "1") => panic!("no GPU: {why}"),
        Err(why) => {
            eprintln!("no GPU ({why}); skipping the plugin GPU comparison");
            None
        }
    }
}

fn picture() -> Buf {
    let [w, h] = SIZE;
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let alpha = if (x / 5 + y / 3) % 4 == 0 { 0.5 } else { 1.0 } * (1.0 - u * v * 0.6);
            let checker = if (x / 4 + y / 4) % 2 == 0 { 0.9 } else { 0.1 };
            img.set(x, y, [u * alpha, checker * alpha, (1.0 - v) * alpha, alpha]);
        }
    }
    Buf { img, offset: [0.0, 0.0], scale: 1.0 }
}

fn params(spec: &EffectSpec, values: &[(&str, f64)]) -> Params {
    let size = [f64::from(SIZE[0]), f64::from(SIZE[1])];
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
    for (id, value) in values {
        params.values.insert(id.to_string(), Value::Scalar(*value));
    }
    params
}

fn ctx(params: &Params) -> EffectCtx<'_> {
    EffectCtx { params, time: 0.0, layer_size: [f64::from(SIZE[0]), f64::from(SIZE[1])], seed: 3, adjustment: false, env: Default::default() }
}

fn worst_difference(a: &Image, b: &Image) -> f32 {
    a.data
        .iter()
        .zip(&b.data)
        .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| if x.is_nan() || y.is_nan() { f32::INFINITY } else { (x - y).abs() }))
        .fold(0.0, f32::max)
}

#[test]
fn probe_gpu_effects_render_on_the_gpu_as_their_cpu_render_does() {
    let app = app("test.gpu-probe", &[]);
    let Some(gpu) = gpu() else { return };
    let chain = [("gpu-invert", vec![("amount", 0.8)]), ("gpu-soften", vec![("strength", 0.6)])];
    let specs: Vec<&'static EffectSpec> =
        chain.iter().map(|(local, _)| effects::find(&format!("{}.{local}", app.id)).expect("the probe registered its GPU effects")).collect();
    let all_params: Vec<Params> = specs.iter().zip(&chain).map(|(spec, (_, values))| params(spec, values)).collect();
    for (spec, params) in specs.iter().zip(&all_params) {
        assert!(spec.gpu && gpu.supports_effect(spec.id), "{}: offered to the GPU", spec.id);
        let cpu = effects::apply(spec, &ctx(params), picture());
        assert_eq!(app.plugins.render_error(spec.id), None, "{}: the CPU render succeeded", spec.id);
        let mut unchanged = picture();
        unchanged.pad(cpu.offset[0] as u32);
        assert!(worst_difference(&cpu.img, &unchanged.img) > 0.05, "{}: the effect changes the picture", spec.id);
        let out = gpu.effects(&[FxStep { spec, ctx: ctx(params) }], &picture(), None).unwrap_or_else(|| panic!("{}: the GPU ran the plan", spec.id));
        assert_eq!((out.offset, out.img.width, out.img.height), (cpu.offset, cpu.img.width, cpu.img.height), "{}: frame geometry", spec.id);
        let worst = worst_difference(&cpu.img, &out.img);
        assert!(worst < 1e-3, "{}: GPU and CPU differ by {worst}", spec.id);
    }
    let steps: Vec<FxStep> = specs.iter().zip(&all_params).map(|(spec, params)| FxStep { spec, ctx: ctx(params) }).collect();
    let batched = gpu.effects(&steps, &picture(), None).expect("the chain runs on the GPU in one batch");
    let cpu = specs.iter().zip(&all_params).fold(picture(), |buf, (spec, params)| effects::apply(spec, &ctx(params), buf));
    let worst = worst_difference(&cpu.img, &batched.img);
    assert!(worst < 1e-3, "the GPU chain differs from the CPU chain by {worst}");
}

#[test]
fn a_draw_pass_fills_the_frame_and_a_plan_with_an_index_past_its_vertices_is_declined() {
    let app = app("test.gpu-fill", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.gpu-fill", app.id)).expect("the probe registered its draw effect");
    let valid = params(spec, &[]);
    let out = gpu.effects(&[FxStep { spec, ctx: ctx(&valid) }], &picture(), None).expect("the GPU ran the draw pass");
    assert!(out.img.data.iter().all(|pixel| *pixel == [0.25, 0.5, 0.75, 1.0]), "every pixel is the fill colour, the first is {:?}", out.img.data[0]);
    let invalid = params(spec, &[("last_index", 7.0)]);
    assert!(gpu.effects(&[FxStep { spec, ctx: ctx(&invalid) }], &picture(), None).is_none(), "the GPU declined the plan");
    let cpu = effects::apply(spec, &ctx(&invalid), picture());
    assert_eq!(cpu.img.data[0], [0.25, 0.5, 0.75, 1.0]);
}

#[test]
fn a_draw_that_keeps_its_target_draws_over_what_the_passes_before_it_wrote() {
    let app = app("test.gpu-keep", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.gpu-fill", app.id)).expect("the probe registered its draw effect");
    let mut keep = params(spec, &[]);
    keep.values.insert("onto_scratch".into(), Value::Bool(true));
    let out = gpu.effects(&[FxStep { spec, ctx: ctx(&keep) }], &picture(), None).expect("the GPU ran the plan");
    assert_eq!(app.plugins.render_error(spec.id), None);
    assert_eq!([out.img.data[0], out.img.data[5]], [[0.25, 0.125, 0.5, 0.5], [0.5, 0.25, 1.0, 1.0]]);
}

#[test]
fn an_effect_without_wgsl_reaches_the_gpu_at_the_settings_its_plan_allows() {
    let app = app("test.gpu-swap", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.swap", app.id)).expect("the probe registered its swap effect");
    let with = |planned: bool| {
        let mut values = params(spec, &[]);
        values.values.insert("planned".into(), Value::Bool(planned));
        values
    };
    let (planned, plain) = (with(true), with(false));
    let first = [0.0, 0.45, 0.5, 0.5];
    assert_eq!(effects::apply(spec, &ctx(&plain), picture()).img.data[0], [0.5, 0.45, 0.0, 0.5]);
    assert_eq!(effects::apply(spec, &ctx(&planned), picture()).img.data[0], [0.5, 0.45, 0.0, 0.5]);
    assert_eq!(picture().img.data[0], first);
    assert!(spec.gpu && gpu.supports_effect(spec.id), "a frame rendered with a host kernel plan offers the effect to the GPU");
    let out = gpu.effects(&[FxStep { spec, ctx: ctx(&planned) }], &picture(), None).expect("the plan of host kernels runs on the GPU");
    assert_eq!(out.img.data[0], [0.5, 0.45, 0.0, 0.5]);
    assert!(gpu.effects(&[FxStep { spec, ctx: ctx(&plain) }], &picture(), None).is_none(), "without a plan the GPU declines the frame");
}

#[test]
fn a_gpu_pass_reads_the_layer_a_layer_parameter_names() {
    let mut app = app("test.gpu-matte", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.gpu-matte", app.id)).expect("the probe registered its matte effect");
    let unset = params(spec, &[]);
    let missing = gpu.effects(&[FxStep { spec, ctx: ctx(&unset) }], &picture(), None).expect("the GPU ran a plan that reads a layer");
    assert!(missing.img.data.iter().all(|pixel| *pixel == [0.0; 4]), "with no layer the texture is transparent, the first pixel is {:?}", missing.img.data[0]);

    let session = &mut app.session;
    session.execute("file.projectSettings", serde_json::json!({"bitDepth": 32})).expect("32 bit");
    let matte = session.execute("layer.newSolid", serde_json::json!({"name": "Matte", "color": "#ff8000", "width": 20, "height": 10})).expect("layer.newSolid")
        ["layer"]
        .as_u64();
    session.execute("prop.set", serde_json::json!({"layer": matte, "path": "transform/opacity", "value": 0})).expect("prop.set");
    let plate = session.execute("layer.newSolid", serde_json::json!({"name": "Plate", "color": "#0000ff", "width": 64, "height": 36})).expect("layer.newSolid")
        ["layer"]
        .as_u64();
    session.execute("effect.apply", serde_json::json!({"layer": plate, "effect": spec.id})).expect("effect.apply");
    let matte_id = session.active_comp().and_then(|comp| comp.layers.iter().find(|layer| layer.name == "Matte")).expect("the matte layer").id.0;
    session.execute_checked("prop.set", serde_json::json!({"layer": plate, "path": "effects/#1/matte", "value": matte_id})).expect("prop.set");
    let comp = session.active_comp_id().expect("an active comp");
    let render = |backend| {
        let renderer = effectcraft_engine::render::Renderer::new(
            &session.project,
            session.footage.as_ref(),
            effectcraft_engine::render::RenderOpts { backend, ..Default::default() },
        );
        renderer.with_accel(Some(gpu)).comp_frame(comp, effectcraft_engine::time::Tick::from_seconds_f64(0.0))
    };
    let on_gpu = render(effectcraft_engine::render::Backend::Gpu);
    let at = |x: u32, y: u32| on_gpu.data[(y * on_gpu.width + x) as usize];
    assert_eq!(at(32, 18)[3], 1.0, "the matte is centred on the layer: {:?}", at(32, 18));
    assert!(at(32, 18)[0] > 0.9 && at(32, 18)[2] < 0.01, "inside the matte it is orange: {:?}", at(32, 18));
    assert_eq!(at(21, 18), [0.0; 4], "left of the 20 pixel wide matte it is transparent");
    let covered = on_gpu.data.iter().filter(|pixel| pixel[3] > 0.5).count();
    assert_eq!(covered, 20 * 10, "the matte, made invisible in the comp, is drawn only by the effect");
    let on_cpu = render(effectcraft_engine::render::Backend::Cpu);
    let worst = worst_difference(&on_gpu, &on_cpu);
    assert!(worst < 1e-3, "the GPU and CPU renders differ by {worst}");
}

#[test]
fn a_layer_input_stays_on_the_gpu_after_the_first_frame() {
    let mut app = app("test.gpu-layer-frames", &[]);
    if gpu().is_none() {
        return;
    }
    let own = Gpu::try_headless().expect("a second device for counting this test's transfers alone");
    let gpu = &own;
    let spec = effects::find(&format!("{}.gpu-matte", app.id)).expect("the probe registered its matte effect");
    let session = &mut app.session;
    session.execute("file.projectSettings", serde_json::json!({"bitDepth": 8})).expect("8 bit");
    let matte = session.execute("layer.newSolid", serde_json::json!({"name": "Matte", "color": "#ff8000", "width": 20, "height": 10})).expect("layer.newSolid")
        ["layer"]
        .as_u64();
    session.execute("prop.set", serde_json::json!({"layer": matte, "path": "transform/opacity", "value": 0})).expect("prop.set");
    let plate = session.execute("layer.newSolid", serde_json::json!({"name": "Plate", "color": "#0000ff", "width": 64, "height": 36})).expect("layer.newSolid")
        ["layer"]
        .as_u64();
    session.execute("effect.apply", serde_json::json!({"layer": plate, "effect": "ec.color.brightnesscontrast"})).expect("effect.apply");
    session.execute("effect.apply", serde_json::json!({"layer": plate, "effect": spec.id})).expect("effect.apply");
    for (time, value) in [(0.0, 0.0), (1.0, 40.0)] {
        session
            .execute("prop.addKey", serde_json::json!({"layer": plate, "path": "effects/#1/brightness", "time": time, "value": value}))
            .expect("prop.addKey");
    }
    let matte_id = session.active_comp().and_then(|comp| comp.layers.iter().find(|layer| layer.name == "Matte")).expect("the matte layer").id.0;
    session.execute_checked("prop.set", serde_json::json!({"layer": plate, "path": "effects/#2/matte", "value": matte_id})).expect("prop.set");
    let comp = session.active_comp_id().expect("an active comp");
    let cache = effectcraft_engine::render::LayerCache::new(256 << 20);
    let render = |backend, frame: f64| {
        let mut renderer = effectcraft_engine::render::Renderer::new(
            &session.project,
            session.footage.as_ref(),
            effectcraft_engine::render::RenderOpts { backend, ..Default::default() },
        );
        renderer.cache = Some(&cache);
        renderer.with_accel(Some(gpu)).comp_frame(comp, effectcraft_engine::time::Tick::from_seconds_f64(frame / 24.0))
    };
    let first = render(effectcraft_engine::render::Backend::Gpu, 0.0);
    let (before, cached) = (gpu.context().transfer_stats(), cache.stats());
    let second = render(effectcraft_engine::render::Backend::Gpu, 1.0);
    let (after, now) = (gpu.context().transfer_stats(), cache.stats());
    assert_eq!(
        (now.misses - cached.misses, now.hits - cached.hits),
        (1, 1),
        "the second frame renders only the animated plate; the matte comes from the cache"
    );
    assert_eq!((after.uploads - before.uploads, after.upload_bytes - before.upload_bytes), (1, 64 * 36 * 16), "the second frame uploads only the plate");
    assert_eq!(after.plugin_layers_copied, 0, "no frame copies the matte through the CPU");
    let covered = second.data.iter().filter(|pixel| pixel[3] > 0.5).count();
    assert_eq!(covered, 20 * 10, "the matte is drawn by the effect on the second frame");
    for (frame, on_gpu) in [(0.0, &first), (1.0, &second)] {
        let on_cpu = render(effectcraft_engine::render::Backend::Cpu, frame);
        let worst = worst_difference(on_gpu, &on_cpu);
        assert!(worst <= 1.0 / 255.0, "frame {frame}: the GPU and CPU renders differ by {worst}");
    }
}

#[test]
fn a_gpu_pass_reads_a_layer_whose_video_is_off_as_its_pixels() {
    let mut app = app("test.gpu-hidden-matte", &[]);
    let Some(gpu) = gpu() else { return };
    let session = &mut app.session;
    let matte =
        session.execute("layer.newSolid", serde_json::json!({"name": "Matte", "color": "#ff8000", "width": 64, "height": 36})).unwrap()["layer"].as_u64();
    session.execute("layer.setSwitch", serde_json::json!({"layers": [matte], "switch": "video", "value": false})).unwrap();
    let plate =
        session.execute("layer.newSolid", serde_json::json!({"name": "Plate", "color": "#0000ff", "width": 64, "height": 36})).unwrap()["layer"].as_u64();
    session.execute("effect.apply", serde_json::json!({"layer": plate, "effect": format!("{}.gpu-matte", app.id)})).unwrap();
    session.execute_checked("prop.set", serde_json::json!({"layer": plate, "path": "effects/#1/matte", "value": matte})).unwrap();
    let comp = session.active_comp_id().unwrap();
    let render = |backend| {
        let renderer = effectcraft_engine::render::Renderer::new(
            &session.project,
            session.footage.as_ref(),
            effectcraft_engine::render::RenderOpts { backend, ..Default::default() },
        );
        renderer.with_accel(Some(gpu)).comp_frame(comp, effectcraft_engine::time::Tick::from_seconds_f64(0.0))
    };
    for backend in [effectcraft_engine::render::Backend::Gpu, effectcraft_engine::render::Backend::Cpu] {
        let image = render(backend);
        let centre = image.data[(18 * image.width + 32) as usize];
        assert!((centre[0] - 1.0).abs() < 0.02 && (centre[1] - 0.502).abs() < 0.02 && centre[2] < 0.02, "{backend:?}: {centre:?}");
    }
}

#[test]
fn every_host_kernel_renders_on_the_gpu_within_two_levels_of_the_cpu() {
    let app = app("test.gpu-kernels", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.host-kernels", app.id)).expect("the probe registered its host kernel effect");
    assert!(spec.gpu && gpu.supports_effect(spec.id), "a plan of host kernels needs no plugin WGSL to run on the GPU");
    let cases = (0..16).chain(26..33).map(|kernel| (kernel, 0)).chain((1..10).map(|mode| (3, mode)));
    for (kernel, mode) in cases {
        let params = params(spec, &[("kernel", f64::from(kernel)), ("mode", f64::from(mode))]);
        let cpu = effects::apply(spec, &ctx(&params), picture());
        assert_eq!(app.plugins.render_error(spec.id), None, "kernel {kernel} mode {mode}: the CPU render succeeded");
        let mut unchanged = picture();
        unchanged.pad(cpu.offset[0] as u32);
        let least_change = if kernel == 13 { 1e-4 } else { 0.02 };
        assert!(worst_difference(&cpu.img, &unchanged.img) > least_change, "kernel {kernel} mode {mode}: the kernel changes the picture");
        let out =
            gpu.effects(&[FxStep { spec, ctx: ctx(&params) }], &picture(), None).unwrap_or_else(|| panic!("kernel {kernel} mode {mode}: the GPU ran the plan"));
        assert_eq!((out.offset, out.img.width, out.img.height), (cpu.offset, cpu.img.width, cpu.img.height), "kernel {kernel} mode {mode}: geometry");
        let worst = worst_difference(&cpu.img, &out.img);
        eprintln!("host kernel {kernel} mode {mode}: GPU and CPU differ by at most {worst:.2e}");
        assert!(worst < 2.0 / 255.0, "kernel {kernel} mode {mode}: GPU and CPU differ by {worst}");
    }
}

#[test]
fn host_kernel_edge_cases_give_the_same_literal_pixels_on_the_cpu_and_the_gpu() {
    let app = app("test.gpu-kernel-edges", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.host-kernels", app.id)).expect("the probe registered its host kernel effect");
    let big = [16_777_216.0, 0.5, 0.25, 1.0];
    let input = Buf {
        img: Image { width: 4, height: 1, data: vec![[0.0; 4], [1.0 / 16.0, 0.0, 0.0, 1.0 / 16.0], [0.0, 0.0, 0.0, 1.0], big] },
        offset: [0.0, 0.0],
        scale: 1.0,
    };
    let mut padded = input.clone();
    padded.pad(6);
    let on_row = |img: &Image, x: u32| img.data[(6 * img.width + x) as usize];
    let cases = ["a Gaussian too small to matter", "a warp past the image", "a bicubic zoom", "Over at amount 1", "Over at amount 0"];
    let mut wrong = Vec::new();
    for (kernel, case) in (16..).zip(cases) {
        let params = params(spec, &[("kernel", f64::from(kernel))]);
        let cpu = effects::apply(spec, &ctx(&params), input.clone());
        assert_eq!(app.plugins.render_error(spec.id), None, "{case}: the CPU render succeeded");
        let on_gpu = gpu.effects(&[FxStep { spec, ctx: ctx(&params) }], &input, None).unwrap_or_else(|| panic!("{case}: the GPU ran the plan"));
        for (backend, out) in [("CPU", &cpu.img), ("GPU", &on_gpu.img)] {
            let right = (out.width, out.height) == (16, 13)
                && match kernel {
                    16 => out.data == padded.img.data,
                    17 => out.data.iter().all(|p| *p == [0.0; 4]),
                    18 => on_row(out, 13) == [0.0; 4] && out.data.iter().all(|p| p[3] > 0.0 || *p == [0.0; 4]),
                    19 => on_row(out, 9) == [0.25, 0.0, 0.0, 1.0],
                    _ => on_row(out, 9) == big,
                };
            if !right {
                wrong.push(format!("{case} on the {backend}"));
            }
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn extreme_values_and_negative_colour_give_the_same_literal_pixels_on_the_cpu_and_the_gpu() {
    let app = app("test.gpu-kernel-extremes", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.host-kernels", app.id)).expect("the probe registered its host kernel effect");
    let filled = |pixel: [f32; 4]| Buf { img: Image { width: 4, height: 4, data: vec![pixel; 16] }, offset: [0.0, 0.0], scale: 1.0 };
    let cases = [
        ("a curve over 0..1e-36", 21, filled([1e-36, 0.0, 0.0, 1.0]), [6, 6], [1e-36, 0.0, 0.0, 1.0]),
        ("halving 1.25e38", 22, filled([1.25e38, 0.0, 0.0, 1.0]), [3, 3], [1.25e38 * 0.875 * 0.875, 0.0, 0.0, 0.875 * 0.875]),
        ("decoding sRGB 1e16", 23, filled([1e16, 0.0, 0.0, 1.0]), [6, 6], [2f32.powi(126), 0.0, 0.0, 1.0]),
        ("decoding with linear negatives", 24, filled([-0.5, 0.0, 0.0, 1.0]), [6, 6], [-0.5 / 12.92, 0.0, 0.0, 1.0]),
        ("encoding with linear negatives", 25, filled([-0.5, 0.0, 0.0, 1.0]), [6, 6], [-6.46, 0.0, 0.0, 1.0]),
    ];
    let mut wrong = Vec::new();
    for (case, kernel, input, [x, y], want) in cases {
        let params = params(spec, &[("kernel", f64::from(kernel))]);
        let cpu = effects::apply(spec, &ctx(&params), input.clone());
        assert_eq!(app.plugins.render_error(spec.id), None, "{case}: the CPU render succeeded");
        let on_gpu = gpu.effects(&[FxStep { spec, ctx: ctx(&params) }], &input, None).unwrap_or_else(|| panic!("{case}: the GPU ran the plan"));
        for (backend, out) in [("CPU", &cpu.img), ("GPU", &on_gpu.img)] {
            let got = out.data[(y * out.width + x) as usize];
            if !got.iter().zip(want).all(|(g, w)| (g - w).abs() <= w.abs() * 1e-6) {
                wrong.push(format!("{case} on the {backend}: {got:?}, not {want:?}"));
            }
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn plugins_are_told_the_padding_the_app_applies_on_the_cpu_and_the_gpu() {
    let sizes = [[1, 1], [7, 3000], [1920, 1080], [3840, 2160], [16000, 100], [16384, 1], [1, 16384], [4097, 4097], [9000, 9000]];
    let paddings = [[0, 0, 0, 0], [1, 1, 1, 1], [5, 0, 2, 0], [300, 300, 300, 300], [4096, 4096, 4096, 4096], [5000, 7, 5000, 7]];
    for size in sizes {
        for padding in paddings {
            let [l, t, r, b] = padding.map(|side| f64::from(side.min(4096)));
            let grown = effects::grow_sides([-l, -t], [f64::from(size[0]) + r, f64::from(size[1]) + b], size[0], size[1]).unwrap_or_default();
            assert_eq!(craft_plugin_host::applied_padding(padding, size), grown, "{size:?} padded by {padding:?}");
        }
    }
    assert_eq!(craft_plugin_host::applied_padding([300; 4], [16384, 1]), [0; 4], "a frame as wide as the app grows one is not padded");
}

#[test]
fn a_shader_the_gpu_turned_off_is_the_effects_error_and_in_the_plugins_log_once() {
    let app = app("test.gpu-stopped", &[]);
    let spec = effects::find(&format!("{}.gpu-invert", app.id)).expect("the probe registered its GPU effect");
    let registered = effects::dynamic::get(spec.id).expect("registered");
    registered.gpu_stopped.set("a loop ran past the cap".into()).expect("not stopped before");
    let params = params(spec, &[("amount", 0.8)]);
    effects::apply(spec, &ctx(&params), picture());
    effects::apply(spec, &ctx(&params), picture());
    assert_eq!(app.plugins.render_error(spec.id).as_deref(), Some("on the GPU: a loop ran past the cap"));
    let logs: Vec<String> = app.plugins.get(&app.id).expect("loaded").plugin.logs(0).into_iter().map(|line| line.message).collect();
    assert_eq!(logs, ["effect `gpu-invert` on the GPU: a loop ran past the cap"]);
}

#[test]
fn an_effect_without_wgsl_whose_plan_covers_small_frames_is_offered_to_the_gpu() {
    let app = app("test.gpu-small-swap", &[]);
    let Some(gpu) = gpu() else { return };
    let spec = effects::find(&format!("{}.small-swap", app.id)).expect("the probe registered its small swap effect");
    assert!(spec.gpu && gpu.supports_effect(spec.id), "a plan at the probe's frame size offers the effect to the GPU");
    let params = params(spec, &[]);
    let out = gpu.effects(&[FxStep { spec, ctx: ctx(&params) }], &picture(), None).expect("the plan of host kernels runs on the GPU");
    assert_eq!(out.img.data[0], effects::apply(spec, &ctx(&params), picture()).img.data[0]);
    assert_ne!(out.img.data[0], picture().img.data[0], "the effect swapped the picture");
}
