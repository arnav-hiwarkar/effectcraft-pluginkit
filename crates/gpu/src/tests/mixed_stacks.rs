use super::*;
use crate::walk::SHARED_PICTURES;

fn traffic(g: &crate::Gpu, s: &Scene, shared: bool, t: Tick) -> (Image, u64, u64) {
    let mut r = Renderer::new(&s.p, &Pattern, RenderOpts { backend: Backend::Gpu, ..opts() });
    r.accel = Some(g);
    SHARED_PICTURES.store(shared, std::sync::atomic::Ordering::Relaxed);
    let before = g.ctx.transfer_stats();
    let img = g.render(&r, s.cid, t);
    let after = g.ctx.transfer_stats();
    SHARED_PICTURES.store(true, std::sync::atomic::Ordering::Relaxed);
    (img.expect("the GPU renders the frame"), after.upload_bytes - before.upload_bytes, after.readback_bytes - before.readback_bytes)
}

#[test]
fn a_layer_whose_stack_starts_on_the_cpu_uploads_once_after_its_cpu_effects() {
    let Some(g) = gpu() else { return };
    let image = 97 * 61 * 16;
    let gpu_levels: &[(&str, &[(&str, f64)])] = &[("ec.blur.gaussian", &[("blurriness", 20.0)]), ("ec.color.levels", &[("gamma", 1.4)])];
    let levels_cpu_levels: &[(&str, &[(&str, f64)])] =
        &[("ec.color.levels", &[("gamma", 1.4)]), ("ec.generate.fractal", &[]), ("ec.color.levels", &[("gamma", 0.8)])];
    let cpu_levels: &[(&str, &[(&str, f64)])] = &[("ec.generate.fractal", &[]), ("ec.color.levels", &[("gamma", 1.4)])];
    let cases = [
        ("GPU effect + Levels", gpu_levels, (image, image), (image, image)),
        ("Levels + CPU effect + Levels", levels_cpu_levels, (2 * image, 2 * image), (2 * image, 2 * image)),
        ("CPU effect + Levels", cpu_levels, (2 * image, 2 * image), (image, image)),
    ];
    for (label, stack, uploaded_first, source_on_cpu) in cases {
        let mut s = Scene::new(BitDepth::Bpc32);
        let mut l = s.footage(97, 61);
        for (id, vals) in stack {
            let vals: Vec<(&str, Value)> = vals.iter().map(|(k, v)| (*k, n(*v))).collect();
            s.effect(&mut l, id, &vals);
        }
        s.push(l);
        let t = Tick::from_seconds_f64(0.2);
        let (old, old_up, old_down) = traffic(g, &s, true, t);
        let (new, new_up, new_down) = traffic(g, &s, false, t);
        assert_eq!((old_up, old_down), uploaded_first, "{label}: the source uploaded first");
        assert_eq!((new_up, new_down), source_on_cpu, "{label}: the source kept on the CPU");
        assert!(old.data == new.data, "{label}: the frame changed");
        check(label, compare_at(&s, opts(), t), 0.0);
    }
}

#[test]
fn a_shape_whose_stack_starts_on_the_cpu_covers_its_layer_before_the_cpu_effect() {
    for depth in [BitDepth::Bpc8, BitDepth::Bpc32] {
        let mut s = Scene::new(depth);
        let mut l = build::layer(&mut s.p, &s.comp, "Shape", LayerSource::Shape, (97, 61), None);
        let mut next = s.p.next_id;
        let mut ids = Ids(&mut next);
        let rect = build::shape_rect(&mut ids, [40.0, 24.0], [6.0, -5.0], 0.0);
        let fill = build::shape_fill(&mut ids, [0.2, 0.8, 0.3, 1.0]);
        let g = build::shape_group(&mut ids, "Rectangle 1", vec![rect, fill]);
        s.p.next_id = next;
        l.props.sub_mut("contents").expect("a shape layer has contents").children.push(g.into());
        s.effect(&mut l, "ec.generate.fractal", &[]);
        s.effect(&mut l, "ec.color.levels", &[("gamma", n(1.4))]);
        s.push(l);
        check(&format!("shape, CPU effect first {depth:?}"), compare_at(&s, opts(), Tick::from_seconds_f64(0.2)), 0.0);
    }
}

