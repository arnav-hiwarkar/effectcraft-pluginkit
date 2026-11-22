use craft_plugin::{BlendMode, BlurShape, Encoding, Filter, GpuImage, GpuPass, GpuPlan, HostKernel, Measure, Negatives, Pixel, Taps};

use super::{Image, Lowered, Ref, Step, StepImage, blur_taps, lower, run, run_fused, steps};

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
            out.push(match filter {
                Filter::Bicubic if sum[3] <= 0.0 => [0.0; 4],
                Filter::Bicubic => sum.map(|v| v.max(0.0)),
                Filter::Bilinear => sum,
            });
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
        (BlendMode::ScreenHdr, [0.6, 0.72, 0.91, 0.9]),
    ];
    for (mode, expected) in expected {
        let got = host(HostKernel::Blend { mode, amount: 1.0 }, &[base], [1, 1], Some(&[top]), [1, 1]);
        assert_close(&got, &[expected], 1e-6, &format!("{mode:?}"));
    }
    let outside = host(HostKernel::Blend { mode: BlendMode::ScreenHdr, amount: 1.0 }, &[[-0.25, 1.5, 0.5, 1.0]], [1, 1], Some(&[[0.5; 4]]), [1, 1]);
    assert_close(&outside, &[[0.25, 1.5, 0.75, 1.0]], 1e-6, "screen past 0 and 1 adds below black and keeps above white");
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

fn exact_to_linear(x: f64) -> f64 {
    let a = x.abs();
    (if a <= 0.04045 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) }).copysign(x)
}

fn exact_to_srgb(x: f64) -> f64 {
    let a = x.abs();
    (if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 }).copysign(x)
}

#[test]
fn transfers_with_linear_negatives_continue_the_straight_segment_below_zero() {
    let pixels = [[-0.5, -0.01, 0.5, 0.7]];
    let decoded = host(HostKernel::Transfer { to: Encoding::Linear, negatives: Negatives::Linear }, &pixels, [1, 1], None, [1, 1]);
    let encoded = host(HostKernel::Transfer { to: Encoding::Srgb, negatives: Negatives::Linear }, &pixels, [1, 1], None, [1, 1]);
    assert_close(&decoded, &[[-0.5 / 12.92, -0.01 / 12.92, 0.214_041_14, 0.7]], 1e-6, "decode");
    assert_close(&encoded, &[[-6.46, -0.1292, 0.735_356_6, 0.7]], 1e-5, "encode");
    let mirrored = host(HostKernel::Transfer { to: Encoding::Linear, negatives: Negatives::Mirror }, &pixels, [1, 1], None, [1, 1]);
    assert_eq!(mirrored, host(HostKernel::ToLinear, &pixels, [1, 1], None, [1, 1]));
}

#[test]
fn srgb_transfers_follow_the_standard_curve_and_keep_alpha() {
    let values: Vec<f32> = (-400..=4000).map(|i| i as f32 / 200.0).chain([0.0, 1e-6, 1.0 / 16384.0, 15.999, 16.0, 300.0]).collect();
    let pixels: Vec<Pixel> = values.iter().map(|v| [*v, *v * 0.5, *v * 0.25, 0.7]).collect();
    let size = [pixels.len() as u32, 1];
    for (kernel, exact) in [(HostKernel::ToLinear, exact_to_linear as fn(f64) -> f64), (HostKernel::ToSrgb, exact_to_srgb)] {
        let got = host(kernel.clone(), &pixels, size, None, size);
        for (p, g) in pixels.iter().zip(&got) {
            for c in 0..3 {
                let want = exact(f64::from(p[c]));
                assert!((f64::from(g[c]) - want).abs() <= 1e-5 * want.abs().max(0.01), "{kernel:?} of {}: {} against {want}", p[c], g[c]);
            }
            assert_eq!(g[3], 0.7, "{kernel:?} keeps alpha");
        }
    }
}

