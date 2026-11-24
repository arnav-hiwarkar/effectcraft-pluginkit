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
