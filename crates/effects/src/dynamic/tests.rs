use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;

use super::{DynamicEffect, DynamicParam, register};
use crate::{Buf, EffectCtx, Image, Params};

const WHITE: [f32; 4] = [1.0; 4];

fn tint(id: &str, renders: Arc<AtomicUsize>) -> DynamicEffect {
    DynamicEffect {
        owner: "test.identity".into(),
        version: "1.0.0".into(),
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        params: vec![DynamicParam { id: "amount".into(), name: "Amount".into(), default: Value::Scalar(0.0), ui: ParamUi::default() }],
        render: Box::new(move |_, mut buf: Buf| {
            renders.fetch_add(1, Ordering::SeqCst);
            buf.img.data.iter_mut().for_each(|px| *px = WHITE);
            buf
        }),
        shader: None,
        gpu: None,
        time_dependent: false,
        reads_other_frames: false,
        reads_comp_scene: false,
        reads_param_history: false,
        presets: Vec::new(),
        revision: 1,
        files: None,
        margin: Some(Box::new(|_, _| Some(12.0))),
        state_version: 0,
        migrate: None,
        param_state: None,
        aux: Vec::new(),
        identity: Some(Box::new(|ctx: &EffectCtx| ctx.params.values.get("amount").is_some_and(|amount| amount.as_f64() == 0.0))),
    }
}

fn run(id: &str, amount: f64) -> (Vec<[f32; 4]>, Option<f64>) {
    let params = Params { values: [("amount".to_string(), Value::Scalar(amount))].into() };
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [4.0, 4.0], seed: 0, adjustment: false, env: Default::default() };
    let spec = crate::find(id).expect("registered");
    let out = crate::apply(spec, &ctx, Buf { img: Image::new(4, 4), offset: [0.0; 2], scale: 1.0 });
    (out.img.data, crate::pixel_margin(id, &ctx, 1.0))
}

#[test]
fn an_identity_effect_passes_its_input_through_without_rendering() {
    let renders = Arc::new(AtomicUsize::new(0));
    register(tint("test.identity.tint", renders.clone()));

    let (pixels, margin) = run("test.identity.tint", 0.0);
    assert_eq!(renders.load(Ordering::SeqCst), 0);
    assert!(pixels.iter().all(|px| *px != WHITE));
    assert_eq!(margin, Some(0.0));

    let (pixels, margin) = run("test.identity.tint", 0.5);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    assert!(pixels.iter().all(|px| *px == WHITE));
    assert_eq!(margin, Some(12.0));
}