#[test]
fn brightness_curve_scales_colour_by_the_curve_of_its_measure() {
    let curve = vec![0.0, 0.0, 0.5, 2.0];
    let range = 3.0;
    let lookup = |m: f32| {
        let t = m / range * 3.0;
        let i = (t as usize).min(2);
        curve[i] + (curve[i + 1] - curve[i]) * (t - i as f32)
    };
    let src = picture(SIZE);
    for measure in [Measure::MaxChannel, Measure::Luma] {
        let got = host(HostKernel::BrightnessCurve { measure, curve: curve.clone(), range }, &src, SIZE, None, SIZE);
        let expected: Vec<Pixel> = src
            .iter()
            .map(|p| {
                let m = match measure {
                    Measure::MaxChannel => p[0].max(p[1]).max(p[2]),
                    Measure::Luma => 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2],
                };
                let gain = if m > 0.0 { lookup(m) / m } else { 0.0 };
                [p[0] * gain, p[1] * gain, p[2] * gain, p[3]]
            })
            .collect();
        assert_close(&got, &expected, 1e-5, &format!("{measure:?}"));
    }
    let far =
        host(HostKernel::BrightnessCurve { measure: Measure::MaxChannel, curve, range }, &[[4.0, 2.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.4]], [2, 1], None, [2, 1]);
    assert_close(&far, &[[3.5, 1.75, 0.0, 1.0], [0.0, 0.0, 0.0, 0.4]], 1e-6, "past the range the last segment continues; black stays black");
}

#[test]
fn dither_adds_at_most_one_level_of_seeded_noise_and_leaves_black_and_white() {
    let src: Vec<Pixel> = (0..64 * 64).map(|i| [0.5, if i % 2 == 0 { 0.0 } else { 1.0 }, 0.25, 1.0]).collect();
    let size = [64, 64];
    let got = host(HostKernel::Dither { seed: 7, origin: [3, -5] }, &src, size, None, size);
    let again = host(HostKernel::Dither { seed: 7, origin: [3, -5] }, &src, size, None, size);
    let other = host(HostKernel::Dither { seed: 8, origin: [3, -5] }, &src, size, None, size);
    assert_eq!(got, again, "the same seed gives the same noise");
    assert_ne!(got, other, "another seed gives other noise");
    let moved: Vec<f32> = got.iter().zip(&src).map(|(g, s)| g[0] - s[0]).collect();
    assert!(moved.iter().all(|d| d.abs() < 1.0 / 255.0), "within one level");
    let mean = moved.iter().sum::<f32>() / moved.len() as f32;
    assert!(mean.abs() < 0.02 / 255.0 && moved.iter().filter(|d| **d != 0.0).count() > 4000, "centred noise on every pixel: mean {mean}");
    assert!(got.iter().zip(&src).all(|(g, s)| g[1] == s[1] && g[3] == s[3]), "black, white and alpha are left exact");
    let shifted = host(HostKernel::Dither { seed: 7, origin: [4, -5] }, &src, size, None, size);
    assert_eq!(shifted[1][0], got[0][0], "the pattern is anchored at the origin");
}

#[test]
fn light_alpha_raises_alpha_to_the_brightest_channel() {
    let got = host(HostKernel::LightAlpha, &[[0.3, 0.6, 0.1, 0.2], [2.0, 0.0, 0.0, 0.5], [0.1, 0.1, 0.1, 0.9], [-1.0, -2.0, -3.0, 0.0]], [4, 1], None, [4, 1]);
    assert_eq!(got, [[0.3, 0.6, 0.1, 0.6], [2.0, 0.0, 0.0, 1.0], [0.1, 0.1, 0.1, 0.9], [-1.0, -2.0, -3.0, 0.0]]);
}

fn sequential(plan: &GpuPlan, source: &[Pixel]) -> Vec<Pixel> {
    let mut images: Vec<Vec<Pixel>> = vec![Vec::new(); plan.images.len() + 1];
    let index = |image: GpuImage| match image {
        GpuImage::Output => 0,
        GpuImage::Scratch(i) => i as usize + 1,
        _ => usize::MAX,
    };
    for pass in &plan.passes {
        let read = |image: GpuImage| Image { size: SIZE, pixels: if image == GpuImage::Source { source } else { &images[index(image)] } };
        let mut out = vec![[0.0; 4]; source.len()];
        run(pass.host.as_ref().expect("a host kernel"), read(pass.src), pass.aux.map(read), SIZE, &mut out);
        images[index(pass.out)] = out;
    }
    std::mem::take(&mut images[0])
}

