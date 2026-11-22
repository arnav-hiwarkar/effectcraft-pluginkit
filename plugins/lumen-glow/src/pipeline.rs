mod tests {
    use craft_plugin::Value;

    use craft_plugin::prelude::*;

    use crate::LumenGlow;
    use crate::pixel::CLEAR;
    use crate::settings::Settings;
    use crate::settings::tests::cx_with;

    fn render(cx: &Cx, pixels: &mut [Pixel], width: usize) {
        let frame = [width as u32, (pixels.len() / width) as u32];
        let plan = crate::gpu::plan(&Settings::from_cx(cx), frame, 0).expect("a plan");
        let area = |size: [u32; 2]| (size[0] * size[1]) as usize;
        let sizes: Vec<[u32; 2]> = std::iter::once(frame).chain(plan.images.clone()).collect();
        let mut written: Vec<Vec<Pixel>> = sizes.iter().map(|s| vec![CLEAR; area(*s)]).collect();
        let slot = |image: GpuImage| match image {
            GpuImage::Source => None,
            GpuImage::Output => Some(0),
            GpuImage::Scratch(index) => Some(index as usize + 1),
            GpuImage::Texture(_) | GpuImage::Layer(_) => {
                panic!("Lumen Glow reads no textures or layers")
            }
        };
        for step in &plan.passes {
            let target = slot(step.out).expect("no pass writes the source");
            let mut out = std::mem::take(&mut written[target]);
            let rows = |image: GpuImage| {
                let [width, height] = slot(image).map_or(frame, |i| sizes[i]).map(|s| s as usize);
                let pixels = slot(image).map_or(&pixels[..], |i| &written[i][..]);
                Rows { width, height, first_row: 0, pixels }
            };
            let [width, height] = sizes[target].map(|s| s as usize);
            let pass = Pass { kernel: &step.kernel, f: &step.f, u: &step.u, src: rows(step.src), aux: step.aux.map(rows), out_size: [width, height] };
            let mut band = Frame { width, height, origin: [0, 0], pixels: &mut out };
            LumenGlow.pass(cx, &pass, &mut band).expect("the pass runs");
            written[target] = out;
        }
        pixels.copy_from_slice(&written[0]);
    }

    const SIZE: usize = 257;
    const CENTRE: usize = SIZE / 2;

    fn glow_of_point(overrides: &[(&str, Value)]) -> Vec<Pixel> {
        glow_of_point_with_brightness(1.0, overrides)
    }

    fn glow_of_point_with_brightness(brightness: f32, overrides: &[(&str, Value)]) -> Vec<Pixel> {
        let mut base = vec![("Output/view", Value::Number(1.0)), ("Output/dither", Value::Bool(false))];
        base.extend(overrides.iter().cloned());
        let mut pixels = vec![CLEAR; SIZE * SIZE];
        pixels[CENTRE * SIZE + CENTRE] = [brightness, brightness, brightness, 1.0];
        render(&cx_with(&base, 1.0), &mut pixels, SIZE);
        pixels
    }

    fn at(pixels: &[Pixel], dx: isize, dy: isize) -> f32 {
        pixels[(CENTRE as isize + dy) as usize * SIZE + (CENTRE as isize + dx) as usize][1]
    }

    #[test]
    fn a_single_bright_pixel_glows_radially_symmetric() {
        let glow = glow_of_point(&[("Glow/radius", Value::Number(120.0))]);
        for d in [8, 20, 50, 90] {
            let axis = [at(&glow, d, 0), at(&glow, -d, 0), at(&glow, 0, d), at(&glow, 0, -d)];
            let diagonal_d = (d as f32 / std::f32::consts::SQRT_2).round() as isize;
            let diagonal = at(&glow, diagonal_d, diagonal_d);
            let (lo, hi) = axis.iter().fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            assert!(lo > 0.0 && hi / lo < 1.05, "distance {d}: {axis:?}");
            assert!((diagonal / hi - 1.0).abs() < 0.1, "distance {d}: diagonal {diagonal} axis {hi}");
        }
    }

    #[test]
    fn glow_falls_off_monotonically_from_the_core() {
        let glow = glow_of_point(&[("Glow/radius", Value::Number(120.0)), ("Output/gamma", Value::Bool(false))]);
        let profile: Vec<f32> = (0..120).map(|d| at(&glow, d, 0)).collect();
        assert!(profile.windows(2).all(|w| w[1] <= w[0]), "{profile:?}");
    }

    #[test]
    fn hdr_sources_glow_in_proportion_to_their_energy() {
        let linear = [("Glow/radius", Value::Number(60.0)), ("Output/gamma", Value::Bool(false))];
        let dim = glow_of_point_with_brightness(1.0, &linear);
        let hot = glow_of_point_with_brightness(16.0, &linear);
        for d in [4, 15, 40] {
            let ratio = at(&hot, d, 0) / at(&dim, d, 0);
            assert!((ratio - 16.0).abs() < 1e-3, "distance {d}: ratio {ratio}");
        }
    }

    fn disc(size: usize, radius: f32) -> Vec<Pixel> {
        let c = size as f32 / 2.0;
        (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 + 0.5 - c, (i / size) as f32 + 0.5 - c);
                if x * x + y * y < radius * radius { [1.0; 4] } else { CLEAR }
            })
            .collect()
    }

    #[test]
    fn half_resolution_matches_the_full_resolution_glow() {
        let overrides = [
            ("Glow/radius", Value::Number(80.0)),
            ("Output/view", Value::Number(1.0)),
            ("Output/gamma", Value::Bool(false)),
            ("Output/dither", Value::Bool(false)),
        ];
        let mut full = disc(256, 24.0);
        render(&cx_with(&overrides, 1.0), &mut full, 256);
        let mut half = disc(128, 12.0);
        render(&cx_with(&overrides, 0.5), &mut half, 128);
        for (x, y) in [(64, 30), (64, 10), (100, 100), (64, 64), (20, 64)] {
            let averaged: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| full[(2 * y + dy) * 256 + 2 * x + dx][1]).sum::<f32>() / 4.0;
            let preview = half[y * 128 + x][1];
            assert!((preview / averaged - 1.0).abs() < 0.04, "({x}, {y}): half {preview} full {averaged}");
        }
    }
}
