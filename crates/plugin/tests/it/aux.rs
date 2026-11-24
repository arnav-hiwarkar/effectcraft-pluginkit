use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectEnv, EffectHost, Image, LayerPixels, Params, dynamic};
use effectcraft_engine::raster::AuxChannels;
use effectcraft_engine::render::LayerCache;
use serde_json::json;

use crate::render::{cached_frame, frame, native_probe};
use crate::{App, app};

fn scene(id: &str) -> (App, serde_json::Value, serde_json::Value) {
    let mut app = app(id, &[]);
    let session = &mut app.session;
    session.execute("file.projectSettings", json!({"bitDepth": 32})).expect("32 bit");
    let inner =
        session.execute("comp.new", json!({"name": "Scene", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).expect("inner")["comp"].clone();
    let near = session.execute("layer.newSolid", json!({"name": "Near", "width": 64, "height": 36})).expect("solid")["layer"].clone();
    session.execute("layer.setSwitch", json!({"layers": [near], "switch": "threeD", "value": true})).expect("3D");
    let outer =
        session.execute("comp.new", json!({"name": "Outer", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).expect("outer")["comp"].clone();
    session.execute("layer.addItem", json!({"item": inner})).expect("the precomp");
    (app, json!([inner, outer]), near)
}

fn centre(app: &App, cache: Option<&LayerCache>) -> [f32; 4] {
    let image = cached_frame(app, None, cache);
    image.data[(image.height / 2 * image.width + image.width / 2) as usize]
}

fn read_depth(app: &mut App, effect: &str) -> [f32; 4] {
    app.session.execute("effect.apply", json!({"effect": effect})).expect("effect.apply");
    let read = centre(app, None);
    app.session.execute("edit.undo", json!({})).expect("undo");
    read
}

#[test]
fn an_effect_reads_the_depth_and_normal_of_a_3d_precomp_sandboxed_and_native() {
    let (mut app, _, _) = scene("test.aux-read");
    let native = native_probe(&app);
    let sandboxed = read_depth(&mut app, "test.aux-read.aux-depth");
    let unsandboxed = read_depth(&mut app, &format!("{native}.aux-depth"));
    let zoom = 64.0 * 50.0 / 36.0 / 1000.0;
    assert!((sandboxed[0] - zoom).abs() < 1e-5 && sandboxed[1..] == [1.0, 0.0, 1.0], "the precomp's centre reads {sandboxed:?}");
    assert_eq!(sandboxed, unsandboxed);
}

#[test]
fn an_effect_on_a_plain_layer_gets_no_channels() {
    let mut app = app("test.aux-plain", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Plain", "width": 64, "height": 36})).expect("solid");
    app.session.execute("effect.apply", json!({"effect": "test.aux-plain.aux-depth"})).expect("effect.apply");
    assert_eq!(frame(&app, None).data[0], [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn a_cached_layer_reading_depth_redraws_when_the_depth_source_moves() {
    let (mut app, comps, near) = scene("test.aux-cache");
    app.session.execute("effect.apply", json!({"effect": "test.aux-cache.aux-depth"})).expect("effect.apply");
    let cache = LayerCache::new(256 << 20);
    let before = centre(&app, Some(&cache));
    assert_eq!(centre(&app, Some(&cache)), before);
    app.session.execute("comp.open", json!({"comp": comps[0]})).expect("open the scene");
    app.session.execute("prop.set", json!({"layer": near, "path": "transform/position", "value": [32.0, 18.0, 200.0]})).expect("move");
    app.session.execute("comp.open", json!({"comp": comps[1]})).expect("open the outer comp");
    let after = centre(&app, Some(&cache));
    assert!((after[0] - before[0] - 0.2).abs() < 1e-5, "depth moved 200 px back, the cached read is {after:?} after {before:?}");
    assert_eq!(after, centre(&app, None));
}

struct CountedAux(AuxChannels, AtomicUsize);

impl EffectHost for CountedAux {
    fn layer(&self, _id: u64, _masks_and_effects: bool) -> Option<LayerPixels> {
        None
    }

    fn audio(&self, _id: u64, _start: f64, _frames: usize, _rate: u32) -> Option<Vec<f32>> {
        None
    }

    fn aux(&self) -> Option<Arc<AuxChannels>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Some(Arc::new(self.0.clone()))
    }
}

#[test]
fn a_banded_render_reads_each_rows_depth_from_one_source_and_its_margin_is_unknown() {
    let _app = app("test.aux-bands", &[]);
    let spec = effects::find("test.aux-bands.aux-depth").expect("the probe effect is registered");
    let (width, height) = (48u32, 300u32);
    let depth = (0..height).flat_map(|y| (0..width).map(move |_| y as f32 * 3.0)).collect();
    let host = CountedAux(AuxChannels { width, height, scale: 1.0, channels: vec![("Z".into(), depth)], ..Default::default() }, AtomicUsize::new(0));
    let size = [f64::from(width), f64::from(height)];
    let params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
    let ctx =
        EffectCtx { params: &params, time: 0.0, layer_size: size, seed: 1, adjustment: false, env: EffectEnv { host: Some(&host), ..EffectEnv::default() } };
    let out = effects::apply(spec, &ctx, Buf { img: Image::new(width, height), offset: [3.0, 7.0], scale: 1.0 });
    let at = |y: u32| out.img.data[(y * width + 20) as usize][0];
    for y in [10, 100, 170, 299] {
        assert!((at(y) - (y - 7) as f32 * 3.0 / 1000.0).abs() < 1e-6, "row {y} reads {}", at(y));
    }
    assert_eq!(at(2), 1.0e10 / 1000.0, "rows above the layer read the background depth");
    assert_eq!(host.1.load(Ordering::SeqCst), 1, "every band and the layout share one source");
    let margin = dynamic::get(spec.id).and_then(|registered| registered.effect.margin.as_ref().map(|margin| margin(&ctx, 1.0)));
    assert_eq!(margin, Some(None), "a layout that may read depth has no known margin");
}

fn depth_exr(width: usize, height: usize) -> tempfile::NamedTempFile {
    use exr::prelude::*;
    let plane = |name: &str, value: &dyn Fn(usize) -> f32| AnyChannel::new(name, FlatSamples::F32((0..width * height).map(|i| value(i / width)).collect()));
    let channels = AnyChannels::sort(
        vec![
            plane("R", &|_| 0.5),
            plane("G", &|_| 0.5),
            plane("B", &|_| 0.5),
            plane("A", &|_| 1.0),
            plane("Z", &|y| y as f32 * 2.0),
            plane("extra.V", &|y| y as f32),
        ]
        .into(),
    );
    let file = tempfile::Builder::new().suffix(".exr").tempfile().expect("temp file");
    Image::from_layer(Layer::new((width, height), LayerAttributes::default(), Encoding::FAST_LOSSLESS, channels))
        .write()
        .to_file(file.path())
        .expect("write the EXR");
    file
}

#[test]
fn an_exr_layer_reads_each_rows_depth_across_bands_and_its_cached_render_is_a_hit() {
    let mut app = app("test.aux-exr", &[]);
    let file = depth_exr(64, 300);
    app.session.execute("file.projectSettings", json!({"bitDepth": 32})).expect("32 bit");
    let imported = app.session.execute("file.import", json!({"paths": [file.path()]})).expect("import");
    app.session.execute("comp.new", json!({"name": "EXR", "width": 64, "height": 300, "frameRate": 24, "duration": 1})).expect("comp");
    app.session.execute("layer.addItem", json!({"item": imported["items"][0]})).expect("the footage layer");
    app.session.execute("effect.apply", json!({"effect": "test.aux-exr.aux-depth"})).expect("effect.apply");
    let cache = LayerCache::new(256 << 20);
    let whole = cached_frame(&app, None, Some(&cache));
    let at = |image: &effectcraft_engine::raster::Image, y: u32| image.data[(y * image.width + 32) as usize][0];
    for y in [0, 90, 180, 299] {
        assert!((at(&whole, y) - y as f32 * 2.0 / 1000.0).abs() < 1e-6, "row {y} reads {}", at(&whole, y));
    }
    let hits = cache.stats().hits;
    assert_eq!(cached_frame(&app, None, Some(&cache)).data, whole.data);
    assert!(cache.stats().hits > hits, "the second render of the EXR layer missed the layer cache");
    let part = frame(&app, Some([0.0, 130.0, 64.0, 100.0]));
    let row = |image: &effectcraft_engine::raster::Image, y: u32, top: u32| {
        image.data[((y - top) * image.width) as usize..((y - top + 1) * image.width) as usize].to_vec()
    };
    assert_eq!(row(&part, 170, 130), row(&whole, 170, 0), "a region of interest reads the same depth as the whole frame");
}
