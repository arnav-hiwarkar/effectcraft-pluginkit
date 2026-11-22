use effectcraft_engine::effects;
use effectcraft_engine::render::{Accelerator, FxStep};
use retro_check::engine::{ctx, gpu, params, picture, spec, worst};
use retro_check::{CRT, NEON};

const SIZE: [u32; 2] = [640, 512];
const TOLERANCE: f32 = 2e-3;

fn compare(id: &str, cases: &[(&str, Vec<(&str, f64)>, f64)]) {
    let spec = spec(id);
    let Some(gpu) = gpu() else { return };
    assert!(spec.gpu && gpu.supports_effect(spec.id), "{id} is offered to the GPU");
    for (name, values, time) in cases {
        let params = params(spec, SIZE, values);
        let cpu = effects::apply(spec, &ctx(&params, SIZE, *time), picture(SIZE));
        let (changed, _) = worst(&cpu.img, &picture(SIZE).img);
        assert!(changed > 0.1, "{id} {name}: the effect changes the picture");
        let out = gpu.effects(&[FxStep { spec, ctx: ctx(&params, SIZE, *time) }], &picture(SIZE), None).unwrap_or_else(|| panic!("{id} {name}: the GPU ran the plan"));
        assert_eq!((out.img.width, out.img.height), (cpu.img.width, cpu.img.height), "{id} {name}: frame size");
        let (difference, at) = worst(&cpu.img, &out.img);
        eprintln!("{id} {name}: worst {difference:.2e} at pixel {at}");
        assert!(difference < TOLERANCE, "{id} {name}: GPU and CPU differ by {difference} at pixel ({}, {})", at as u32 % SIZE[0], at as u32 / SIZE[0]);
    }
}

#[test]
fn crt_display_renders_the_same_on_the_gpu_and_the_cpu() {
    let banded = vec![("Screen/curvature", 0.02), ("Phosphor Glow/glow_radius", 4.0)];
    compare(
        CRT,
        &[
            ("default", vec![], 0.0),
            ("banded", banded.clone(), 0.0),
            ("slot mask in motion", [banded.clone(), vec![("Mask/mask", 1.0), ("Signal/noise", 20.0)]].concat(), 1.3),
            ("dots", vec![("Mask/mask", 2.0), ("Mask/pitch", 5.0), ("Signal/convergence", -3.5)], 0.4),
            ("strong curve", vec![("Screen/curvature", 0.25), ("Screen/corner", 0.2), ("Phosphor Glow/glow_radius", 40.0)], 0.0),
        ],
    );
}

#[test]
fn neon_outline_renders_the_same_on_the_gpu_and_the_cpu() {
    compare(
        NEON,
        &[
            ("default", vec![], 0.0),
            ("banded lines only", vec![("Bloom/radius", 6.0), ("lines_only", 1.0)], 0.0),
            ("alpha, thick, flicker", vec![("Edges/source", 1.0), ("Edges/thickness", 5.0), ("Bloom/flicker", 50.0)], 2.0),
            ("wide bloom", vec![("Bloom/radius", 70.0), ("Bloom/falloff", 90.0)], 0.0),
        ],
    );
}
