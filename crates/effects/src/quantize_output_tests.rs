use effectcraft_keyframe::Value;

use crate::{Buf, EffectCtx, EffectEnv, Image, Params, QUANTIZES_OUTPUT, apply, default_value, find, num};

fn input(quantized: Option<f32>) -> Image {
    let (w, h) = (37, 23);
    let mut img = Image::new(w, h);
    for (i, p) in img.data.iter_mut().enumerate() {
        let (x, y) = ((i % w as usize) as f32, (i / w as usize) as f32);
        let a = match i % 11 {
            0 => 0.0,
            1 => -0.2,
            2 => 1.3,
            _ => ((x * 0.37 + y * 0.11).sin() * 0.5 + 0.5).clamp(0.0, 1.0),
        };
        let c = |k: f32| (x * k + y * (1.0 - k) * 0.7).cos() * 0.8 + 0.45;
        *p = [c(0.21) * a, c(0.53) * a.max(0.3), c(0.87) * a + 0.01, a];
    }
    img.data[5] = [0.4, 0.2, 0.1, 0.0];
    img.data[17] = [f32::NAN, 0.3, 0.2, 0.6];
    img.data[29] = [0.2, 0.2, 0.2, f32::NAN];
    if let Some(l) = quantized {
        img.data.iter_mut().for_each(|p| effectcraft_raster::quantize_px(p, l, 1.0 / l));
    }
    img
}

fn run(id: &str, vals: &[(&str, Value)], img: Image, adjustment: bool, env: EffectEnv) -> Buf {
    let spec = find(id).unwrap();
    let size = [img.width as f64, img.height as f64];
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), default_value(p, size))).collect() };
    for (k, v) in vals {
        assert!(params.values.contains_key(*k), "{id}: unknown param {k}");
        params.values.insert(k.to_string(), v.clone());
    }
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: size, seed: 1, adjustment, env };
    apply(spec, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
}

fn bits(b: &Buf) -> (u32, u32, [u64; 2], Vec<u32>) {
    (b.img.width, b.img.height, b.offset.map(f64::to_bits), b.img.data.iter().flatten().map(|v| v.to_bits()).collect())
}

#[test]
fn effects_that_quantise_their_output_write_the_bytes_of_a_separate_quantise_pass() {
    let levels_red_gamma = crate::color_fx::levels_channel_ids(1)[2].clone();
    let levels_alpha_white = crate::color_fx::levels_channel_ids(4)[4].clone();
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("ec.blur.gaussian", vec![]),
        ("ec.blur.gaussian", vec![("blurriness", num(12.0))]),
        ("ec.blur.gaussian", vec![("blurriness", num(0.6))]),
        ("ec.blur.gaussian", vec![("blurriness", num(9.0)), ("dimensions", Value::Enum(1)), ("repeatEdge", Value::Bool(true))]),
        ("ec.blur.gaussian", vec![("blurriness", num(7.0)), ("dimensions", Value::Enum(2))]),
        ("ec.color.levels", vec![]),
        ("ec.color.levels", vec![("gamma", num(1.7)), ("outWhite", num(0.8)), ("inBlack", num(0.1))]),
        ("ec.color.levels", vec![(levels_red_gamma.as_str(), num(0.6)), (levels_alpha_white.as_str(), num(0.7))]),
        ("ec.color.curves", vec![]),
        ("ec.color.curves", vec![("rgb", Value::Str("0,0 0.4,0.65 1,1".into()))]),
        ("ec.color.curves", vec![("alpha", Value::Str("0,0.2 1,0.9".into())), ("blue", Value::Str("0,0 0.5,0.2 1,1".into()))]),
        ("ec.color.huesaturation", vec![]),
        ("ec.color.huesaturation", vec![("hue", num(40.0)), ("saturation", num(30.0)), ("lightness", num(-20.0))]),
        ("ec.color.huesaturation", vec![("colorize", Value::Bool(true)), ("colorizeHue", num(200.0))]),
        ("ec.stylize.glow", vec![]),
        ("ec.stylize.glow", vec![("threshold", num(20.0)), ("radius", num(6.0)), ("intensity", num(2.5)), ("operation", Value::Enum(1))]),
        ("ec.stylize.glow", vec![("operation", Value::Enum(2)), ("based", Value::Enum(0))]),
        ("ec.stylize.glow", vec![("glowOperation", Value::Enum(3)), ("colors", Value::Enum(1))]),
    ];
    for id in QUANTIZES_OUTPUT {
        assert!(cases.iter().any(|(c, _)| c == id), "{id} has no case");
    }
    let mut compared = 0;
    for levels in [255.0f32, 32768.0] {
        for input_quantized in [false, true] {
            for adjustment in [false, true] {
                for (id, vals) in &cases {
                    let img = || input(input_quantized.then_some(levels));
                    let mut want = run(id, vals, img(), adjustment, EffectEnv::default());
                    want.img.data.iter_mut().for_each(|p| effectcraft_raster::quantize_px(p, levels, 1.0 / levels));
                    let env = EffectEnv { quantize_output: Some(levels), input_quantized, ..EffectEnv::default() };
                    let got = run(id, vals, img(), adjustment, env);
                    assert!(bits(&got) == bits(&want), "{id} {vals:?} levels {levels} input quantised {input_quantized} adjustment {adjustment}");
                    compared += 1;
                }
            }
        }
    }
    assert_eq!(compared, 144);
}
