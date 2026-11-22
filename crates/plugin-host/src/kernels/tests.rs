use craft_plugin::{BlendMode, BlurShape, Filter, HostKernel, Pixel, Taps};

use super::{Image, blur_taps, run};

const SIZE: [u32; 2] = [37, 23];

fn picture(size: [u32; 2]) -> Vec<Pixel> {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 1000) as f32 / 1000.0
    };
    (0..size[0] * size[1])
        .map(|i| {
            let alpha = if i % 7 == 0 { 0.0 } else { next() };
            [next() * alpha * 3.0, next() * alpha, next() * alpha, alpha]
        })
        .collect()
}

fn at(pixels: &[Pixel], size: [u32; 2], x: isize, y: isize) -> Pixel {
    let [w, h] = size.map(|side| side as isize);
    if x < 0 || y < 0 || x >= w || y >= h { [0.0; 4] } else { pixels[(y * w + x) as usize] }
}

fn host(kernel: HostKernel, src: &[Pixel], src_size: [u32; 2], aux: Option<&[Pixel]>, size: [u32; 2]) -> Vec<Pixel> {
    let mut out = vec![[f32::NAN; 4]; (size[0] * size[1]) as usize];
    let aux = aux.map(|pixels| Image { size, pixels });
    run(&kernel, Image { size: src_size, pixels: src }, aux, size, &mut out);
    out
}

fn assert_close(got: &[Pixel], expected: &[Pixel], tolerance: f32, case: &str) {
    assert_eq!(got.len(), expected.len(), "{case}");
    let worst = got.iter().flatten().zip(expected.iter().flatten()).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(worst <= tolerance, "{case}: worst difference {worst}");
}

fn reference_convolve(src: &[Pixel], size: [u32; 2], x_taps: &[f32], y_taps: &[f32]) -> Vec<Pixel> {
    let (rx, ry) = ((x_taps.len() / 2) as isize, (y_taps.len() / 2) as isize);
    let mut out = Vec::new();
    for y in 0..size[1] as isize {
        for x in 0..size[0] as isize {
            let mut sum = [0.0; 4];
            for (j, wy) in y_taps.iter().enumerate() {
                for (i, wx) in x_taps.iter().enumerate() {
                    let p = at(src, size, x + i as isize - rx, y + j as isize - ry);
                    (0..4).for_each(|c| sum[c] += p[c] * wx * wy);
                }
            }
            out.push(sum);
        }
    }
    out
}

#[test]
fn blur_taps_have_their_textbook_shapes() {
    assert_eq!(blur_taps(BlurShape::Box, 1.5), vec![0.125, 0.25, 0.25, 0.25, 0.125]);
    assert_eq!(blur_taps(BlurShape::Box, 0.0), vec![1.0]);
    assert_eq!(blur_taps(BlurShape::Gaussian, 0.0), vec![1.0]);
    let gaussian = blur_taps(BlurShape::Gaussian, 3.0);
    let raw: Vec<f32> = (-3..=3).map(|i: i32| (-(i * i) as f32 / 2.0).exp()).collect();
    let total: f32 = raw.iter().sum();
    assert_eq!(gaussian.len(), 7);
    assert!(gaussian.iter().zip(&raw).all(|(got, raw)| (got - raw / total).abs() < 1e-7), "{gaussian:?}");
}

#[test]
fn blur_and_separable_convolve_match_a_direct_sum() {
    let src = picture(SIZE);
    let gaussian = host(HostKernel::Blur { shape: BlurShape::Gaussian, radius: [4.5, 2.0] }, &src, SIZE, None, SIZE);
    assert_close(&gaussian, &reference_convolve(&src, SIZE, &blur_taps(BlurShape::Gaussian, 4.5), &blur_taps(BlurShape::Gaussian, 2.0)), 1e-5, "gaussian");
    let boxed = host(HostKernel::Blur { shape: BlurShape::Box, radius: [0.0, 1.5] }, &src, SIZE, None, SIZE);
    assert_close(&boxed, &reference_convolve(&src, SIZE, &[1.0], &[0.125, 0.25, 0.25, 0.25, 0.125]), 1e-5, "box");
    let (x, y) = (vec![-1.0, 3.0, -1.0], vec![0.25, 0.5, 0.25, 0.0, 0.0]);
    let sharpened = host(HostKernel::Convolve { taps: Taps::Separable { x: x.clone(), y: y.clone() } }, &src, SIZE, None, SIZE);
    assert_close(&sharpened, &reference_convolve(&src, SIZE, &x, &y), 1e-5, "separable");
}

