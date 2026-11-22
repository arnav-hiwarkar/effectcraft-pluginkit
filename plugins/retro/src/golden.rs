use craft_plugin::prelude::*;

use crate::crt::{self, Crt};
use crate::neon::{self, Neon};

const SIZE: [usize; 2] = [96, 54];
const COLUMNS: usize = 8;
const ROWS: usize = 6;

fn source() -> Vec<Pixel> {
    (0..SIZE[0] * SIZE[1])
        .map(|i| {
            let (x, y) = (i % SIZE[0], i / SIZE[0]);
            let checker = if (x / 8 + y / 6) % 2 == 0 { 0.8 } else { 0.1 };
            [x as f32 / 96.0, y as f32 / 54.0, checker, 1.0]
        })
        .collect()
}

fn sampled_bytes(render: impl Fn(&mut Frame)) -> Vec<u8> {
    let mut pixels = source();
    let mut frame = Frame { width: SIZE[0], height: SIZE[1], origin: [0, 0], pixels: &mut pixels };
    render(&mut frame);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    (0..ROWS).flat_map(|j| (0..COLUMNS).map(move |i| (4 + 9 * j) * SIZE[0] + 6 + 12 * i)).flat_map(|at| pixels[at].into_iter().take(3).map(byte)).collect()
}

fn crt_bytes(mask: u32) -> Vec<u8> {
    let crt = Crt {
        size: [96, 54],
        curvature: 0.1,
        corner: 0.05,
        mask,
        pitch: 2.0,
        mask_strength: 0.6,
        scan_strength: 0.5,
        scan_period: 3.0,
        roll: 0.7,
        convergence: 1.0,
        glow: 0.4,
        glow_sigma: 6.0,
        glow_threshold: 0.4,
        vignette: 0.3,
        noise: 0.0,
        flicker: 0.0,
        compensation: 1.0,
        contrast: 1.1,
        frame: 3,
    };
    sampled_bytes(|frame| crt::render(&crt, frame))
}

fn neon_bytes() -> Vec<u8> {
    let neon = Neon {
        size: [96, 54],
        alpha: false,
        threshold: 0.2,
        softness: 0.05,
        step: 1,
        core: [1.0, 0.9, 0.8, 1.0],
        glow: [0.0, 0.6, 1.0, 1.0],
        intensity: 1.0,
        sigma: 4.0,
        falloff: 0.5,
        flicker: 0.0,
        lines_only: false,
        frame: 3,
    };
    sampled_bytes(|frame| neon::render(&neon, frame))
}

const SLOT_BYTES: [u8; 144] = [
    11, 4, 88, 61, 8, 9, 117, 10, 9, 173, 11, 10, 227, 11, 105, 255, 10, 102, 255, 7, 19, 255, 4, 8, 19, 40, 98, 73, 45, 106, 130, 49, 10, 188, 51, 11, 243,
    51, 114, 255, 49, 112, 255, 46, 12, 255, 41, 16, 24, 81, 11, 79, 88, 12, 138, 93, 118, 199, 96, 122, 255, 97, 18, 255, 97, 19, 255, 94, 120, 255, 87, 112,
    25, 125, 16, 82, 135, 19, 143, 142, 126, 206, 147, 130, 255, 149, 26, 255, 148, 27, 255, 145, 127, 255, 134, 117, 21, 165, 109, 78, 181, 122, 141, 189, 28,
    205, 194, 29, 255, 196, 133, 255, 194, 131, 255, 189, 30, 255, 172, 30, 12, 178, 97, 66, 200, 22, 127, 212, 24, 189, 218, 26, 247, 220, 122, 255, 215, 119,
    255, 205, 35, 255, 180, 16,
];

const DOT_BYTES: [u8; 144] = [
    7, 2, 62, 43, 4, 5, 83, 6, 6, 123, 7, 6, 161, 7, 74, 194, 6, 72, 218, 4, 13, 230, 2, 5, 13, 28, 69, 51, 31, 75, 92, 34, 6, 133, 35, 7, 173, 35, 81, 209,
    35, 80, 238, 32, 8, 255, 29, 11, 3, 57, 25, 17, 62, 28, 32, 66, 249, 48, 69, 255, 65, 70, 34, 81, 70, 35, 96, 69, 245, 102, 63, 229, 4, 90, 31, 19, 99, 35,
    37, 105, 255, 56, 109, 255, 75, 111, 42, 93, 111, 43, 109, 109, 252, 114, 100, 235, 15, 122, 80, 57, 135, 90, 103, 142, 23, 151, 147, 25, 197, 148, 100,
    238, 148, 99, 255, 144, 27, 255, 129, 24, 8, 131, 71, 47, 149, 18, 92, 159, 20, 138, 164, 22, 182, 165, 91, 219, 162, 89, 246, 154, 28, 249, 133, 13,
];

const NEON_BYTES: [u8; 144] = [
    16, 19, 204, 48, 19, 204, 80, 19, 26, 112, 19, 26, 143, 20, 206, 175, 22, 209, 207, 22, 30, 239, 25, 35, 16, 62, 205, 48, 61, 204, 80, 61, 26, 112, 61, 26,
    143, 61, 204, 175, 61, 204, 207, 61, 26, 239, 63, 29, 16, 105, 28, 48, 104, 26, 80, 104, 204, 112, 104, 204, 143, 104, 26, 175, 104, 26, 207, 104, 204,
    239, 106, 207, 16, 148, 28, 48, 146, 26, 80, 146, 204, 112, 146, 204, 143, 146, 26, 175, 146, 26, 207, 146, 204, 239, 148, 207, 16, 190, 206, 48, 189, 204,
    80, 189, 26, 112, 189, 26, 143, 189, 204, 175, 189, 204, 207, 189, 26, 239, 191, 29, 16, 237, 214, 48, 236, 212, 80, 236, 34, 112, 236, 34, 143, 236, 212,
    175, 236, 212, 207, 236, 34, 239, 238, 36,
];

fn assert_within_one_level(now: Vec<u8>, golden: &[u8; 144]) {
    let worst = now.iter().zip(golden).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 1, "largest 8-bit difference {worst}");
}

#[test]
fn the_slot_mask_crt_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(crt_bytes(1), &SLOT_BYTES);
}

#[test]
fn the_dot_mask_crt_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(crt_bytes(2), &DOT_BYTES);
}

#[test]
fn neon_outline_matches_the_output_made_with_the_standard_library_within_one_level() {
    assert_within_one_level(neon_bytes(), &NEON_BYTES);
}
