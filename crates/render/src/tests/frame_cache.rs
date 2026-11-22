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
    assert!(img == render_cached(&p, cid, t, None));
}

#[test]
fn a_comp_with_animation_composites_every_frame() {
    let (mut p, cid) = static_scene();
    p.comp_mut(cid).unwrap().layers[1].props.prop_mut("transform/position").unwrap().keys =
        vec![Keyframe::new(Tick::ZERO, Value::Vec3([20.0, 20.0, 0.0])), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Vec3([180.0, 80.0, 0.0]))];
    let cache = LayerCache::default();
    profiled(&p, cid, Tick::ZERO, &cache);
    let t = Tick::from_seconds_f64(0.5);
    let (img, drawn) = profiled(&p, cid, t, &cache);
    assert_eq!(drawn, 3);
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