fn shape_layer(s: &mut Scene, size: [f64; 2], position: [f64; 2]) -> Layer {
    let mut l = build::layer(&mut s.p, &s.comp, "Shape", LayerSource::Shape, (97, 61), None);
    let mut next = s.p.next_id;
    let mut ids = Ids(&mut next);
    let rect = build::shape_rect(&mut ids, size, position, 0.0);
    let fill = build::shape_fill(&mut ids, [0.2, 0.8, 0.3, 1.0]);
    let g = build::shape_group(&mut ids, "Rectangle 1", vec![rect, fill]);
    s.p.next_id = next;
    l.props.sub_mut("contents").expect("a shape layer has contents").children.push(g.into());
    l
}

#[test]
fn a_cover_over_the_texture_limit_stays_on_the_gpu() {
    let Some(g) = gpu() else { return };
    let small = Gpu::from_context(g.ctx.with_max_dim(128));
    let mut s = Scene::new(BitDepth::Bpc32);
    let mut l = shape_layer(&mut s, [40.0, 24.0], [6.0, -5.0]);
    set(&mut l, "transform/scale", Value::Vec3([200.0, 200.0, 100.0]));
    s.effect(&mut l, "ec.generate.fractal", &[]);
    s.effect(&mut l, "ec.color.levels", &[("gamma", n(1.4))]);
    s.push(l);
    let mut r = Renderer::new(&s.p, &Pattern, RenderOpts { backend: Backend::Gpu, ..opts() });
    r.accel = Some(&small);
    assert!(small.render(&r, s.cid, Tick::from_seconds_f64(0.2)).is_some(), "the GPU compositor returns the frame");
}

#[test]
fn a_cpu_fallback_after_a_pending_alignment_keeps_the_grid_phase() {
    use effectcraft_render::{FxStep, FxTarget};
    let Some(g) = gpu() else { return };
    let mut s = Scene::new(BitDepth::Bpc16);
    let mut l = shape_layer(&mut s, [40.0, 24.0], [6.0, -5.0]);
    s.effect(&mut l, "ec.blur.gaussian", &[("blurriness", n(2.0))]);
    s.push(l);
    check("shape, small Gaussian Blur 16 bpc", compare_at(&s, opts(), Tick::from_seconds_f64(0.2)), 0.0);

    let spec = effectcraft_effects::find("ec.blur.gaussian").unwrap();
    let size = [97.0, 61.0];
    let mut params =
        effectcraft_effects::Params { values: spec.params.iter().map(|ps| (ps.id.to_string(), effectcraft_effects::default_value(ps, size))).collect() };
    params.values.insert("blurriness".into(), n(2.0));
    let ctx = effectcraft_effects::EffectCtx { params: &params, time: 0.0, layer_size: size, seed: 1, adjustment: false, env: Default::default() };
    let source = effectcraft_effects::Buf { img: pattern(3, 41, 25), offset: [-70.0, -67.0], scale: 1.0 };
    let mut aligned = source.clone();
    aligned.align(size);
    assert!(aligned.img.width > source.img.width && aligned.img.height > source.img.height, "the fixture needs a nonzero alignment");
    let mut e = crate::context::Enc::new(&g.ctx);
    let mut fx = crate::effects::GpuFx::source(&mut e, std::sync::Arc::new(source), None);
    fx.align(size);
    assert!(!fx.gpu(&[FxStep { spec, ctx }]), "the GPU declines the small blur");
    let mut seen = None;
    fx.cpu(&mut |buf| {
        seen = Some((buf.img.width, buf.img.height, buf.offset));
        buf
    });
    assert_eq!(seen, Some((aligned.img.width, aligned.img.height, aligned.offset)), "the CPU blur sees the aligned buffer");
}
