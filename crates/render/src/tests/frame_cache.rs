use effectcraft_color::BlendMode;
use effectcraft_keyframe::{Keyframe, Value};
use effectcraft_project::{BitDepth, ItemId, ItemKind, LayerSource, MatteKind, Project, TrackMatte};
use effectcraft_time::Tick;

use super::{Edit, add_effect_200, render_cached, setup, solid};
use crate::{Image, LayerCache, NoFootage, RenderOpts, Renderer};

fn static_scene() -> (Project, ItemId) {
    let (mut p, cid, comp) = setup();
    p.settings.bit_depth = BitDepth::Bpc8;
    let mut layers = vec![];
    for (i, c) in [[0.8, 0.3, 0.2], [0.1, 0.6, 0.9], [0.4, 0.9, 0.3]].into_iter().enumerate() {
        let mut l = solid(&mut p, &comp, c, 120, 60);
        add_effect_200(&mut p, &mut l, "ec.color.huesaturation", &[("hue", Value::Scalar(30.0 * i as f64))]);
        l.props.prop_mut("transform/opacity").unwrap().value = Value::Scalar(50.0);
        l.props.prop_mut("transform/position").unwrap().value = Value::Vec3([60.0 + 40.0 * i as f64, 40.0 + 10.0 * i as f64, 0.0]);
        layers.push(l);
    }
    p.comp_mut(cid).unwrap().layers = layers.into();
    (p, cid)
}

fn profiled(p: &Project, cid: ItemId, t: Tick, cache: &LayerCache) -> (Image, usize) {
    let prof = std::sync::Mutex::new(vec![]);
    let mut r = Renderer::new(p, &NoFootage, RenderOpts::default());
    r.cache = Some(cache);
    r.profile = Some(&prof);
    let img = r.comp_frame(cid, t);
    (img, prof.into_inner().unwrap().len())
}

#[test]
fn a_frame_of_a_comp_where_nothing_animates_comes_from_the_cache_without_compositing() {
    let (p, cid) = static_scene();
    let cache = LayerCache::default();
    let (_, drawn) = profiled(&p, cid, Tick::ZERO, &cache);
    assert_eq!(drawn, 3);
    let before = cache.stats();
    let t = Tick::from_seconds_f64(0.5);
    let (img, drawn) = profiled(&p, cid, t, &cache);
    let after = cache.stats();
    assert_eq!((drawn, after.hits - before.hits, after.misses - before.misses), (0, 1, 0));
    let level = |n: [f32; 4]| n.map(|v| v * (1.0 / 255.0));
    assert_eq!((img.get(10, 15), img.get(190, 5)), (level([102.0, 39.0, 26.0, 128.0]), [0.0; 4]));
    assert!(img == render_cached(&p, cid, t, None));
}

fn moving(from: [f64; 3], to: [f64; 3]) -> Vec<Keyframe> {
    vec![Keyframe::new(Tick::ZERO, Value::Vec3(from)), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Vec3(to))]
}

fn static_stack_under_a_moving_layer() -> (Project, ItemId) {
    let (mut p, cid) = static_scene();
    let comp = p.comp(cid).unwrap().clone();
    let mut layers: Vec<_> = comp.layers.iter().cloned().collect();
    for c in [[0.2, 0.2, 0.7], [0.9, 0.9, 0.1]] {
        let mut l = solid(&mut p, &comp, c, 90, 40);
        l.props.prop_mut("transform/opacity").unwrap().value = Value::Scalar(60.0);
        layers.push(l);
    }
    let mut top = solid(&mut p, &comp, [1.0, 1.0, 1.0], 30, 30);
    top.props.prop_mut("transform/position").unwrap().keys = moving([20.0, 20.0, 0.0], [180.0, 80.0, 0.0]);
    layers.insert(0, top);
    p.comp_mut(cid).unwrap().layers = layers.into();
    (p, cid)
}

#[test]
fn a_comp_with_animation_composites_only_the_layers_from_the_first_animated_one_up() {
    let (p, cid) = static_stack_under_a_moving_layer();
    let cache = LayerCache::default();
    let (_, drawn) = profiled(&p, cid, Tick::ZERO, &cache);
    assert_eq!(drawn, 6);
    for t in [0.25, 0.5] {
        let t = Tick::from_seconds_f64(t);
        let (img, drawn) = profiled(&p, cid, t, &cache);
        assert_eq!(drawn, 1);
        assert!(img == render_cached(&p, cid, t, None));
    }
    let (img, _) = profiled(&p, cid, Tick::from_seconds_f64(0.5), &cache);
    assert_eq!((img.get(100, 50), img.get(5, 95)), ([1.0; 4], [0.0; 4]));
}

#[test]
fn a_static_layer_with_a_moving_parent_is_composited_every_frame() {
    let (mut p, cid) = static_stack_under_a_moving_layer();
    let c = p.comp_mut(cid).unwrap();
    let parent = c.layers[0].id;
    c.layers[5].parent = Some(parent);
    let cache = LayerCache::default();
    profiled(&p, cid, Tick::ZERO, &cache);
    let t = Tick::from_seconds_f64(0.5);
    let (img, drawn) = profiled(&p, cid, t, &cache);
    assert_eq!(drawn, 6);
    assert!(img == render_cached(&p, cid, t, None));
}

