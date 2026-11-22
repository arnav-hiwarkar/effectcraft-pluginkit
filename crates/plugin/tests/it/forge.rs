use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use effectcraft_engine::effects::{self, Buf, EffectCtx, EffectSpec, Params};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::raster::Image;
use effectcraft_engine::render::{Accelerator, Backend, FxStep, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use serde_json::json;

use crate::gpu::gpu;

const SIZE: [u32; 2] = [160, 96];
const EFFECT: &str = "org.craftkit.forge-3d.forge";
const SPHERE: Value = Value::Enum(1);
const GEM: Value = Value::Enum(4);

fn bundle() -> &'static Path {
    static BUNDLE: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, path) = BUNDLE.get_or_init(|| {
        let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/forge-3d");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(plugin.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(status.success(), "Forge 3D builds");
        let dir = tempfile::tempdir().expect("temp dir");
        let bundle = dir.path().join("org.craftkit.forge-3d");
        std::fs::create_dir_all(bundle.join("assets")).expect("bundle dir");
        std::fs::copy(plugin.join("target/wasm32-unknown-unknown/release/forge_3d.wasm"), bundle.join("plugin.wasm")).expect("copy the wasm");
        std::fs::copy(plugin.join("plugin.json"), bundle.join("plugin.json")).expect("copy the manifest");
        std::fs::copy(plugin.join("assets/gem.obj"), bundle.join("assets/gem.obj")).expect("copy the model");
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
        plugins.load_for_session(bundle()).expect("Forge 3D loads");
    });
    effects::find(EFFECT).expect("Forge 3D registered its effect")
}

fn params(spec: &EffectSpec, size: [u32; 2], values: &[(&str, Value)]) -> Params {
    let size = size.map(f64::from);
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), effects::default_value(p, size))).collect() };
    for (id, value) in values {
        assert!(params.values.contains_key(*id), "Forge 3D has a parameter {id}");
        params.values.insert(id.to_string(), value.clone());
    }
    params
}

fn render_over(background: Image, values: &[(&str, Value)]) -> Option<Image> {
    let spec = spec();
    let gpu = gpu()?;
    assert!(spec.gpu && gpu.supports_effect(spec.id), "Forge 3D is offered to the GPU");
    let size = [background.width, background.height];
    let params = params(spec, size, values);
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: size.map(f64::from), seed: 5, adjustment: false, env: Default::default() };
    let below = Buf { img: background, offset: [0.0, 0.0], scale: 1.0 };
    Some(gpu.effects(&[FxStep { spec, ctx }], &below, None).expect("the GPU ran Forge 3D's plan").img)
}

fn render(values: &[(&str, Value)]) -> Option<Image> {
    let flat = [("Transform/rotation_x", Value::Scalar(0.0)), ("Transform/rotation_y", Value::Scalar(0.0)), ("smooth", Value::Bool(false))];
    let values: Vec<(&str, Value)> = flat.iter().chain(values).cloned().collect();
    render_over(Image::new(SIZE[0], SIZE[1]), &values)
}

fn pixel(image: &Image, x: u32, y: u32) -> [f32; 4] {
    image.data[(y * image.width + x) as usize]
}

#[test]
fn a_lit_sphere_shows_the_key_light_on_its_centre() {
    let matte = [
        ("model", SPHERE),
        ("Transform/size", Value::Scalar(40.0)),
        ("Material/colour", Value::Color([0.8, 0.4, 0.2, 1.0])),
        ("Material/specular", Value::Scalar(0.0)),
        ("Material/rim", Value::Scalar(0.0)),
        ("Lighting/ambient", Value::Scalar(20.0)),
    ];
    let Some(image) = render(&matte) else { return };
    let centre = pixel(&image, 80, 48);
    let facing_the_key_light = 0.75 / (0.4f32 * 0.4 + 0.5 * 0.5 + 0.75 * 0.75).sqrt();
    let expected = [0.8, 0.4, 0.2].map(|base| base * (0.2 + facing_the_key_light));
    for (channel, expected) in centre.iter().zip(expected) {
        assert!((channel - expected).abs() < 0.02, "the centre is {centre:?}, expected {expected:?}");
    }
    assert_eq!(centre[3], 1.0);
    assert_eq!(pixel(&image, 80, 4), [0.0; 4], "above the sphere the layer shows through");
    let (towards, away) = (pixel(&image, 66, 34), pixel(&image, 94, 62));
    assert!(towards[0] > away[0] + 0.2, "the side facing the light is brighter: {towards:?} against {away:?}");
}

