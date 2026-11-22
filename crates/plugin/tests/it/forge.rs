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
