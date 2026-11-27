use std::path::Path;
use std::sync::OnceLock;

use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectSpec, Params};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::raster::Image;
use effectcraft_gpu::Gpu;

use crate::{build_bundle, test_card};

pub fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    match GPU.get_or_init(Gpu::try_headless) {
        Ok(gpu) => Some(gpu),
        Err(why) if std::env::var("EFFECTCRAFT_REQUIRE_GPU").is_ok_and(|v| v == "1") => panic!("no GPU: {why}"),
        Err(why) => {
            eprintln!("no GPU ({why}); skipping the retro GPU comparison");
            None
        }
    }
}

pub fn spec(id: &str) -> &'static EffectSpec {
    static LOADED: OnceLock<tempfile::TempDir> = OnceLock::new();
    LOADED.get_or_init(|| {
        let dir = tempfile::tempdir().expect("temp dir");
        let bundle = build_bundle(dir.path(), false);
        let config = dir.path().join("config");
        let mut session = effectcraft_host::session();
        let plugins = effectcraft_plugin::install(&mut session, Some(Path::new(&config)));
        plugins.load_for_session(&bundle).expect("retro loads");
        dir
    });
    effects::find(id).unwrap_or_else(|| panic!("retro registered {id}"))
}

pub fn picture(size: [u32; 2]) -> Buf {
    let [w, h] = size;
    let mut img = Image::new(w, h);
    for (i, p) in test_card(w, h).into_iter().enumerate() {
        img.set(i as u32 % w, i as u32 / w, p);
    }
    Buf { img, offset: [0.0, 0.0], scale: 1.0 }
}

pub fn params(spec: &EffectSpec, size: [u32; 2], values: &[(&str, f64)]) -> Params {
    let layer = size.map(f64::from);
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, layer))).collect() };
    for (id, value) in values {
        assert!(params.values.contains_key(*id), "retro has a parameter {id}");
        params.values.insert(id.to_string(), Value::Scalar(*value));
    }
    params
}

pub fn ctx(params: &Params, size: [u32; 2], time: f64) -> EffectCtx<'_> {
    EffectCtx { params, time, layer_size: size.map(f64::from), seed: 5, adjustment: false, env: Default::default() }
}

pub fn worst(a: &Image, b: &Image) -> (f32, usize) {
    let mut worst = (0.0f32, 0);
    for (i, (p, q)) in a.data.iter().zip(&b.data).enumerate() {
        for (x, y) in p.iter().zip(q) {
            let d = if x.is_finite() && y.is_finite() { (x - y).abs() } else { f32::INFINITY };
            if d > worst.0 {
                worst = (d, i);
            }
        }
    }
    worst
}
