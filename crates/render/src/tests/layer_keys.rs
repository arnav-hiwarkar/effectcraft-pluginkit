use effectcraft_keyframe::{Ease, Interp, Keyframe, ShapePath, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Expression, ItemId, ItemKind, LayerSource, MaskMode, Project};
use effectcraft_time::Tick;

use super::{Edit, add_effect_200, setup, solid};
use crate::{NoFootage, RenderOpts, Renderer, cache};

fn scene() -> (Project, ItemId) {
    let (mut p, cid, comp) = setup();
    let mut l = solid(&mut p, &comp, [0.4, 0.5, 0.6], 200, 100);
    add_effect_200(&mut p, &mut l, "ec.blur.gaussian", &[("blurriness", Value::Scalar(4.0))]);
    add_effect_200(&mut p, &mut l, "ec.color.curves", &[("rgb", Value::Str("0,0 0.5,0.6 1,1".into()))]);
    l.props.prop_mut("effects/#1/blurriness").unwrap().keys =
        vec![Keyframe::new(Tick::ZERO, Value::Scalar(2.0)), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Scalar(8.0))];
    let mut next = p.next_id;
    let m = build::mask(&mut Ids(&mut next), "Mask 1", ShapePath::rect([50.0, 50.0], 40.0, 40.0), MaskMode::Add, [255, 255, 0]);
    p.next_id = next;
    l.props.sub_mut("masks").unwrap().children.push(m.into());
    p.comp_mut(cid).unwrap().layers.push(l);
    (p, cid)
}

fn key(p: &Project, cid: ItemId) -> u64 {
    let r = Renderer::new(p, &NoFootage, RenderOpts::default());
    let comp = p.comp(cid).unwrap();
    let ctx = r.ctx(cid, comp, Tick::from_seconds_f64(0.5));
    cache::layer_key(&ctx, &comp.layers[0], 1.0, false, false).unwrap()
}

fn layer(p: &mut Project, cid: ItemId) -> &mut effectcraft_project::Layer {
    &mut p.comp_mut(cid).unwrap().layers[0]
}

#[test]
fn every_edit_that_changes_a_layers_pixels_changes_its_key() {
    let (p, cid) = scene();
    let base = key(&p, cid);
    let changing: Vec<(&str, Edit)> = vec![
        ("static value", Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#2/rgb").unwrap().value = Value::Str("0,0 0.5,0.7 1,1".into()))),
        ("keyframe value", Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#1/blurriness").unwrap().keys[1].value = Value::Scalar(9.0))),
        ("keyframe time", Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#1/blurriness").unwrap().keys[1].time = Tick::from_seconds_f64(1.5))),
        ("keyframe interpolation", Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#1/blurriness").unwrap().keys[0].out_interp = Interp::Hold)),
        (
            "keyframe ease",
            Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#1/blurriness").unwrap().keys[0].out_ease = vec![Ease { speed: 1.0, influence: 50.0 }]),
        ),
        (
            "expression",
            Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#2/rgb").unwrap().expr = Some(Expression { text: "value".into(), enabled: false })),
        ),
        ("negative zero", Box::new(|p, cid| layer(p, cid).props.prop_mut("effects/#1/blurriness").unwrap().keys[0].value = Value::Scalar(-0.0))),
        ("effect disabled", Box::new(|p, cid| layer(p, cid).props.group_mut("effects/#2").unwrap().enabled = false)),
        ("effects reordered", Box::new(|p, cid| layer(p, cid).props.sub_mut("effects").unwrap().children.swap(0, 1))),
        (
            "effect added",
            Box::new(|p, cid| {
                let mut l = p.comp(cid).unwrap().layers[0].clone();
                add_effect_200(p, &mut l, "ec.channel.invert", &[]);
                *layer(p, cid) = l;
            }),
        ),
        (
            "mask mode",
            Box::new(|p, cid| {
                if let effectcraft_project::GroupKind::Mask { mode, .. } = &mut layer(p, cid).props.group_mut("masks/#1").unwrap().kind {
                    *mode = MaskMode::Subtract;
                }
            }),
        ),
        ("mask removed", Box::new(|p, cid| layer(p, cid).props.sub_mut("masks").unwrap().children.clear())),
        ("effects switch", Box::new(|p, cid| layer(p, cid).switches.effects = false)),
        (
            "solid colour",
            Box::new(|p, cid| {
                let LayerSource::Solid { item } = p.comp(cid).unwrap().layers[0].source else { panic!("not a solid layer") };
                if let ItemKind::Solid(s) = &mut p.item_mut(item).unwrap().kind {
                    s.color = [0.4, 0.5, 0.61];
                }
            }),
        ),
        ("bit depth", Box::new(|p, _| p.settings.bit_depth = effectcraft_project::BitDepth::Bpc16)),
    ];
    let mut keys = std::collections::BTreeSet::from([base]);
    for (what, edit) in &changing {
        let mut q = p.clone();
        edit(&mut q, cid);
        assert!(keys.insert(key(&q, cid)), "{what} kept a key already seen");
    }
    let keeping: Vec<(&str, Edit)> = vec![
        ("position", Box::new(|p, cid| layer(p, cid).props.prop_mut("transform/position").unwrap().value = Value::Vec3([10.0, 20.0, 0.0]))),
        ("audio switch", Box::new(|p, cid| layer(p, cid).switches.audio = !layer(p, cid).switches.audio)),
        ("name", Box::new(|p, cid| layer(p, cid).name = "Renamed".into())),
    ];
    for (what, edit) in &keeping {
        let mut q = p.clone();
        edit(&mut q, cid);
        assert_eq!(key(&q, cid), base, "{what}");
    }
}

#[test]
fn a_scene_reader_sampling_other_frames_gets_a_key_per_comp_frame_while_the_scene_holds() {
    use effectcraft_effects::dynamic::{self, DynamicEffect};
    let id = "test.keys.scene-ahead";
    dynamic::register(DynamicEffect {
        owner: "test.keys".into(),
        version: "1.0.0".into(),
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        params: Vec::new(),
        render: Box::new(|_, buf| buf),
        shader: None,
        gpu: None,
        time_dependent: false,
        reads_other_frames: true,
        reads_comp_scene: true,
        reads_param_history: false,
        presets: Vec::new(),
        revision: 1,
        files: None,
        margin: None,
        state_version: 0,
        migrate: None,
    });
    let (mut p, cid, comp) = setup();
    let mut l = solid(&mut p, &comp, [0.4, 0.5, 0.6], 200, 100);
    add_effect_200(&mut p, &mut l, id, &[]);
    p.comp_mut(cid).unwrap().layers.push(l);
    let r = Renderer::new(&p, &NoFootage, RenderOpts::default());
    let comp = p.comp(cid).unwrap();
    let key_at = |frame: i64| cache::layer_key(&r.ctx(cid, comp, Tick::from_seconds_f64(frame as f64 / 30.0)), &comp.layers[0], 1.0, false, false);
    let keys: std::collections::BTreeSet<_> = (10..13).map(key_at).collect();
    assert_eq!(keys.len(), 3, "three frames with an unchanged scene share a key: {keys:?}");
}
