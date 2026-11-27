use std::collections::BTreeMap;

use craft_plugin::{Cx, Frame, ParamKind, RenderCx, Value};

use crate::path::Path;
use crate::settings::Settings;

const SIZE: [usize; 2] = [160, 90];
const COLUMNS: usize = 8;
const ROWS: usize = 6;

fn sampled_bytes(preset: f64) -> Vec<u8> {
    let mut params: BTreeMap<String, Value> = crate::spec::effect()
        .params
        .into_iter()
        .filter_map(|p| {
            let value = match p.kind {
                ParamKind::Slider { default, .. } | ParamKind::Percent { default, .. } => Value::Number(default),
                ParamKind::Popup { default, .. } => Value::Number(f64::from(default)),
                ParamKind::Point { default } => Value::List(vec![default[0] * SIZE[0] as f64, default[1] * SIZE[1] as f64]),
                ParamKind::Color { default } => Value::List(default.to_vec()),
                _ => return None,
            };
            Some((p.id, value))
        })
        .collect();
    params.insert(crate::spec::PRESET.into(), Value::Number(preset));
    let cx = RenderCx {
        params,
        time: 1.25,
        scale: 1.0,
        layer_size: [SIZE[0] as f64, SIZE[1] as f64],
        size: [SIZE[0] as u32, SIZE[1] as u32],
        instance: 7,
        ..Default::default()
    };
    let settings = Settings::read(&Cx::new(cx));
    let paths = [Path::new(vec![[16.0, 20.0], [150.0, 70.0]], false)];
    let source = vec![[0.25, 0.5, 0.75, 1.0]; SIZE[0] * SIZE[1]];
    let mut pixels = vec![[0.0; 4]; SIZE[0] * SIZE[1]];
    let mut frame = Frame::whole(SIZE[0], SIZE[1], &source, &mut pixels);
    crate::render::render(&settings, &paths, SIZE, &mut frame);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    (0..ROWS).flat_map(|j| (0..COLUMNS).map(move |i| (7 + 15 * j) * SIZE[0] + 10 + 20 * i)).flat_map(|at| pixels[at].into_iter().take(3).map(byte)).collect()
}

const DEFAULT_BYTES: [u8; 144] = [
    174, 255, 255, 188, 255, 255, 179, 255, 255, 148, 255, 255, 115, 252, 255, 90, 197, 255, 75, 159, 255, 68, 141, 220, 211, 255, 255, 255, 255, 255, 230,
    255, 255, 181, 255, 255, 148, 255, 255, 123, 255, 255, 102, 225, 255, 90, 198, 255, 143, 255, 255, 196, 255, 255, 255, 255, 255, 255, 255, 255, 231, 255,
    255, 187, 255, 255, 151, 255, 255, 128, 255, 255, 110, 242, 255, 141, 255, 255, 180, 255, 255, 214, 255, 255, 255, 255, 255, 255, 255, 255, 220, 255, 255,
    169, 255, 255, 86, 187, 255, 101, 223, 255, 123, 255, 255, 145, 255, 255, 170, 255, 255, 199, 255, 255, 255, 255, 255, 255, 255, 255, 69, 143, 224, 73,
    155, 249, 82, 179, 255, 96, 212, 255, 117, 255, 255, 144, 255, 255, 166, 255, 255, 173, 255, 255,
];

const ELECTRIC_BYTES: [u8; 144] = [
    233, 255, 255, 233, 255, 255, 219, 255, 255, 182, 255, 255, 136, 218, 255, 99, 173, 255, 79, 147, 239, 71, 136, 214, 253, 255, 255, 255, 255, 255, 255,
    255, 255, 236, 255, 255, 204, 255, 255, 169, 255, 255, 133, 215, 255, 110, 187, 255, 249, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
    255, 248, 255, 255, 207, 255, 255, 169, 255, 255, 227, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 252, 255, 255,
    213, 255, 255, 170, 255, 255, 213, 255, 255, 253, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 104, 180, 255, 138,
    220, 255, 181, 255, 255, 216, 255, 255, 244, 255, 255, 255, 255, 255, 255, 255, 255, 246, 255, 255,
];

fn assert_within_one_level(preset: f64, golden: &[u8; 144]) {
    let worst = sampled_bytes(preset).iter().zip(golden).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 1, "largest 8-bit difference {worst}");
}

#[test]
fn the_default_preset_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(0.0, &DEFAULT_BYTES);
}

#[test]
fn the_pinned_electric_preset_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(2.0, &ELECTRIC_BYTES);
}
