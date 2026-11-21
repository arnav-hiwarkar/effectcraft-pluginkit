use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectSpec, Params};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::raster::Image;
use effectcraft_engine::render::{Accelerator, FxStep};
use effectcraft_gpu::Gpu;

const SIZE: [u32; 2] = [160, 96];
const EFFECT: &str = "org.craftkit.lumen-glow.glow";
const MEAN_TOLERANCE: f64 = 0.004;
const WORST_TOLERANCE: f32 = 0.03;

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    match GPU.get_or_init(Gpu::try_headless) {
        Ok(gpu) => Some(gpu),
        Err(why) if std::env::var("EFFECTCRAFT_REQUIRE_GPU").is_ok_and(|v| v == "1") => panic!("no GPU: {why}"),
        Err(why) => {
            eprintln!("no GPU ({why}); skipping the Lumen Glow GPU comparison");
            None
        }
    }
}

fn bundle() -> &'static Path {
    static BUNDLE: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, path) = BUNDLE.get_or_init(|| {
        let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/lumen-glow");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(plugin.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(status.success(), "Lumen Glow builds");
        let dir = tempfile::tempdir().expect("temp dir");
        let bundle = dir.path().join("org.craftkit.lumen-glow");
        std::fs::create_dir_all(&bundle).expect("bundle dir");
        std::fs::copy(plugin.join("target/wasm32-unknown-unknown/release/lumen_glow.wasm"), bundle.join("plugin.wasm")).expect("copy the wasm");
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
        plugins.load_for_session(bundle()).expect("Lumen Glow loads");
    });
    effects::find(EFFECT).expect("Lumen Glow registered its effect")
}

fn picture() -> Buf {
    let [w, h] = SIZE;
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let disc = (fx - 70.0).powi(2) + (fy - 50.0).powi(2) < 18.0f32.powi(2);
            let hot = (fx - 120.0).powi(2) + (fy - 30.0).powi(2) < 4.0f32.powi(2);
            let edge_bar = x < 6 && (20..80).contains(&y);
            let top_line = y < 2 && (90..150).contains(&x);
            let ring = ((fx - 120.0).powi(2) + (fy - 75.0).powi(2)).sqrt();
            let pixel = if hot {
                [12.0, 9.0, 4.0, 1.0]
            } else if disc {
                [2.5, 1.2, 0.4, 1.0]
            } else if edge_bar {
                [0.3, 0.9, 1.0, 1.0]
            } else if top_line {
                [1.6, 1.6, 1.6, 1.0]
            } else if (8.0..11.0).contains(&ring) {
                [0.4, 0.2, 0.9, 0.5]
            } else {
                [0.0; 4]
            };
            img.set(x, y, pixel);
        }
    }
    Buf { img, offset: [0.0, 0.0], scale: 1.0 }
}

fn params(spec: &EffectSpec, values: &[(&str, f64)]) -> Params {
    let size = [f64::from(SIZE[0]), f64::from(SIZE[1])];
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
    for (id, value) in [("Output/dither", 0.0)].iter().chain(values) {
        assert!(params.values.contains_key(*id), "Lumen Glow has a parameter {id}");
        params.values.insert(id.to_string(), Value::Scalar(*value));
    }
    params
}

fn ctx(params: &Params) -> EffectCtx<'_> {
    EffectCtx { params, time: 0.0, layer_size: [f64::from(SIZE[0]), f64::from(SIZE[1])], seed: 11, adjustment: false, env: Default::default() }
}

fn differences(a: &Image, b: &Image) -> (f64, f32) {
    let diffs: Vec<f32> = a
        .data
        .iter()
        .zip(&b.data)
        .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| if x.is_finite() && y.is_finite() { (x - y).abs() } else { f32::INFINITY }))
        .collect();
    let mean = diffs.iter().map(|d| f64::from(*d)).sum::<f64>() / diffs.len().max(1) as f64;
    (mean, diffs.iter().copied().fold(0.0, f32::max))
}

#[test]
fn lumen_glow_renders_on_the_gpu_as_its_cpu_render_does() {
    let spec = spec();
    let Some(gpu) = gpu() else { return };
    assert!(spec.gpu && gpu.supports_effect(spec.id), "Lumen Glow is offered to the GPU");
    let cases: [(&str, Vec<(&str, f64)>); 10] = [
        ("default", vec![]),
        ("large radius", vec![("Glow/radius", 600.0)]),
        ("threshold", vec![("Glow/threshold", 0.9), ("Glow/smoothness", 70.0)]),
        ("aspect", vec![("Glow/aspect", 0.8), ("Glow/radius", 200.0)]),
        ("vertical aspect", vec![("Glow/aspect", -1.0)]),
        ("tint and aberration", vec![("Colour/tint", 1.0), ("Colour/mix", 80.0), ("Aberration/amount", 70.0)]),
        ("glow only", vec![("Output/view", 1.0), ("Glow/falloff", 80.0)]),
        ("screen", vec![("Output/blend", 1.0), ("Output/source_opacity", 70.0), ("Output/glow_opacity", 60.0)]),
        ("linear", vec![("Output/gamma", 0.0), ("Glow/exposure", 1.5)]),
        ("dithered", vec![("Output/dither", 1.0)]),
    ];
    for (name, values) in cases {
        let params = params(spec, &values);
        let cpu = effects::apply(spec, &ctx(&params), picture());
        let mut unchanged = picture();
        unchanged.pad(cpu.offset[0] as u32);
        assert!(differences(&cpu.img, &unchanged.img).1 > 0.05, "{name}: the glow changes the picture");
        let out = gpu.effects(&[FxStep { spec, ctx: ctx(&params) }], &picture(), None).unwrap_or_else(|| panic!("{name}: the GPU ran the plan"));
        assert_eq!((out.offset, out.img.width, out.img.height), (cpu.offset, cpu.img.width, cpu.img.height), "{name}: frame geometry");
        let (mean, worst) = differences(&cpu.img, &out.img);
        eprintln!("lumen glow {name}: mean {mean:.2e} worst {worst:.2e}");
        assert!(mean < MEAN_TOLERANCE && worst < WORST_TOLERANCE, "{name}: GPU and CPU differ by mean {mean}, worst {worst}");
    }
}
