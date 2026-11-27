use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectSpec, Params};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::raster::Image;
use effectcraft_engine::render::{Accelerator, FxStep};
use effectcraft_gpu::Gpu;

const SIZE: [u32; 2] = [1920, 1080];
const EFFECT: &str = "org.craftkit.halo.flare";
const TOLERANCE: f32 = 2e-3;
const LOOKS: [&str; 6] = ["Sun", "Anamorphic Blue", "Warm Studio", "Sci-Fi Ring", "Vintage Zoom", "Night Street"];

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    match GPU.get_or_init(Gpu::try_headless) {
        Ok(gpu) => Some(gpu),
        Err(why) if std::env::var("EFFECTCRAFT_REQUIRE_GPU").is_ok_and(|v| v == "1") => panic!("no GPU: {why}"),
        Err(why) => {
            eprintln!("no GPU ({why}); skipping the Halo Flares GPU comparison");
            None
        }
    }
}

fn bundle() -> &'static Path {
    static BUNDLE: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, path) = BUNDLE.get_or_init(|| {
        let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(plugin.join("Cargo.toml"))
            .env_remove("CARGO_TARGET_DIR")
            .status()
            .expect("cargo runs");
        assert!(status.success(), "Halo Flares builds");
        let dir = tempfile::tempdir().expect("temp dir");
        let bundle = dir.path().join("org.craftkit.halo");
        std::fs::create_dir_all(&bundle).expect("bundle dir");
        std::fs::copy(plugin.join("target/wasm32-unknown-unknown/release/halo.wasm"), bundle.join("plugin.wasm")).expect("copy the wasm");
        std::fs::copy(plugin.join("plugin.json"), bundle.join("plugin.json")).expect("copy the manifest");
        (dir, bundle)
    });
    path
}

fn spec() -> &'static EffectSpec {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| {
        let config = tempfile::tempdir().expect("temp dir").keep();
        let mut session = effectcraft_host::session();
        let plugins = effectcraft_plugin::install(&mut session, Some(&config));
        plugins.load_for_session(bundle()).expect("Halo Flares loads");
    });
    effects::find(EFFECT).expect("Halo Flares registered its effect")
}

fn gradient() -> Buf {
    let [w, h] = SIZE;
    let mut img = Image::new(w, h);
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            img.set(x, y, [0.015 + 0.04 * v, 0.02 + 0.055 * v, 0.06 + 0.12 * v, 1.0]);
        }
    }
    Buf { img, offset: [0.0, 0.0], scale: 1.0 }
}

fn params(spec: &EffectSpec, values: &[(&str, f64)]) -> Params {
    let size = [f64::from(SIZE[0]), f64::from(SIZE[1])];
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
    for (id, value) in values {
        assert!(params.values.contains_key(*id), "Halo Flares has a parameter {id}");
        params.values.insert(id.to_string(), Value::Scalar(*value));
    }
    params
}

fn ctx(params: &Params, time: f64) -> EffectCtx<'_> {
    EffectCtx { params, time, layer_size: [f64::from(SIZE[0]), f64::from(SIZE[1])], seed: 5, adjustment: false, env: Default::default() }
}

fn worst(a: &Image, b: &Image) -> f32 {
    a.data
        .iter()
        .zip(&b.data)
        .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| if x.is_finite() && y.is_finite() { (x - y).abs() } else { f32::INFINITY }))
        .fold(0.0, f32::max)
}

#[test]
fn every_look_renders_on_the_gpu_as_its_cpu_render_does_at_1080p() {
    let spec = spec();
    let Some(gpu) = gpu() else { return };
    assert!(spec.gpu && gpu.supports_effect(spec.id), "Halo Flares is offered to the GPU");
    for (n, name) in LOOKS.iter().enumerate() {
        for (time, blend) in [(0.0, 0.0), (1.7, 1.0)] {
            let params = params(spec, &[("look", n as f64 + 1.0), ("Composite/blend", blend)]);
            let start = Instant::now();
            let cpu = effects::apply(spec, &ctx(&params, time), gradient());
            let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert!(worst(&cpu.img, &gradient().img) > 0.5, "{name}: the flare changes the picture");
            let mut gpu_ms = f64::MAX;
            let mut out = None;
            for _ in 0..5 {
                let start = Instant::now();
                out = gpu.effects(&[FxStep { spec, ctx: ctx(&params, time) }], &gradient(), None);
                gpu_ms = gpu_ms.min(start.elapsed().as_secs_f64() * 1000.0);
            }
            let out = out.unwrap_or_else(|| panic!("{name}: the GPU ran the plan"));
            assert_eq!((out.img.width, out.img.height), (cpu.img.width, cpu.img.height), "{name}: frame size");
            let difference = worst(&cpu.img, &out.img);
            eprintln!("halo {name} t={time} blend={blend}: worst {difference:.2e}, cpu {cpu_ms:.1} ms, gpu round trip {gpu_ms:.1} ms");
            assert!(difference < TOLERANCE, "{name}: GPU and CPU differ by {difference}");
        }
    }
}
