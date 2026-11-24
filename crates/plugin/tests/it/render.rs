use effectcraft_engine::render::{RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use serde_json::json;

use effectcraft_plugin::ui::{TextStyle, UiEvent, UiEventKind, Widget};

use crate::{App, app};

fn centre_pixel(app: &App) -> [f32; 4] {
    let session = &app.session;
    let comp = session.active_comp_id().expect("an active comp");
    let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts::default());
    renderer.expr = session.expr.as_deref();
    let image = renderer.comp_frame(comp, Tick::from_seconds_f64(0.0));
    image.data[(image.height / 2 * image.width + image.width / 2) as usize]
}

fn close(found: [f32; 4], wanted: [f32; 4]) -> bool {
    found.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.02)
}

#[test]
fn a_plugin_effect_applied_to_a_layer_changes_the_rendered_composition() {
    let mut app = app("test.render-invert", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "the plain solid is {:?}", centre_pixel(&app));

    app.session.execute("effect.apply", json!({"effect": "test.render-invert.invert"})).unwrap();
    assert!(close(centre_pixel(&app), [0.0, 0.8, 1.0, 1.0]), "inverted it is {:?}", centre_pixel(&app));
}

#[test]
fn a_plugin_effect_parameter_is_a_property_that_can_be_set_and_undone() {
    let mut app = app("test.render-param", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-param.invert"})).unwrap();

    app.session.execute("prop.set", json!({"path": "effects/#1/amount", "value": 0.0})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "amount 0 gives {:?}", centre_pixel(&app));

    app.session.execute("edit.undo", json!({})).unwrap();
    assert!(close(centre_pixel(&app), [0.0, 0.8, 1.0, 1.0]), "after undo it is {:?}", centre_pixel(&app));
}

#[test]
fn stopping_a_plugin_removes_its_effects_and_the_layer_renders_unchanged() {
    let mut app = app("test.render-stop", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-stop.invert"})).unwrap();
    assert!(effectcraft_engine::effects::find("test.render-stop.invert").is_some());

    std::fs::remove_file(app.bundle.path().join("probe.wasm")).unwrap();
    assert!(app.plugins.load_for_session(app.bundle.path()).is_err(), "a bundle without its module fails to load");
    assert!(effectcraft_engine::effects::find("test.render-stop.invert").is_none());
    assert!(close(centre_pixel(&app), [1.0, 0.2, 0.0, 1.0]), "without the plugin it is {:?}", centre_pixel(&app));
}

#[test]
fn reloading_a_plugin_with_new_code_changes_the_effect_revision_so_cached_frames_redraw() {
    let app = app("test.render-revision", &[]);
    let effect = "test.render-revision.invert";
    let first = effectcraft_engine::effects::revision(effect);
    assert_ne!(first, 0);

    app.plugins.load_for_session(app.bundle.path()).unwrap();
    assert_eq!(effectcraft_engine::effects::revision(effect), first, "the same code keeps its revision");

    let module = app.bundle.path().join("probe.wasm");
    let mut bytes = std::fs::read(&module).unwrap();
    bytes.extend_from_slice(&[0, 4, 1, b'x', 7, 7]);
    std::fs::write(&module, bytes).unwrap();
    app.plugins.load_for_session(app.bundle.path()).unwrap();
    assert_ne!(effectcraft_engine::effects::revision(effect), first, "new code gets a new revision");
}

fn reader_key(app: &App) -> Option<u64> {
    let session = &app.session;
    let comp_id = session.active_comp_id().expect("an active comp");
    let comp = session.active_comp().expect("an active comp");
    let ctx = effectcraft_engine::render::EvalCtx::new(&session.project, comp_id, comp, Tick::from_seconds_f64(0.0));
    let reader = comp.layers.iter().find(|layer| layer.name == "Reader").expect("the reader layer");
    effectcraft_engine::render::cache::layer_key(&ctx, reader, 1.0, false, false)
}

#[test]
fn a_layer_that_samples_a_plugin_layer_changes_its_cache_key_when_the_plugin_changes() {
    let mut app = app("test.render-dependent", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Source", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Source", "effect": "test.render-dependent.invert"})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Reader", "color": "#00ff00", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Reader", "effect": "ec.channel.setmatte"})).unwrap();
    let source = app.session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Source").unwrap().id.0;
    app.session.execute_checked("prop.set", json!({"layer": "Reader", "path": "effects/#1/takeMatteFromLayer", "value": source})).unwrap();
    let before = reader_key(&app).expect("the reader is cacheable");
    let comp = app.session.active_comp_id().unwrap();
    let content = |app: &App| effectcraft_engine::render::disk_cache::comp_content_key(&app.session.project, comp);
    let content_before = content(&app);

    let manifest = app.bundle.path().join("plugin.json");
    let changed = std::fs::read_to_string(&manifest).unwrap().replace("\"1.0.0\"", "\"1.0.1\"");
    std::fs::write(&manifest, changed).unwrap();
    app.plugins.load_for_session(app.bundle.path()).expect("the changed plugin loads");

    assert_ne!(reader_key(&app).expect("the reader is cacheable"), before);
    assert_ne!(content(&app), content_before, "the comp's thumbnail and disk frames are keyed on the plugin too");
}

#[test]
fn a_plugin_effect_changes_its_cache_key_when_a_file_in_its_data_folder_changes() {
    let mut app = app("test.render-data", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Reader", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Reader", "effect": "test.render-data.invert"})).unwrap();
    let before = reader_key(&app).expect("the layer is cacheable");
    assert_eq!(reader_key(&app), Some(before), "the key is stable while nothing changes");

    let data = app.plugins.get("test.render-data").expect("the plugin runs").data_dir.clone();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("palette.json"), "[1, 2, 3]").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_ne!(reader_key(&app).expect("the layer is cacheable"), before);
}

