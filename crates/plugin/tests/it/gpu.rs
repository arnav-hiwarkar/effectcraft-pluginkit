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
fn a_gpu_plan_past_its_budget_shows_why_in_the_effect_error() {
    let app = app("test.gpu-budget", &[]);
    let Ok(gpu) = Gpu::try_headless() else { return };
    let spec = effects::find(&format!("{}.gpu-invert", app.id)).expect("the probe registered its GPU effects");
    let values = params(spec, &[("amount", 0.8)]);
    gpu.context().set_plugin_gpu_budget(std::time::Duration::ZERO);
    assert!(gpu.effects(&[FxStep { spec, ctx: ctx(&values) }], &picture(), None).is_none(), "the plan is stopped");
    let why = app.plugins.render_error(spec.id).expect("Effect Controls shows why the effect left the GPU");
    assert!(why.contains(spec.id) && why.contains("past the 0 ms budget") && why.contains("until the plugin is reloaded"), "{why}");
    assert!(!gpu.supports_effect(spec.id), "the next frame renders on the CPU");
}