#[test]
fn of_two_overlapping_copies_the_nearer_one_wins() {
    let two = [
        ("model", SPHERE),
        ("Transform/size", Value::Scalar(30.0)),
        ("Replicator/shape", Value::Enum(1)),
        ("Replicator/columns", Value::Scalar(2.0)),
        ("Replicator/spacing_x", Value::Scalar(40.0)),
        ("Replicator/step_z", Value::Scalar(150.0)),
        ("Replicator/ramp", Value::Bool(true)),
        ("Material/colour", Value::Color([1.0, 0.0, 0.0, 1.0])),
        ("Replicator/last_colour", Value::Color([0.0, 0.0, 1.0, 1.0])),
        ("Lighting/ambient", Value::Scalar(100.0)),
    ];
    let Some(image) = render(&two) else { return };
    let (near_only, both, far_only) = (pixel(&image, 20, 48), pixel(&image, 85, 48), pixel(&image, 110, 48));
    assert!(near_only[0] > 0.5 && near_only[2] < 0.05, "the near copy is red: {near_only:?}");
    assert!(far_only[2] > 0.5 && far_only[0] < 0.05, "the far copy, drawn last, is blue: {far_only:?}");
    assert!(both[0] > 0.5 && both[2] < 0.05, "where they overlap the near copy shows: {both:?}");
}

#[test]
fn the_bundled_obj_model_renders() {
    let Some(gem) = render(&[("model", GEM), ("Transform/size", Value::Scalar(40.0))]) else { return };
    let covered = gem.data.iter().filter(|pixel| pixel[3] > 0.5).count();
    let disc = std::f64::consts::PI * 40.0 * 40.0;
    assert!((0.5 * disc..1.2 * disc).contains(&(covered as f64)), "the gem covers {covered} pixels of a {disc:.0} pixel disc");
}

#[test]
fn a_ring_of_models_draws_over_the_layer_at_1080p() {
    const FRAME: [u32; 2] = [1920, 1080];
    let mut backdrop = Image::new(FRAME[0], FRAME[1]);
    for y in 0..FRAME[1] {
        let shade = 0.02 + 0.1 * y as f32 / FRAME[1] as f32;
        for x in 0..FRAME[0] {
            backdrop.set(x, y, [shade * 0.6, shade * 0.8, shade * 1.4, 1.0]);
        }
    }
    let ring = |turn: f64| {
        [
            ("model", Value::Enum(2)),
            ("Transform/size", Value::Scalar(170.0)),
            ("Transform/rotation_x", Value::Scalar(-18.0)),
            ("Transform/rotation_y", Value::Scalar(turn)),
            ("Replicator/shape", Value::Enum(2)),
            ("Replicator/columns", Value::Scalar(9.0)),
            ("Replicator/radius", Value::Scalar(620.0)),
            ("Replicator/ramp", Value::Bool(true)),
        ]
    };
    let Some(image) = render_over(backdrop.clone(), &ring(20.0)) else { return };
    let changed = image.data.iter().zip(&backdrop.data).filter(|(after, before)| after != before).count();
    let frame = (FRAME[0] * FRAME[1]) as usize;
    assert!((frame / 20..frame / 2).contains(&changed), "the models cover {changed} of {frame} pixels");
    assert_eq!(image.data[0], backdrop.data[0], "the corner still shows the layer");
    let Ok(path) = std::env::var("FORGE_3D_PNG") else { return };
    let frames = 20;
    let started = std::time::Instant::now();
    for i in 0..frames {
        render_over(backdrop.clone(), &ring(21.0 + f64::from(i))).expect("a GPU");
    }
    eprintln!("forge 3d: {:.2} ms for each 1920x1080 frame over {frames} frames", started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames));
    let encode = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
    let rgba: Vec<u8> = image.data.iter().flat_map(|p| [encode(p[0]), encode(p[1]), encode(p[2]), 255]).collect();
    let png = effectcraft_automation::encode_png(FRAME[0], FRAME[1], rgba, 0).expect("PNG encodes");
    std::fs::write(&path, png).expect("the PNG is written");
}