fn fused(plan: &GpuPlan, source: &[Pixel]) -> Vec<Pixel> {
    let mut images: Vec<Vec<Pixel>> = vec![vec![[0.0; 4]; source.len()]; plan.images.len() + 1];
    let index = |image: GpuImage| match image {
        GpuImage::Output => 0,
        GpuImage::Scratch(i) => i as usize + 1,
        _ => usize::MAX,
    };
    for step in steps(plan, SIZE) {
        let Step::Fused(group) = step else { panic!("only pointwise kernels in this plan") };
        let inputs: Vec<Image> =
            group.inputs.iter().map(|image| Image { size: SIZE, pixels: if *image == GpuImage::Source { source } else { &images[index(*image)] } }).collect();
        let mut out = vec![[0.0; 4]; source.len()];
        run_fused(&group, &inputs, SIZE[0] as usize, &mut out);
        images[index(group.out)] = out;
    }
    std::mem::take(&mut images[0])
}

#[test]
fn fused_pointwise_passes_give_exactly_what_they_give_one_by_one() {
    let source = picture(SIZE);
    let mut plan = GpuPlan::new();
    let [a, b, c] = [plan.scratch(SIZE), plan.scratch(SIZE), plan.scratch(SIZE)];
    let rows = [[0.5, 0.2, 0.0, 0.0], [0.0, 1.0, 0.3, 0.0], [0.1, 0.0, 0.7, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let plan = plan
        .pass(GpuPass::to_linear(GpuImage::Source, a))
        .pass(GpuPass::matrix(a, b, rows))
        .pass(GpuPass::brightness_curve(a, a, Measure::Luma, vec![0.0, 0.2, 1.5], 1.0))
        .pass(GpuPass::blend_images(b, a, c, BlendMode::Screen, 0.8))
        .pass(GpuPass::blend_images(c, GpuImage::Source, c, BlendMode::Lighten, 1.0))
        .pass(GpuPass::to_srgb(c, c))
        .pass(GpuPass::dither(c, c, 9, [2, 2]))
        .pass(GpuPass::light_alpha(c, b))
        .pass(GpuPass::curves(b, GpuImage::Output, &[[0.0; 4], [1.0, 0.5, 1.0, 1.0]]));
    assert_eq!(plan.validate(SIZE), Ok(()), "pointwise host kernels may write the image they read");
    let groups: Vec<usize> = steps(&plan, SIZE).iter().map(|step| if let Step::Fused(group) = step { group.ops.len() } else { 0 }).collect();
    assert_eq!(groups, [9], "the whole chain is one loop");
    assert_eq!(fused(&plan, &source), sequential(&plan, &source));
}

#[test]
fn a_fused_loop_reads_an_overwritten_image_and_one_read_twice_as_written_at_that_point() {
    let source = vec![[0.25, 0.5, 0.75, 1.0]; (SIZE[0] * SIZE[1]) as usize];
    let mut plan = GpuPlan::new();
    let [a, b] = [plan.scratch(SIZE), plan.scratch(SIZE)];
    let double = [[2.0, 0.0, 0.0, 0.0], [0.0, 2.0, 0.0, 0.0], [0.0, 0.0, 2.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let plan = plan
        .pass(GpuPass::matrix(GpuImage::Source, a, double))
        .pass(GpuPass::blend_images(a, GpuImage::Source, b, BlendMode::Add, 1.0))
        .pass(GpuPass::matrix(a, a, [[0.5, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.5, 0.0], [0.0, 0.0, 0.0, 0.5]]))
        .pass(GpuPass::blend_images(b, a, GpuImage::Output, BlendMode::Subtract, 1.0));
    assert_eq!(steps(&plan, SIZE).len(), 1, "one loop");
    let out = fused(&plan, &source);
    assert!(out.iter().all(|p| *p == [0.5, 1.0, 1.5, 1.5]), "{:?}", out[0]);
}

#[test]
fn a_fused_loop_ends_where_a_value_is_read_later_or_a_third_image_is_needed() {
    let mut plan = GpuPlan::new();
    let [a, b, c, d] = [plan.scratch(SIZE), plan.scratch(SIZE), plan.scratch(SIZE), plan.scratch(SIZE)];
    let half = plan.scratch([19, 12]);
    let plan = plan
        .pass(GpuPass::to_linear(GpuImage::Source, a))
        .pass(GpuPass::to_srgb(a, b))
        .pass(GpuPass::resample(a, half, Filter::Bilinear))
        .pass(GpuPass::blend_images(b, GpuImage::Source, c, BlendMode::Add, 1.0))
        .pass(GpuPass::blend_images(c, a, d, BlendMode::Add, 1.0))
        .pass(GpuPass::resample(half, b, Filter::Bilinear))
        .pass(GpuPass::blend_images(d, b, GpuImage::Output, BlendMode::Add, 1.0));
    let shape: Vec<(Vec<usize>, Vec<Ref>)> = steps(&plan, SIZE)
        .into_iter()
        .map(|step| match step {
            Step::Single(index) => (vec![index], Vec::new()),
            Step::Fused(group) => (group.passes.collect(), group.ops.iter().map(|op| op.src).collect()),
        })
        .collect();
    assert_eq!(
        shape,
        [
            (vec![0], vec![Ref::Input(0)]),
            (vec![1], vec![Ref::Input(0)]),
            (vec![2], vec![]),
            (vec![3], vec![Ref::Input(0)]),
            (vec![4], vec![Ref::Input(0)]),
            (vec![5], vec![]),
            (vec![6], vec![Ref::Input(0)]),
        ]
    );
}

#[test]
fn on_the_gpu_a_resample_read_only_by_the_next_fused_loop_is_sampled_inside_it() {
    let mut plan = GpuPlan::new();
    let (half, up) = (plan.scratch([19, 12]), plan.scratch(SIZE));
    let plan = plan
        .pass(GpuPass::resample(GpuImage::Source, half, Filter::Bilinear))
        .pass(GpuPass::resample(half, up, Filter::Bicubic))
        .pass(GpuPass::blend_images(GpuImage::Source, up, GpuImage::Output, BlendMode::Screen, 1.0));
    let (lowered, temps) = lower(&plan, SIZE);
    let steps: Vec<(&str, StepImage, Option<StepImage>, Vec<u32>, bool)> = lowered
        .iter()
        .map(|step| match step {
            Lowered::Host(step) => {
                (step.entry, step.src, step.aux, step.u.clone(), step.wgsl.contains("craft_resampled(aux, p, vec2<f32>(pass_f(0u), pass_f(1u)), true)"))
            }
            Lowered::Plugin(pass) => panic!("no plugin pass in this plan: {pass:?}"),
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("craft_resample", StepImage::Plan(GpuImage::Source), None, vec![0], false),
            ("craft_pointwise", StepImage::Plan(GpuImage::Source), Some(StepImage::Plan(half)), vec![], true),
        ]
    );
    assert!(temps.is_empty());
}

#[test]
fn a_gaussian_too_small_to_matter_keeps_the_image() {
    assert_eq!(blur_taps(BlurShape::Gaussian, 1e-30), vec![0.0, 1.0, 0.0]);
    let src = [[0.5, 0.25, 0.125, 1.0]];
    let got = host(HostKernel::Blur { shape: BlurShape::Gaussian, radius: [1e-30, 0.0] }, &src, [1, 1], None, [1, 1]);
    assert_eq!(got, src);
}

#[test]
fn a_warp_that_lands_outside_the_image_or_overflows_reads_clear() {
    let src = [[0.5, 0.25, 0.125, 1.0]];
    let warp = |displacement: Pixel| host(HostKernel::Warp { amount: [2.0, 0.0] }, &src, [1, 1], Some(&[displacement]), [1, 1])[0];
    assert_eq!(warp([f32::MAX, 0.0, 0.0, 0.0]), [0.0; 4]);
    assert_eq!(warp([-f32::MAX, 0.0, 0.0, 0.0]), [0.0; 4]);
    assert_eq!(warp([f32::NAN, 0.0, 0.0, 0.0]), [0.0; 4]);
    assert_eq!(warp([-0.5, 0.0, 0.0, 0.0]), [0.0; 4]);
    assert_eq!(warp([0.25, 0.0, 0.0, 0.0]), [0.25, 0.125, 0.0625, 0.5]);
}

#[test]
fn bicubic_resampling_clears_a_pixel_whose_alpha_filters_to_nothing() {
    let src = [[0.0; 4], [1.0 / 16.0, 0.0, 0.0, 1.0 / 16.0], [0.0, 0.0, 0.0, 1.0]];
    let got = host(HostKernel::Resample { filter: Filter::Bicubic, scale: Some([0.5, 1.0]) }, &src, [3, 1], None, [6, 1]);
    assert_eq!(got[1], [0.0; 4]);
    assert!(got.iter().all(|p| p[3] > 0.0 || *p == [0.0; 4]), "{got:?}");
}

#[test]
fn a_blend_at_full_amount_is_the_blend_and_at_none_the_base() {
    let base = [16_777_216.0, 0.5, 0.25, 1.0];
    let blend = |mode, amount, top: Pixel| host(HostKernel::Blend { mode, amount }, &[base], [1, 1], Some(&[top]), [1, 1])[0];
    assert_eq!(blend(BlendMode::Over, 1.0, [0.25, 0.0, 0.0, 1.0]), [0.25, 0.0, 0.0, 1.0]);
    assert_eq!(blend(BlendMode::Over, 0.0, [f32::INFINITY, 0.0, 0.0, 1.0]), base);
    assert_eq!(blend(BlendMode::Add, 0.0, [f32::INFINITY, 0.0, 0.0, 1.0]), base);
}

#[test]
fn on_the_gpu_blurs_of_one_size_share_one_temporary() {
    let mut plan = GpuPlan::new();
    let (soft, small, smaller) = (plan.scratch(SIZE), plan.scratch([5, 4]), plan.scratch([5, 4]));
    let mut plan = plan.pass(GpuPass::blur(GpuImage::Source, soft, [2.0; 2]));
    for k in 0..62 {
        let (src, out) = if k % 2 == 0 { (soft, GpuImage::Output) } else { (GpuImage::Output, soft) };
        plan = plan.pass(GpuPass::blur(src, out, [2.0; 2]));
    }
    let plan = plan.pass(GpuPass::resample(soft, small, Filter::Bilinear)).pass(GpuPass::blur(small, smaller, [1.0; 2]));
    assert_eq!(plan.validate(SIZE), Ok(()));
    let (lowered, temps) = lower(&plan, SIZE);
    assert_eq!(lowered.len(), 2 * 64 + 1);
    assert_eq!(temps, vec![SIZE, [5, 4]]);
}

#[test]
fn on_the_gpu_a_fused_loop_is_straight_line_code_whose_numbers_stay_in_its_data() {
    let plan = |amount: f32, mode: BlendMode, curve: Vec<f32>| {
        let mut plan = GpuPlan::new();
        let soft = plan.scratch(SIZE);
        plan.pass(GpuPass::brightness_curve(GpuImage::Source, soft, Measure::Luma, curve, 2.0))
            .pass(GpuPass::blend_images(soft, GpuImage::Source, soft, mode, amount))
            .pass(GpuPass::light_alpha(soft, GpuImage::Output))
    };
    let step = |plan: &GpuPlan| match lower(plan, SIZE).0.as_slice() {
        [Lowered::Host(step)] => step.clone(),
        other => panic!("one fused step: {other:?}"),
    };
    let screen = step(&plan(0.5, BlendMode::Screen, vec![0.0, 0.25, 2.0]));
    assert_eq!(
        screen.wgsl,
        "@compute @workgroup_size(16, 16)
fn craft_pointwise(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!inside_out(id.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let i0 = texel(src, p);
    let v0 = craft_brightness_curve(0u, i0);
    let v1 = craft_amount(v0, v0 + i0 - v0 * i0, craft_data(6u));
    let v2 = craft_light_alpha(v1);
    store(id.xy, v2);
}
"
    );
    assert_eq!(screen.data, [1.0, 3.0, 2.0, 0.0, 0.25, 2.0, 0.5]);
    let other_numbers = step(&plan(0.75, BlendMode::Screen, vec![0.0, 0.5, 1.0]));
    assert_eq!((other_numbers.wgsl == screen.wgsl, other_numbers.data[6]), (true, 0.75));
    assert!(step(&plan(0.5, BlendMode::Multiply, vec![0.0, 0.25, 2.0])).wgsl.contains("craft_amount(v0, v0 * i0, craft_data(6u))"));
}