#[test]
fn every_edit_of_a_static_comp_renders_a_new_frame() {
    let (p, cid) = static_scene();
    let t = Tick::from_seconds_f64(0.5);
    let edits: Vec<(&str, Edit)> = vec![
        ("opacity", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[0].props.prop_mut("transform/opacity").unwrap().value = Value::Scalar(90.0))),
        (
            "position",
            Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[2].props.prop_mut("transform/position").unwrap().value = Value::Vec3([30.0, 30.0, 0.0])),
        ),
        ("blend mode", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[0].blend_mode = BlendMode::Difference)),
        ("effect value", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[1].props.prop_mut("effects/#1/hue").unwrap().value = Value::Scalar(170.0))),
        ("video switch", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[0].switches.video = false)),
        ("out point", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers[0].out_point = Tick::from_seconds_f64(0.25))),
        ("layer order", Box::new(|p, cid| p.comp_mut(cid).unwrap().layers.swap(0, 2))),
        (
            "parent",
            Box::new(|p, cid| {
                let c = p.comp_mut(cid).unwrap();
                let parent = c.layers[2].id;
                c.layers[0].parent = Some(parent);
            }),
        ),
        (
            "track matte",
            Box::new(|p, cid| {
                let c = p.comp_mut(cid).unwrap();
                let matte = c.layers[2].id;
                c.layers[1].track_matte = Some(TrackMatte { layer: matte, kind: MatteKind::Alpha });
            }),
        ),
        (
            "solid colour",
            Box::new(|p, cid| {
                let LayerSource::Solid { item } = p.comp(cid).unwrap().layers[1].source else { panic!("not a solid layer") };
                if let ItemKind::Solid(s) = &mut p.item_mut(item).unwrap().kind {
                    s.color = [0.9, 0.9, 0.1];
                }
            }),
        ),
        ("bit depth", Box::new(|p, _| p.settings.bit_depth = BitDepth::Bpc32)),
    ];
    for (what, edit) in edits {
        let cache = LayerCache::default();
        render_cached(&p, cid, Tick::ZERO, Some(&cache));
        let orig = render_cached(&p, cid, t, Some(&cache));
        let mut q = p.clone();
        edit(&mut q, cid);
        let cached = render_cached(&q, cid, t, Some(&cache));
        let fresh = render_cached(&q, cid, t, None);
        assert!(cached == fresh, "{what}");
        assert!(orig != fresh, "{what}: edit had no visible effect");
    }
}

#[test]
fn a_hidden_matte_that_ends_clears_the_cached_frame() {
    let (mut p, cid, comp) = setup();
    let fill = solid(&mut p, &comp, [1.0, 0.0, 0.0], 200, 100);
    let mut matte = solid(&mut p, &comp, [1.0, 1.0, 1.0], 200, 100);
    matte.switches.video = false;
    matte.out_point = Tick::from_seconds_f64(1.0);
    let fill = effectcraft_project::Layer { track_matte: Some(TrackMatte { layer: matte.id, kind: MatteKind::Alpha }), ..fill };
    p.comp_mut(cid).unwrap().layers = vec![matte, fill].into();
    let cache = LayerCache::default();
    let before = render_cached(&p, cid, Tick::from_seconds_f64(0.5), Some(&cache));
    let after = render_cached(&p, cid, Tick::from_seconds_f64(1.5), Some(&cache));
    assert_eq!((before.get(100, 50), after.get(100, 50)), ([1.0, 0.0, 0.0, 1.0], [0.0; 4]));
}

struct Plate(std::sync::atomic::AtomicBool);

impl crate::FootageSource for Plate {
    fn frame(&self, _: ItemId, _: &effectcraft_project::Footage, _: Tick) -> Option<std::sync::Arc<Image>> {
        self.0.load(std::sync::atomic::Ordering::SeqCst).then(|| std::sync::Arc::new(Image::filled(200, 100, [0.0, 1.0, 0.0, 1.0])))
    }
    fn recording(&self, reads: &crate::reads::ReadLog, f: &mut dyn FnMut()) {
        let stamp = match self.0.load(std::sync::atomic::Ordering::SeqCst) {
            true => crate::reads::FileStamp::Native { len: 1, modified: Some(1) },
            false => crate::reads::FileStamp::Missing,
        };
        reads.note(crate::reads::ReadFile::new("plate.png", stamp));
        f();
    }
}

#[test]
fn a_frame_rendered_while_its_still_was_missing_is_not_kept() {
    let (mut p, cid, comp) = setup();
    let f = effectcraft_project::Footage {
        path: "plate.png".into(),
        kind: effectcraft_project::FootageKind::Still,
        width: 200,
        height: 100,
        pixel_aspect: 1.0,
        has_video: true,
        ..Default::default()
    };
    let fid = p.add_item("plate", effectcraft_color::Label::Aqua, None, ItemKind::Footage(f));
    let l = effectcraft_project::build::layer(&mut p, &comp, "plate", LayerSource::Footage { item: fid }, (200, 100), None);
    p.comp_mut(cid).unwrap().layers = vec![l].into();
    let plate = Plate(false.into());
    let cache = LayerCache::default();
    let render = |t: f64| {
        let mut r = Renderer::new(&p, &plate, RenderOpts::default());
        r.cache = Some(&cache);
        r.comp_frame(cid, Tick::from_seconds_f64(t)).get(100, 50)
    };
    let missing = render(0.0);
    plate.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!((missing, render(0.5)), ([0.0; 4], [0.0, 1.0, 0.0, 1.0]));
}