#[test]
fn full_convolve_matches_a_direct_sum() {
    let src = picture(SIZE);
    let weights: Vec<f32> = (0..15).map(|i| (i as f32 - 6.0) * 0.1).collect();
    let got = host(HostKernel::Convolve { taps: Taps::Full { width: 5, weights: weights.clone() } }, &src, SIZE, None, SIZE);
    let mut expected = Vec::new();
    for y in 0..SIZE[1] as isize {
        for x in 0..SIZE[0] as isize {
            let mut sum = [0.0; 4];
            for (k, w) in weights.iter().enumerate() {
                let p = at(&src, SIZE, x + (k % 5) as isize - 2, y + (k / 5) as isize - 1);
                (0..4).for_each(|c| sum[c] += p[c] * w);
            }
            expected.push(sum);
        }
    }
    assert_close(&got, &expected, 1e-5, "full");
}

fn reference_resample(src: &[Pixel], src_size: [u32; 2], filter: Filter, scale: [f32; 2], size: [u32; 2]) -> Vec<Pixel> {
    let weight = |d: f32| {
        let d = d.abs();
        match filter {
            Filter::Bilinear => (1.0 - d).max(0.0),
            Filter::Bicubic if d < 1.0 => 1.5 * d * d * d - 2.5 * d * d + 1.0,
            Filter::Bicubic if d < 2.0 => -0.5 * d * d * d + 2.5 * d * d - 4.0 * d + 2.0,
            Filter::Bicubic => 0.0,
        }
    };
    let axis = |o: u32, s: f32| {
        let (centre, widen) = ((o as f32 + 0.5) * s - 0.5, s.max(1.0));
        let taps: Vec<(isize, f32)> = (-300..300).map(|i| (i, weight((i as f32 - centre) / widen))).filter(|(_, w)| *w != 0.0).collect();
        let total: f32 = taps.iter().map(|(_, w)| w).sum();
        taps.into_iter().map(move |(i, w)| (i, w / total))
    };
    let mut out = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let mut sum = [0.0f32; 4];
            for (j, wy) in axis(y, scale[1]) {
                for (i, wx) in axis(x, scale[0]) {
                    let p = at(src, src_size, i, j);
                    (0..4).for_each(|c| sum[c] += p[c] * wx * wy);
                }
            }
            out.push(if filter == Filter::Bicubic { sum.map(|v| v.max(0.0)) } else { sum });
        }
    }
    out
}

#[test]
fn resample_matches_a_direct_sum_both_ways_with_either_filter() {
    let src = picture(SIZE);
    for (filter, size, scale) in [
        (Filter::Bilinear, [19, 12], None),
        (Filter::Bilinear, [19, 12], Some([2.0, 2.0])),
        (Filter::Bilinear, [74, 23], None),
        (Filter::Bicubic, [80, 50], None),
        (Filter::Bicubic, [9, 5], None),
    ] {
        let got = host(HostKernel::Resample { filter, scale }, &src, SIZE, None, size);
        let used = scale.unwrap_or([0, 1].map(|a| SIZE[a] as f32 / size[a] as f32));
        assert_close(&got, &reference_resample(&src, SIZE, filter, used, size), 1e-5, &format!("{filter:?} to {size:?} by {scale:?}"));
    }
}

#[test]
fn halving_by_two_uses_the_eighths_filter_and_doubling_the_quarters() {
    let mut src = vec![[0.0; 4]; 8 * 8];
    src[4 * 8 + 3] = [8.0; 4];
    let half = host(HostKernel::Resample { filter: Filter::Bilinear, scale: None }, &src, [8, 8], None, [4, 4]);
    let column: Vec<f32> = (0..4).map(|y| half[y * 4 + 1][0]).collect();
    assert_eq!(column, [0.0, 8.0 * 0.375 * 0.125, 8.0 * 0.375 * 0.375, 0.0]);
    let double = host(HostKernel::Resample { filter: Filter::Bilinear, scale: None }, &half, [4, 4], None, [8, 8]);
    let h = |x: usize, y: usize| half[y * 4 + x][0];
    let expected = 0.75 * (0.75 * h(1, 2) + 0.25 * h(0, 2)) + 0.25 * (0.75 * h(1, 1) + 0.25 * h(0, 1));
    assert!((double[4 * 8 + 2][0] - expected).abs() < 1e-7, "{} against {expected}", double[4 * 8 + 2][0]);
}

