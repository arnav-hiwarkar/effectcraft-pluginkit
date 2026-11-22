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
const BEFORE: &str = "org.craftkit.lumen-glow-before-host-kernels";
const OLD_OUTPUT_TOLERANCE: f32 = 2.0 / 255.0;
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

fn before_bundle() -> &'static Path {
    static BUNDLE: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, path) = BUNDLE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join(BEFORE);
        std::fs::create_dir_all(&dest).expect("bundle dir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/lumen-glow-before-host-kernels.wasm");
        std::fs::copy(fixture, dest.join("plugin.wasm")).expect("copy the old wasm");
        let manifest = std::fs::read_to_string(bundle().join("plugin.json")).expect("the manifest");
        let mut manifest: serde_json::Value = serde_json::from_str(&manifest).expect("the manifest is JSON");
        manifest["id"] = BEFORE.into();
        std::fs::write(dest.join("plugin.json"), manifest.to_string()).expect("write the manifest");
        (dir, dest)
    });
    path
}

fn loaded() {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| {
        let config = tempfile::tempdir().expect("temp dir").keep();
        let mut session = effectcraft_host::session();
        let plugins = effectcraft_plugin::install(&mut session, Some(&config));
        plugins.load_for_session(bundle()).expect("Lumen Glow loads");
        plugins.load_for_session(before_bundle()).expect("Lumen Glow as built before host kernels loads");
    });
}

fn spec() -> &'static EffectSpec {
    loaded();
    effects::find(EFFECT).expect("Lumen Glow registered its effect")
}

fn before_spec() -> &'static EffectSpec {
    loaded();
    effects::find(&format!("{BEFORE}.glow")).expect("the old Lumen Glow registered its effect")
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
    params_for(spec, SIZE, values)
}

fn params_for(spec: &EffectSpec, size: [u32; 2], values: &[(&str, f64)]) -> Params {
    let size = [f64::from(size[0]), f64::from(size[1])];
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

fn cases() -> [(&'static str, Vec<(&'static str, f64)>); 10] {
    [
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
    ]
}

#[test]
fn lumen_glow_on_host_kernels_matches_the_build_before_them_and_that_build_still_renders() {
    let (spec, before) = (spec(), before_spec());
    for (name, values) in cases() {
        let params = params(spec, &values);
        let new = effects::apply(spec, &ctx(&params), picture());
        let old = effects::apply(before, &ctx(&params), picture());
        let mut unchanged = picture();
        unchanged.pad(old.offset[0] as u32);
        assert!(differences(&old.img, &unchanged.img).1 > 0.05, "{name}: the old build renders a glow");
        assert_eq!((new.offset, new.img.width, new.img.height), (old.offset, old.img.width, old.img.height), "{name}: frame geometry");
        let (mean, worst) = differences(&old.img, &new.img);
        eprintln!("lumen glow {name} against the old build: mean {mean:.2e} worst {worst:.2e}");
        assert!(worst < OLD_OUTPUT_TOLERANCE, "{name}: the output moved by {worst} from the build before host kernels");
    }
}

#[test]
fn lumen_glow_renders_on_the_gpu_as_its_cpu_render_does() {
    let spec = spec();
    let Some(gpu) = gpu() else { return };
    assert!(spec.gpu && gpu.supports_effect(spec.id), "Lumen Glow is offered to the GPU");
    for (name, values) in cases() {
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

const POINT: u32 = 257;
const CENTRE: usize = POINT as usize / 2;

fn glow(values: &[(&str, f64)], img: Image, scale: f64, layer: u32) -> Image {
    let spec = spec();
    let base = [("Output/view", 1.0), ("Output/dither", 0.0)];
    let values: Vec<(&str, f64)> = base.iter().chain(values).copied().collect();
    let params = params_for(spec, [layer; 2], &values);
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [f64::from(layer); 2], seed: 11, adjustment: false, env: Default::default() };
    let pad = |buf: &Buf| buf.offset[0] as usize;
    let out = effects::apply(spec, &ctx, Buf { img, offset: [0.0, 0.0], scale });
    let mut cropped = Image::new(out.img.width - 2 * pad(&out) as u32, out.img.height - 2 * pad(&out) as u32);
    for y in 0..cropped.height {
        for x in 0..cropped.width {
            cropped.set(x, y, out.img.get(i64::from(x) + pad(&out) as i64, i64::from(y) + pad(&out) as i64));
        }
    }
    cropped
}

fn glow_of_point(brightness: f32, values: &[(&str, f64)]) -> Image {
    let mut img = Image::new(POINT, POINT);
    img.set(CENTRE as u32, CENTRE as u32, [brightness, brightness, brightness, 1.0]);
    glow(values, img, 1.0, POINT)
}

fn at(img: &Image, dx: isize, dy: isize) -> f32 {
    img.get(CENTRE as i64 + dx as i64, CENTRE as i64 + dy as i64)[1]
}

#[test]
fn a_single_bright_pixel_glows_radially_symmetric() {
    let glow = glow_of_point(1.0, &[("Glow/radius", 120.0)]);
    for d in [8, 20, 50, 90] {
        let axis = [at(&glow, d, 0), at(&glow, -d, 0), at(&glow, 0, d), at(&glow, 0, -d)];
        let diagonal_d = (d as f32 / std::f32::consts::SQRT_2).round() as isize;
        let diagonal = at(&glow, diagonal_d, diagonal_d);
        let (lo, hi) = axis.iter().fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(lo > 0.0 && hi / lo < 1.05, "distance {d}: {axis:?}");
        assert!((diagonal / hi - 1.0).abs() < 0.1, "distance {d}: diagonal {diagonal} axis {hi}");
    }
}

#[test]
fn glow_falls_off_monotonically_from_the_core() {
    let glow = glow_of_point(1.0, &[("Glow/radius", 120.0), ("Output/gamma", 0.0)]);
    let profile: Vec<f32> = (0..120).map(|d| at(&glow, d, 0)).collect();
    assert!(profile.windows(2).all(|w| w[1] <= w[0]), "{profile:?}");
}

#[test]
fn hdr_sources_glow_in_proportion_to_their_energy() {
    let linear = [("Glow/radius", 60.0), ("Output/gamma", 0.0)];
    let (dim, hot) = (glow_of_point(1.0, &linear), glow_of_point(16.0, &linear));
    for d in [4, 15, 40] {
        let ratio = at(&hot, d, 0) / at(&dim, d, 0);
        assert!((ratio - 16.0).abs() < 1e-3, "distance {d}: ratio {ratio}");
    }
}

fn disc(size: u32, radius: f32) -> Image {
    let mut img = Image::new(size, size);
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            if dx * dx + dy * dy < radius * radius {
                img.set(x, y, [1.0; 4]);
            }
        }
    }
    img
}

#[test]
fn half_resolution_matches_the_full_resolution_glow() {
    let values = [("Glow/radius", 80.0), ("Output/gamma", 0.0)];
    let full = glow(&values, disc(256, 24.0), 1.0, 256);
    let half = glow(&values, disc(128, 12.0), 0.5, 256);
    for (x, y) in [(64, 30), (64, 10), (100, 100), (64, 64), (20, 64)] {
        let averaged: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| full.get(2 * x + dx, 2 * y + dy)[1]).sum::<f32>() / 4.0;
        let preview = half.get(x, y)[1];
        assert!((preview / averaged - 1.0).abs() < 0.04, "({x}, {y}): half {preview} full {averaged}");
    }
}