fn controls_of(app: &mut App, effect: &str, instance: u64, value: &str) -> Vec<Widget> {
    let mut panels = effectcraft_plugin::Panels::default();
    let surface = effectcraft_plugin::Surface::Control { effect, param: "level", instance, value, edit: None };
    crate::panels::settled(app, &mut panels, "control", surface, &mut 0)
}

#[test]
fn a_custom_control_draws_widgets_for_the_current_value_and_sets_the_edited_value() {
    let mut app = app("test.render-controls", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Scaled", "color": "#ffffff", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"layer": "Scaled", "effect": "test.render-controls.scale"})).unwrap();
    let found = app.session.execute("prop.get", json!({"layer": "Scaled", "path": "effects/#1/level"})).unwrap();
    let comp = app.session.active_comp_id().unwrap().0;
    let target =
        effectcraft_plugin::Target { comp, layer: found["layer"].as_u64().unwrap(), prop: found["uid"].as_u64().unwrap(), time: 0.0, merge: "m".into() };
    let mut panels = effectcraft_plugin::Panels::default();
    let surface =
        effectcraft_plugin::Surface::Control { effect: "test.render-controls.scale", param: "level", instance: target.prop, value: "1", edit: Some(&target) };
    let mut frame = 0;
    let widgets = crate::panels::settled(&mut app, &mut panels, "control", surface, &mut frame);
    assert!(matches!(&widgets[0], Widget::Label { text, style: TextStyle::Body, .. } if text == "Level 1"), "{:?}", widgets[0]);
    let Widget::Button { id, .. } = widgets[2] else { panic!("a button, not {:?}", widgets[2]) };
    panels.submit("control", vec![UiEvent { id, kind: UiEventKind::Click }], "");
    crate::panels::settled(&mut app, &mut panels, "control", surface, &mut frame);
    let level = app.session.execute("prop.get", json!({"layer": "Scaled", "path": "effects/#1/level"})).unwrap()["value"].clone();
    assert_eq!(level, json!("0.5"));
}

#[test]
fn a_custom_control_is_told_which_effect_instance_it_draws() {
    let mut app = app("test.render-instances", &[]);
    let first = controls_of(&mut app, "test.render-instances.scale", 11, "1");
    let second = controls_of(&mut app, "test.render-instances.scale", 12, "1");
    let text = |widget: &Widget| match widget {
        Widget::Label { text, .. } => text.clone(),
        other => format!("{other:?}"),
    };
    assert_eq!((text(&first[1]), text(&second[1])), ("Instance 11".to_string(), "Instance 12".to_string()));
}

#[test]
fn a_project_with_a_custom_value_opens_and_renders_without_the_plugin_and_again_with_it() {
    let mut app = app("test.render-custom", &[]);
    app.session.execute("layer.newSolid", json!({"name": "White", "color": "#ffffff", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-custom.scale"})).unwrap();
    app.session.execute("prop.set", json!({"path": "effects/#1/level", "value": "0.25"})).unwrap();
    assert!(close(centre_pixel(&app), [0.25, 0.25, 0.25, 1.0]), "level 0.25 gives {:?}", centre_pixel(&app));
    let saved = app.bundle.path().join("custom.ecproj");
    app.session.execute("file.saveAs", json!({"path": saved})).unwrap();

    let module = std::fs::read(app.bundle.path().join("probe.wasm")).unwrap();
    std::fs::remove_file(app.bundle.path().join("probe.wasm")).unwrap();
    assert!(app.plugins.load_for_session(app.bundle.path()).is_err());
    app.session.execute("file.open", json!({"path": saved})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 1.0, 1.0, 1.0]), "without the plugin it is {:?}", centre_pixel(&app));
    let kept = app.session.execute("prop.get", json!({"layer": "White", "path": "effects/#1/level"})).unwrap();
    assert_eq!(kept["value"], "0.25");

    std::fs::write(app.bundle.path().join("probe.wasm"), module).unwrap();
    app.plugins.load_for_session(app.bundle.path()).expect("the plugin loads again");
    app.session.execute("file.open", json!({"path": saved})).unwrap();
    assert!(close(centre_pixel(&app), [0.25, 0.25, 0.25, 1.0]), "with the plugin back it is {:?}", centre_pixel(&app));
}

fn reported_camera(id: &str, camera: Option<serde_json::Value>) -> ([f32; 3], f32, serde_json::Value) {
    let mut app = app(id, &[]);
    app.session.execute("file.projectSettings", json!({"bitDepth": 32})).expect("32 bit");
    if let Some(camera) = camera {
        app.session.execute("layer.newCamera", camera).expect("layer.newCamera");
    }
    app.session.execute("layer.newSolid", json!({"name": "Plate", "color": "#000000", "width": 64, "height": 36})).expect("layer.newSolid");
    app.session.execute("effect.apply", json!({"effect": format!("{id}.scene-camera")})).expect("effect.apply");
    let view = app.session.execute("view.get3D", json!({})).expect("view.get3D");
    let comp = app.session.active_comp_id().expect("an active comp");
    let image = Renderer::new(&app.session.project, app.session.footage.as_ref(), RenderOpts::default()).comp_frame(comp, Tick::from_seconds_f64(0.0));
    let [x, y, z, _] = image.data[0];
    ([x, y, z], image.data[1][0], view)
}

#[test]
fn the_scene_camera_a_plugin_reads_is_the_camera_layer_in_the_layer_pixels() {
    let camera = json!({"name": "Shot", "type": "twoNode", "position": [10.0, 5.0, -300.0], "poi": [40.0, 20.0, 0.0], "zoom": 250.0});
    let (position, zoom, _) = reported_camera("test.camera-layer", Some(camera));
    assert_eq!((position, zoom), ([10.0, 5.0, -300.0], 250.0));
}

#[test]
fn without_a_camera_layer_the_scene_camera_is_the_comp_default_view() {
    let (position, zoom, view) = reported_camera("test.default-view", None);
    let eye: Vec<f32> = view["camera"]["eye"].as_array().expect("an eye").iter().map(|c| c.as_f64().expect("a number") as f32).collect();
    let default_zoom = view["camera"]["zoom"].as_f64().expect("a zoom") as f32;
    assert_eq!(position.to_vec(), eye);
    assert!((zoom - default_zoom).abs() < 1e-3, "zoom {zoom} against the view's {default_zoom}");
}

#[test]
fn a_layer_parameter_reads_a_layer_whose_video_is_off_as_its_pixels() {
    let mut app = app("test.render-hidden-input", &[]);
    let session = &mut app.session;
    let matte = session.execute("layer.newSolid", json!({"name": "Matte", "color": "#ff8000", "width": 64, "height": 36})).unwrap()["layer"].as_u64();
    session.execute("layer.setSwitch", json!({"layers": [matte], "switch": "video", "value": false})).unwrap();
    let plate = session.execute("layer.newSolid", json!({"name": "Plate", "color": "#0000ff", "width": 64, "height": 36})).unwrap()["layer"].as_u64();
    session.execute("effect.apply", json!({"layer": plate, "effect": format!("{}.gpu-matte", app.id)})).unwrap();
    session.execute_checked("prop.set", json!({"layer": plate, "path": "effects/#1/matte", "value": matte})).unwrap();
    assert!(close(centre_pixel(&app), [1.0, 0.502, 0.0, 1.0]), "the plate shows the hidden matte: {:?}", centre_pixel(&app));
}

#[test]
fn reloading_new_code_with_the_same_effect_declarations_reuses_their_descriptions() {
    let app = app("test.render-reuse", &[]);
    let effect = "test.render-reuse.invert";
    let first = effectcraft_engine::effects::find(effect).unwrap();
    let module = app.bundle.path().join("probe.wasm");
    for extra in 0..3u8 {
        let mut bytes = std::fs::read(&module).unwrap();
        bytes.extend_from_slice(&[0, 4, 1, b'x', 7, extra]);
        std::fs::write(&module, bytes).unwrap();
        app.plugins.load_for_session(app.bundle.path()).unwrap();
        assert!(std::ptr::eq(effectcraft_engine::effects::find(effect).unwrap(), first), "reload {extra} leaked a new description");
    }
}

#[test]
fn a_plugin_that_starts_after_its_project_opened_gives_existing_instances_their_missing_parameters() {
    let mut app = app("test.render-late", &[]);
    let solid = app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap()["layer"].as_u64();
    app.session.execute("effect.apply", json!({"effect": "test.render-late.invert"})).unwrap();
    let comp = app.session.active_comp_id().unwrap();
    let layer =
        std::sync::Arc::make_mut(&mut app.session.project).comp_mut(comp).unwrap().layer_mut(effectcraft_engine::project::LayerId(solid.unwrap())).unwrap();
    let Some(effectcraft_engine::project::Node::Group(instance)) = layer.props.sub_mut("effects").unwrap().children.first_mut() else { panic!("no instance") };
    instance.children.retain(|node| !matches!(node, effectcraft_engine::project::Node::Prop(prop) if prop.match_id == "amount"));
    let path = json!({"layer": solid, "path": "effects/#1/amount"});
    assert!(app.session.execute("prop.get", path.clone()).is_err());

    app.session.mark_saved();

    app.session.execute("plugin.load", json!({"path": app.bundle.path()})).unwrap();
    assert_eq!(app.session.execute("prop.get", path.clone()).unwrap()["value"], json!(1.0));
    assert!(app.session.is_dirty(), "the migrated project still looks saved");
    app.session.execute("edit.undo", json!({})).unwrap();
    assert!(app.session.execute("prop.get", path).is_err(), "undo kept the migrated parameter");
}

fn tally(app: &App) -> u64 {
    let bytes = app.plugins.get(&app.id).and_then(|loaded| loaded.plugin.cached("tally"));
    bytes.and_then(|bytes| <[u8; 8]>::try_from(&bytes[..]).ok()).map_or(0, u64::from_le_bytes)
}

fn frame(app: &App, roi: Option<[f64; 4]>) -> effectcraft_engine::raster::Image {
    cached_frame(app, roi, None)
}

fn cached_frame(app: &App, roi: Option<[f64; 4]>, cache: Option<&effectcraft_engine::render::LayerCache>) -> effectcraft_engine::raster::Image {
    let session = &app.session;
    let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts { roi, ..RenderOpts::default() });
    renderer.expr = session.expr.as_deref();
    renderer.cache = cache;
    renderer.comp_frame(session.active_comp_id().expect("an active comp"), Tick::from_seconds_f64(0.0))
}

#[test]
fn a_layer_partly_outside_the_comp_moved_a_few_pixels_is_served_from_the_layer_cache() {
    let mut app = app("test.render-moving", &[]);
    app.session.execute("comp.new", json!({"name": "Wide", "width": 640, "height": 100, "frameRate": 24, "duration": 2})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Grey", "color": "#808080", "width": 640, "height": 100})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-moving.tally"})).unwrap();
    let cache = effectcraft_engine::render::LayerCache::new(256 << 20);
    app.session.execute("prop.set", json!({"path": "transform/position", "value": [700, 50]})).unwrap();
    cached_frame(&app, None, Some(&cache));
    let rendered = tally(&app);
    assert!(rendered < 64_000, "the layer rendered {rendered} of 64000 pixels, not only its shown part");

    app.session.execute("prop.set", json!({"path": "transform/position", "value": [704, 50]})).unwrap();
    let moved = cached_frame(&app, None, Some(&cache));
    assert_eq!(tally(&app), rendered, "a layer moved by 4 pixels rendered its effect again");

    let whole = frame(&app, Some([0.0, 0.0, 1280.0, 100.0]));
    let fresh: Vec<[f32; 4]> = whole.data.chunks(whole.width as usize).flat_map(|row| row[..640].to_vec()).collect();
    assert!(moved.data == fresh, "the cached render of the moved layer differs from the whole layer's render");
}

#[test]
fn a_layer_mostly_outside_the_comp_renders_only_its_visible_part() {
    let mut app = app("test.render-visible", &[]);
    app.session.execute("comp.new", json!({"name": "Wide", "width": 640, "height": 100, "frameRate": 24, "duration": 2})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Grey", "color": "#808080", "width": 640, "height": 100})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-visible.tally"})).unwrap();
    let whole = frame(&app, None);
    assert_eq!(tally(&app), 64_000, "a layer inside the comp renders whole");

    app.session.execute("prop.set", json!({"path": "transform/position", "value": [896, 50]})).unwrap();
    let shown = frame(&app, None);
    let rendered = tally(&app) - 64_000;
    assert!(
        (6_400..=12_800).contains(&rendered),
        "a layer 10 % inside the comp rendered {rendered} of 64000 pixels, more than its part snapped out to 64 pixels"
    );
    let at = |image: &effectcraft_engine::raster::Image, x: u32| image.data[(50 * image.width + x) as usize];
    assert_eq!(at(&shown, 600), at(&whole, 600), "the visible part has the layer's pixels");
    assert_eq!(at(&shown, 500), [0.0; 4], "the comp left of the layer is empty");

    let zoomed = frame(&app, Some([576.0, 0.0, 32.0, 100.0]));
    assert_eq!(tally(&app) - 64_000 - rendered, 6_400, "a region of interest renders its part grown by the resampling margin and snapped out to 64 pixels");
    assert_eq!(at(&zoomed, 10), at(&whole, 600));
}

#[test]
fn padding_on_chosen_sides_grows_only_those_sides_of_the_layer() {
    let _app = app("test.render-streak", &[]);
    let spec = effectcraft_engine::effects::find("test.render-streak.streak").expect("the streak effect");
    let params = effectcraft_engine::effects::Params::default();
    let ctx =
        effectcraft_engine::effects::EffectCtx { params: &params, time: 0.0, layer_size: [4.0, 3.0], seed: 1, adjustment: false, env: Default::default() };
    let mut img = effectcraft_engine::raster::Image::new(4, 3);
    img.data.iter_mut().enumerate().for_each(|(i, pixel)| *pixel = [i as f32, 0.5, 0.25, 1.0]);
    let out = effectcraft_engine::effects::apply(spec, &ctx, effectcraft_engine::effects::Buf { img, offset: [0.0, 0.0], scale: 1.0 });
    assert_eq!((out.img.width, out.img.height, out.offset), (11, 3, [5.0, 0.0]));
    assert_eq!([out.img.data[0], out.img.data[5], out.img.data[11 + 8], out.img.data[10]], [[0.0; 4], [0.0, 0.5, 0.25, 1.0], [7.0, 0.5, 0.25, 1.0], [0.0; 4]]);
}

#[test]
fn a_failed_render_is_in_the_plugin_log_with_the_effect_and_a_sequence_number_to_read_after() {
    let mut app = app("test.render-log", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Plate", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-log.read-inputs"})).unwrap();
    centre_pixel(&app);
    let page = app.session.execute("plugin.log", json!({"id": "test.render-log"})).unwrap();
    let lines = &page["lines"];
    let failed: Vec<&serde_json::Value> = lines.as_array().unwrap().iter().filter(|line| line["message"].as_str().unwrap().contains("no matte")).collect();
    assert_eq!(failed[0]["message"], "render of effect `read-inputs` failed: no matte layer");
    assert_eq!(failed[0]["level"], "error");
    let last = lines.as_array().unwrap().last().unwrap()["seq"].clone();
    let newer = app.session.execute("plugin.log", json!({"id": "test.render-log", "after": last, "load": page["load"]})).unwrap();
    assert_eq!((&newer["lines"], &newer["missed"]), (&json!([]), &json!(0)));
}

#[test]
fn a_shown_part_of_a_layer_minified_with_a_fractional_offset_equals_that_part_of_the_whole_layer() {
    let mut app = app("test.render-minified", &[]);
    app.session.execute("layer.newSolid", json!({"name": "Checks", "color": "#808080", "width": 512, "height": 256})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.render-minified.checks"})).unwrap();
    app.session.execute("prop.set", json!({"path": "transform/scale", "value": [25, 25]})).unwrap();
    app.session.execute("prop.set", json!({"path": "transform/position", "value": [20.25, 13.75]})).unwrap();
    let whole = frame(&app, Some([-200.0, -200.0, 464.0, 436.0]));
    let shown = frame(&app, None);
    assert_eq!((shown.width, shown.height), (64, 36));
    for y in 0..36 {
        for x in 0..64 {
            let (cropped, full) = (shown.data[(y * 64 + x) as usize], whole.data[((y + 200) * whole.width + x + 200) as usize]);
            assert!(cropped.iter().zip(full).all(|(a, b)| (a - b).abs() <= 1e-5), "pixel ({x}, {y}): {cropped:?} shown, {full:?} whole");
        }
    }
}

#[test]
fn a_plugin_effect_at_8_bpc_writes_the_pixels_of_a_separate_quantise_pass() {
    use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectEnv, Params};
    let _app = app("test.render-quantise", &[]);
    let (width, height) = (48, 300);
    let input = || {
        let mut image = effectcraft_engine::raster::Image::new(width, height);
        for (i, pixel) in image.data.iter_mut().enumerate() {
            let v = (i as f32 * 0.0137).fract();
            *pixel = [v * 0.9, 0.8 - v * 0.7, v * 0.5, 0.25 + v * 0.75];
        }
        image
    };
    let bits = |pixels: &[[f32; 4]]| pixels.iter().map(|pixel| pixel.map(f32::to_bits)).collect::<Vec<_>>();
    for effect in ["invert", "shift-down", "host-kernels"] {
        let spec = effects::find(&format!("test.render-quantise.{effect}")).expect("the probe effect is registered");
        assert!(spec.quantizes_output(), "{effect} quantises its own output, so the pipeline skips its pass");
        let size = [f64::from(width), f64::from(height)];
        let params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
        let run = |quantize_output| {
            let env = EffectEnv { quantize_output, ..EffectEnv::default() };
            let ctx = EffectCtx { params: &params, time: 0.0, layer_size: size, seed: 1, adjustment: false, env };
            effects::apply(spec, &ctx, Buf { img: input(), offset: [0.0, 0.0], scale: 1.0 }).img.data
        };
        let unquantised = run(None);
        let mut separate_pass = unquantised.clone();
        effectcraft_engine::raster::quantize_slice(&mut separate_pass, 255.0);
        assert!(bits(&unquantised) != bits(&separate_pass), "{effect} writes values between 8 bpc levels");
        assert!(bits(&run(Some(255.0))) == bits(&separate_pass), "{effect} at 8 bpc differs from render then quantise");
    }
}

#[test]
fn a_plugin_effect_at_32_bpc_keeps_values_between_8_bpc_levels() {
    let mut app = app("test.render-float", &[]);
    app.session.execute("file.projectSettings", json!({"bitDepth": "32"})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Red", "color": "#ff3300", "width": 64, "height": 36})).unwrap();
    let plain = centre_pixel(&app);
    app.session.execute("effect.apply", json!({"effect": "test.render-float.invert"})).unwrap();
    app.session.execute("prop.set", json!({"path": "effects/#1/amount", "value": 0.37})).unwrap();
    let inverted = centre_pixel(&app);
    for (channel, (found, before)) in inverted.iter().zip(plain).take(3).enumerate() {
        let wanted = before + (1.0 - 2.0 * before) * 0.37;
        assert!((found - wanted).abs() < 1e-6, "channel {channel}: {found}, wanted {wanted}");
        assert!((found * 255.0 - (found * 255.0).round()).abs() > 0.01, "channel {channel}: {found} lies on an 8 bpc level");
    }
}

#[test]
fn a_plugin_effect_followed_by_a_small_blur_and_a_colour_effect_renders_only_the_shown_part_grown_by_their_margins() {
    let mut app = app("test.render-margins", &[]);
    app.session.execute("comp.new", json!({"name": "Wide", "width": 640, "height": 100, "frameRate": 24, "duration": 2})).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Grey", "color": "#808080", "width": 640, "height": 100})).unwrap();
    for effect in ["test.render-margins.checks", "test.render-margins.tally", "ec.blur.gaussian", "ec.color.levels"] {
        app.session.execute("effect.apply", json!({"effect": effect})).unwrap();
    }
    app.session.execute("prop.set", json!({"path": "effects/#3/blurriness", "value": 5.9})).unwrap();
    app.session.execute("prop.set", json!({"path": "effects/#4/gamma", "value": 2.2})).unwrap();
    let whole = frame(&app, None);
    assert_eq!(tally(&app), 64_000, "a layer inside the comp renders whole");

    let shown = frame(&app, Some([256.0, 32.0, 64.0, 32.0]));
    assert_eq!(tally(&app) - 64_000, 210 * 100, "the 64x32 part snapped out to 192x128 and grown by 9 pixels of blur on each side, within the 100 rows");
    for y in 0..32 {
        for x in 0..64 {
            let (part, full) = (shown.data[(y * 64 + x) as usize], whole.data[((y + 32) * 640 + x + 256) as usize]);
            assert!(part == full, "pixel ({x}, {y}): {part:?} shown, {full:?} whole");
        }
    }
}