#[test]
fn a_comp_light_replaces_the_key_light_in_a_rendered_composition() {
    let Some(gpu) = gpu() else { return };
    spec();
    let mut session = effectcraft_host::session();
    session.execute("comp.new", json!({"name": "Lit", "width": 160, "height": 96, "frameRate": 24, "duration": 2})).expect("comp.new");
    session.execute("layer.newSolid", json!({"name": "Black", "color": "#000000", "width": 160, "height": 96})).expect("layer.newSolid");
    session.execute("effect.apply", json!({"effect": EFFECT})).expect("effect.apply");
    let comp = session.active_comp_id().expect("an active comp");
    let frame = |session: &effectcraft_engine::Session| {
        let renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts { backend: Backend::Gpu, ..Default::default() });
        renderer.with_accel(Some(gpu)).comp_frame(comp, Tick::from_seconds_f64(0.0))
    };
    let key_lit = frame(&session);
    let (brightest, lit) = key_lit.data.iter().enumerate().max_by(|a, b| a.1[0].total_cmp(&b.1[0])).expect("a frame");
    assert!(lit[0] > 0.5 && lit[0] > lit[2] * 2.0, "under the key light the orange model shows on the black solid: {lit:?}");
    session.execute("layer.newLight", json!({"kind": "Ambient", "name": "Blue", "color": "#0000ff", "intensity": 100.0})).expect("layer.newLight");
    let comp_lit = frame(&session).data[brightest];
    assert!(comp_lit[2] > comp_lit[0] && comp_lit[2] > 0.1, "under a blue ambient comp light the same spot is blue: {comp_lit:?}");
}

struct Comp {
    session: effectcraft_engine::Session,
    layer: u64,
}

impl Comp {
    fn new(size: [u32; 2]) -> Comp {
        spec();
        let mut session = effectcraft_host::session();
        session.execute("comp.new", json!({"name": "Scene", "width": size[0], "height": size[1], "frameRate": 24, "duration": 2})).expect("comp.new");
        let solid =
            session.execute("layer.newSolid", json!({"name": "Black", "color": "#000000", "width": size[0], "height": size[1]})).expect("layer.newSolid");
        let layer = solid["layer"].as_u64().expect("a layer id");
        Comp { session, layer }
    }

    fn forge(&mut self, values: &[(&str, serde_json::Value)]) {
        self.session.execute("effect.apply", json!({"layer": self.layer, "effect": EFFECT})).expect("effect.apply");
        for (id, value) in values {
            self.session.execute("prop.set", json!({"layer": self.layer, "path": format!("effects/#1/{id}"), "value": value})).expect("prop.set");
        }
    }

    fn white_sphere(&mut self, size: f64) {
        self.forge(&[
            ("model", json!(1)),
            ("Transform/size", json!(size)),
            ("Material/colour", json!([1.0, 1.0, 1.0, 1.0])),
            ("Material/specular", json!(0.0)),
            ("Material/rim", json!(0.0)),
            ("Lighting/ambient", json!(0.0)),
            ("smooth", json!(false)),
        ]);
    }

    fn frame(&self, gpu: &'static effectcraft_gpu::Gpu) -> Image {
        let comp = self.session.active_comp_id().expect("an active comp");
        let renderer = Renderer::new(&self.session.project, self.session.footage.as_ref(), RenderOpts { backend: Backend::Gpu, ..Default::default() });
        renderer.with_accel(Some(gpu)).comp_frame(comp, Tick::from_seconds_f64(0.0))
    }
}

fn luminance(pixel: &[f32; 4]) -> f64 {
    f64::from(0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2])
}

fn mean_luminance(image: &Image, inside: impl Fn(u32, u32) -> bool) -> f64 {
    let (mut sum, mut count) = (0.0, 0usize);
    for y in 0..image.height {
        for x in 0..image.width {
            if inside(x, y) {
                sum += luminance(&image.data[(y * image.width + x) as usize]);
                count += 1;
            }
        }
    }
    sum / count.max(1) as f64
}

fn sphere_lit_by(light: serde_json::Value) -> Option<Image> {
    let gpu = gpu()?;
    let mut comp = Comp::new([160, 160]);
    comp.white_sphere(50.0);
    comp.session.execute("layer.newLight", light).expect("layer.newLight");
    Some(comp.frame(gpu))
}

