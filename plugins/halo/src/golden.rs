use std::collections::BTreeMap;

use craft_plugin::prelude::*;
use craft_plugin::{ParamKind, RenderCx, Value};

const SIZE: [usize; 2] = [160, 90];
const COLUMNS: usize = 8;
const ROWS: usize = 6;

fn defaults(overrides: &[(&str, f64)]) -> BTreeMap<String, Value> {
    let mut params: BTreeMap<String, Value> = crate::spec::spec()
        .params
        .into_iter()
        .filter_map(|p| {
            let value = match p.kind {
                ParamKind::Slider { default, .. } | ParamKind::Percent { default } | ParamKind::Angle { default } => Value::Number(default),
                ParamKind::Checkbox { default } => Value::Bool(default),
                ParamKind::Popup { default, .. } => Value::Number(f64::from(default)),
                ParamKind::Point { default } => Value::List(vec![default[0] * SIZE[0] as f64, default[1] * SIZE[1] as f64]),
                ParamKind::Color { default } => Value::List(default.to_vec()),
                _ => return None,
            };
            Some((p.id, value))
        })
        .collect();
    params.extend(overrides.iter().map(|(id, v)| (id.to_string(), Value::Number(*v))));
    params
}

fn sampled_bytes(overrides: &[(&str, f64)]) -> Vec<u8> {
    let cx = RenderCx {
        params: defaults(overrides),
        time: 1.25,
        scale: 1.0,
        layer_size: [SIZE[0] as f64, SIZE[1] as f64],
        size: [SIZE[0] as u32, SIZE[1] as u32],
        instance: 3,
        ..Default::default()
    };
    let mut pixels = vec![[0.25, 0.5, 0.75, 1.0]; SIZE[0] * SIZE[1]];
    let mut frame = Frame { width: SIZE[0], height: SIZE[1], origin: [0, 0], pixels: &mut pixels };
    crate::Halo.render(&Cx::new(cx), &mut frame).unwrap();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    (0..ROWS).flat_map(|j| (0..COLUMNS).map(move |i| (7 + 15 * j) * SIZE[0] + 10 + 20 * i)).flat_map(|at| pixels[at].into_iter().take(3).map(byte)).collect()
}

const SUN_BYTES: [u8; 144] = [
    113, 174, 236, 154, 213, 255, 156, 213, 255, 157, 213, 255, 113, 174, 236, 102, 164, 226, 92, 155, 217, 86, 148, 211, 132, 193, 254, 158, 212, 255, 242,
    255, 255, 160, 222, 255, 139, 198, 255, 131, 192, 253, 123, 184, 245, 112, 173, 235, 120, 182, 243, 148, 203, 255, 199, 255, 255, 165, 226, 255, 126, 183,
    246, 109, 172, 239, 100, 162, 224, 94, 156, 218, 117, 178, 239, 126, 186, 247, 163, 225, 255, 178, 235, 255, 126, 183, 254, 106, 168, 236, 95, 157, 222,
    86, 148, 211, 134, 194, 254, 116, 177, 238, 141, 201, 255, 115, 176, 237, 137, 198, 255, 103, 165, 234, 94, 156, 224, 85, 148, 210, 101, 163, 225, 106,
    167, 229, 131, 191, 251, 104, 166, 228, 106, 168, 230, 114, 176, 244, 86, 149, 212, 82, 145, 208,
];

const SCI_FI_BYTES: [u8; 144] = [
    153, 191, 229, 193, 216, 247, 221, 241, 255, 189, 218, 250, 158, 195, 231, 148, 188, 227, 136, 179, 222, 127, 173, 218, 181, 211, 241, 238, 253, 255, 255,
    255, 255, 240, 252, 255, 213, 234, 255, 176, 208, 239, 166, 201, 236, 146, 187, 227, 168, 202, 236, 223, 242, 255, 255, 255, 255, 227, 245, 255, 198, 223,
    255, 160, 197, 236, 150, 190, 229, 136, 179, 222, 163, 198, 233, 211, 235, 255, 205, 229, 255, 204, 227, 255, 173, 203, 241, 149, 189, 231, 142, 184, 228,
    125, 171, 217, 160, 197, 233, 174, 207, 239, 162, 197, 233, 215, 236, 255, 165, 197, 237, 147, 187, 231, 135, 179, 225, 123, 170, 217, 161, 197, 233, 153,
    191, 229, 149, 188, 227, 157, 194, 231, 165, 201, 239, 142, 184, 228, 124, 170, 217, 117, 166, 214,
];

fn assert_within_one_level(look: f64, golden: &[u8; 144]) {
    let now = sampled_bytes(&[("look", look)]);
    let worst = now.iter().zip(golden).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 1, "largest 8-bit difference {worst}");
}

#[test]
fn the_sun_look_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(0.0, &SUN_BYTES);
}

#[test]
fn the_sci_fi_look_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(3.0, &SCI_FI_BYTES);
}
