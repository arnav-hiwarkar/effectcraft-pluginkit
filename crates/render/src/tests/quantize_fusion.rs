use effectcraft_effects::{EffectCtx, EffectEnv};
use effectcraft_keyframe::Value;
use effectcraft_project::{BitDepth, GroupKind};
use effectcraft_time::Tick;

use super::{add_effect, setup, solid};
use crate::{NoFootage, RenderOpts, Renderer};

#[test]
fn fused_and_plain_effects_give_the_bytes_of_quantising_after_each_effect() {
    for depth in [BitDepth::Bpc8, BitDepth::Bpc16, BitDepth::Bpc32] {
        let (mut p, cid, comp) = setup();
        p.settings.bit_depth = depth;
        let mut l = solid(&mut p, &comp, [0.83, 0.41, 0.27], 120, 60);
        let size = [120.0, 60.0];
        add_effect(&mut p, &mut l, "ec.blur.gaussian", size, &[("blurriness", Value::Scalar(9.0))]);
        add_effect(&mut p, &mut l, "ec.color.levels", size, &[("gamma", Value::Scalar(1.4)), ("outWhite", Value::Scalar(0.9))]);
        add_effect(&mut p, &mut l, "ec.color.tint", size, &[("amount", Value::Scalar(35.0))]);
        add_effect(&mut p, &mut l, "ec.color.curves", size, &[("rgb", Value::Str("0,0 0.3,0.62 1,1".into()))]);
        add_effect(&mut p, &mut l, "ec.color.huesaturation", size, &[("hue", Value::Scalar(25.0)), ("saturation", Value::Scalar(20.0))]);
        add_effect(&mut p, &mut l, "ec.stylize.glow", size, &[("threshold", Value::Scalar(80.0)), ("radius", Value::Scalar(7.0))]);
        add_effect(&mut p, &mut l, "ec.color.tint", size, &[("amount", Value::Scalar(37.0)), ("white", Value::Color([0.71, 0.33, 0.52, 1.0]))]);
        add_effect(&mut p, &mut l, "ec.color.curves", size, &[]);
        p.comp_mut(cid).unwrap().layers.push(l);
        let comp = p.comp(cid).unwrap();
        let layer = &comp.layers[0];
        let r = Renderer::new(&p, &NoFootage, RenderOpts::default());
        let ctx = r.ctx(cid, comp, Tick::from_seconds_f64(0.5));
        let source = r.source(&ctx, layer).unwrap();
        let got = r.apply_effects(&ctx, layer, source.clone(), false);

        let pipe = crate::color::Pipe::of(&p.settings);
        let (layer_size, origin) = ctx.effect_bounds(layer);
        let mut want = source;
        want.rebase(origin);
        want.cover(layer_size);
        let mut ran = 0;
        for g in layer.effects().unwrap().groups() {
            let GroupKind::Effect { effect } = &g.kind else { continue };
            let spec = effectcraft_effects::find(effect).unwrap();
            let params = effectcraft_effects::flatten_params(g, &mut |pr| ctx.value(layer, pr));
            let env = EffectEnv { float_depth: pipe.levels.is_none(), ..EffectEnv::default() };
            let ectx = EffectCtx { params: &params, time: 0.5, layer_size, seed: g.uid as u32, adjustment: false, env };
            want = effectcraft_effects::apply(spec, &ectx, want);
            pipe.quantize(&mut want.img);
            ran += 1;
        }
        want.rebase([-origin[0], -origin[1]]);
        assert_eq!(ran, 8);
        assert_eq!((got.img.width, got.img.height, got.offset), (want.img.width, want.img.height, want.offset), "{depth:?}");
        let bits = |b: &effectcraft_effects::Buf| b.img.data.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert!(bits(&got) == bits(&want), "{depth:?}");
        let distinct: std::collections::BTreeSet<u32> = got.img.data.iter().map(|p| p[0].to_bits()).collect();
        assert!(distinct.len() > 20, "{depth:?}: {}", distinct.len());
    }
}