fn halves(image: &Image) -> [f64; 4] {
    let (w, h) = (image.width, image.height);
    [
        mean_luminance(image, |x, _| x < w / 2),
        mean_luminance(image, |x, _| x >= w / 2),
        mean_luminance(image, |_, y| y < h / 2),
        mean_luminance(image, |_, y| y >= h / 2),
    ]
}

#[test]
fn a_point_light_on_the_right_lights_the_right_half() {
    let Some(image) = sphere_lit_by(json!({"kind": "Point", "name": "Right", "position": [400.0, 80.0, 0.0]})) else { return };
    let [left, right, _, _] = halves(&image);
    assert!(right > left + 0.05, "right {right:.3} against left {left:.3}");
}

#[test]
fn a_point_light_above_lights_the_top_half() {
    let Some(image) = sphere_lit_by(json!({"kind": "Point", "name": "Above", "position": [80.0, -240.0, 0.0]})) else { return };
    let [_, _, top, bottom] = halves(&image);
    assert!(top > bottom + 0.05, "top {top:.3} against bottom {bottom:.3}");
}

#[test]
fn a_point_light_in_front_lights_the_sphere_and_one_behind_does_not() {
    let in_front = sphere_lit_by(json!({"kind": "Point", "name": "Front", "position": [80.0, 80.0, -400.0]}));
    let behind = sphere_lit_by(json!({"kind": "Point", "name": "Behind", "position": [80.0, 80.0, 400.0]}));
    let (Some(in_front), Some(behind)) = (in_front, behind) else { return };
    let disc = |x: u32, y: u32| (f64::from(x) - 80.0).powi(2) + (f64::from(y) - 80.0).powi(2) < 40.0f64.powi(2);
    let (front, back) = (mean_luminance(&in_front, disc), mean_luminance(&behind, disc));
    assert!(front > 0.5 && front > back + 0.4, "front {front:.3} against behind {back:.3}");
}

#[test]
fn a_spot_light_lights_only_where_it_points_and_inside_its_cone() {
    let at_sphere = sphere_lit_by(json!({"kind": "Spot", "name": "At", "position": [400.0, 80.0, 0.0], "poi": [80.0, 80.0, 0.0], "coneAngle": 60.0}));
    let away = sphere_lit_by(json!({"kind": "Spot", "name": "Away", "position": [400.0, 80.0, 0.0], "poi": [800.0, 80.0, 0.0], "coneAngle": 60.0}));
    let narrow = sphere_lit_by(
        json!({"kind": "Spot", "name": "Narrow", "position": [80.0, 80.0, -400.0], "poi": [80.0, 80.0, 0.0], "coneAngle": 6.0, "coneFeather": 0.0}),
    );
    let (Some(at_sphere), Some(away), Some(narrow)) = (at_sphere, away, narrow) else { return };
    let [left, right, _, _] = halves(&at_sphere);
    assert!(right > left + 0.05, "aimed at the sphere from the right: right {right:.3} against left {left:.3}");
    let everything = mean_luminance(&away, |_, _| true);
    assert!(everything < 0.002, "aimed away from the sphere it is dark: {everything:.4}");
    let distance = |x: u32, y: u32| ((f64::from(x) - 80.0).powi(2) + (f64::from(y) - 80.0).powi(2)).sqrt();
    let (centre, edge) = (mean_luminance(&narrow, |x, y| distance(x, y) < 8.0), mean_luminance(&narrow, |x, y| (34.0..44.0).contains(&distance(x, y))));
    assert!(centre > 0.5 && edge < 0.01, "a 6 degree cone lights the centre {centre:.3} and not the rim {edge:.3}");
}

#[test]
fn a_parallel_light_shines_along_its_direction() {
    let from_right = sphere_lit_by(json!({"kind": "Parallel", "name": "Right", "position": [400.0, 80.0, 0.0], "poi": [80.0, 80.0, 0.0]}));
    let from_below = sphere_lit_by(json!({"kind": "Parallel", "name": "Below", "position": [80.0, 400.0, 0.0], "poi": [80.0, 80.0, 0.0]}));
    let (Some(from_right), Some(from_below)) = (from_right, from_below) else { return };
    let [left, right, _, _] = halves(&from_right);
    assert!(right > left + 0.05, "from the right: right {right:.3} against left {left:.3}");
    let [_, _, top, bottom] = halves(&from_below);
    assert!(bottom > top + 0.05, "from below: bottom {bottom:.3} against top {top:.3}");
}