#[test]
fn every_blend_mode_gives_its_formula() {
    let (base, top) = ([0.2, 0.6, 0.9, 0.8], [0.5, 0.3, 0.1, 0.5]);
    let expected = [
        (BlendMode::Over, [0.6, 0.6, 0.55, 0.9]),
        (BlendMode::Add, [0.7, 0.9, 1.0, 1.3]),
        (BlendMode::Subtract, [-0.3, 0.3, 0.8, 0.3]),
        (BlendMode::Multiply, [0.1, 0.18, 0.09, 0.4]),
        (BlendMode::Screen, [0.6, 0.72, 0.91, 0.9]),
        (BlendMode::Overlay, [0.2, 0.44, 0.82, 0.8]),
        (BlendMode::Darken, [0.2, 0.3, 0.1, 0.5]),
        (BlendMode::Lighten, [0.5, 0.6, 0.9, 0.8]),
        (BlendMode::Difference, [0.3, 0.3, 0.8, 0.3]),
    ];
    for (mode, expected) in expected {
        let got = host(HostKernel::Blend { mode, amount: 1.0 }, &[base], [1, 1], Some(&[top]), [1, 1]);
        assert_close(&got, &[expected], 1e-6, &format!("{mode:?}"));
    }
    let half = host(HostKernel::Blend { mode: BlendMode::Add, amount: 0.5 }, &[base], [1, 1], Some(&[top]), [1, 1]);
    assert_close(&half, &[[0.45, 0.75, 0.95, 1.05]], 1e-6, "half add");
}

#[test]
fn curves_work_on_straight_colour_and_clamp_their_input() {
    let table = [[0.0, 0.0, 1.0, 0.0], [0.8, 0.5, 0.5, 0.5], [1.0, 1.0, 0.0, 1.0]];
    let pixels = [[0.25, 0.1, 0.0, 0.5], [0.0; 4], [3.0, 0.5, 0.25, 1.0]];
    let got = host(HostKernel::Curves { table: table.to_vec() }, &pixels, [3, 1], None, [3, 1]);
    assert_close(&got, &[[0.4, 0.1, 0.5, 0.5], [0.0; 4], [1.0, 0.5, 0.75, 1.0]], 1e-6, "curves");
}

#[test]
fn warp_moves_each_pixel_by_its_displacement() {
    let src = picture(SIZE);
    let area = (SIZE[0] * SIZE[1]) as usize;
    let whole = host(HostKernel::Warp { amount: [2.0, 4.0] }, &src, SIZE, Some(&vec![[0.5, -0.25, 0.0, 0.0]; area]), SIZE);
    let half = host(HostKernel::Warp { amount: [1.0, 0.0] }, &src, SIZE, Some(&vec![[0.5, 0.0, 0.0, 0.0]; area]), SIZE);
    let (mut moved, mut averaged) = (Vec::new(), Vec::new());
    for y in 0..SIZE[1] as isize {
        for x in 0..SIZE[0] as isize {
            moved.push(at(&src, SIZE, x + 1, y - 1));
            let (a, b) = (at(&src, SIZE, x, y), at(&src, SIZE, x + 1, y));
            averaged.push(std::array::from_fn(|c| 0.5 * (a[c] + b[c])));
        }
    }
    assert_close(&whole, &moved, 1e-6, "whole pixels");
    assert_close(&half, &averaged, 1e-6, "half pixels");
}

#[test]
fn matrix_mixes_channels_per_pixel() {
    let src = picture(SIZE);
    let rows = [[0.0, 0.0, 1.0, 0.0], [0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.2], [0.0, 0.0, 0.0, 1.0]];
    let got = host(HostKernel::Matrix { rows }, &src, SIZE, None, SIZE);
    let expected: Vec<Pixel> = src.iter().map(|p| [p[2], 0.5 * p[0] + 0.5 * p[1], p[0] + 0.2 * p[3], p[3]]).collect();
    assert_close(&got, &expected, 1e-6, "matrix");
}
