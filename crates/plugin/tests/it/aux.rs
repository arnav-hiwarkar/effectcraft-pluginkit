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
