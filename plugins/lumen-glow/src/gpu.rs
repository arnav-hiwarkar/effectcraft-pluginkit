use craft_plugin::prelude::*;

use crate::pixel::Mix;
use crate::plan::Plan;
use crate::settings::{Blend, Settings, View};

const HALF: [f32; 2] = [2.0, 2.0];
const DOUBLE: [f32; 2] = [0.5, 0.5];

pub fn plan(settings: &Settings, size: [u32; 2], padding: u32) -> Option<GpuPlan> {
    let frame = size.map(|s| s.saturating_add(padding.saturating_mul(2)));
    if frame.contains(&0) {
        return None;
    }
    let plan = Plan::new(settings);
    let sizes: Vec<[u32; 2]> = plan.sizes(frame.map(|s| s as usize)).into_iter().map(|s| s.map(|v| v as u32)).collect();
    let mut gpu = GpuPlan::new();
    let [work, base] = [gpu.scratch(frame), gpu.scratch(frame)];
    let glow = glow(&mut gpu, settings, &plan, &sizes, work, frame);
    composite(&mut gpu, settings, &plan.core, glow, work, base, padding);
    Some(gpu)
}

fn linear(gpu: &mut GpuPlan, settings: &Settings, work: GpuImage) -> GpuImage {
    if !settings.gamma {
        return GpuImage::Source;
    }
    gpu.passes.push(GpuPass::to_linear(GpuImage::Source, work));
    work
}

fn emission(settings: &Settings, src: GpuImage, work: GpuImage) -> GpuPass {
    let (curve, range) = settings.knee.curve();
    GpuPass::brightness_curve(src, work, Measure::MaxChannel, curve, range)
}

fn glow(gpu: &mut GpuPlan, settings: &Settings, plan: &Plan, sizes: &[[u32; 2]], work: GpuImage, frame: [u32; 2]) -> Option<GpuImage> {
    let images: Vec<([GpuImage; 2], [u32; 2])> = sizes.iter().map(|&size| ([gpu.scratch(size), gpu.scratch(size)], size)).collect();
    let finest = images.first()?.0[0];
    let source = linear(gpu, settings, work);
    gpu.passes.push(emission(settings, source, work));
    gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(HALF) }, work, finest));
    for (pair, level) in images.windows(2).zip(plan.levels.iter().skip(1)) {
        gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(level.halve.map(factor)) }, pair[0].0[0], pair[1].0[0]));
    }
    let mut coarser: Option<(GpuImage, [bool; 2])> = None;
    for (level, &([pyramid, scratch], _)) in plan.levels.iter().zip(&images).rev() {
        let [x, y] = level.kernels();
        gpu.passes.push(GpuPass::convolve(pyramid, scratch, Taps::Separable { x, y }));
        if let Some((image, halve)) = coarser {
            let scale = halve.map(|halved| 1.0 / factor(halved));
            gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(scale) }, image, pyramid));
        }
        gpu.passes.push(GpuPass::matrix(scratch, scratch, mix_rows(&level.mix, settings.glow_opacity)));
        if coarser.is_some() {
            gpu.passes.push(GpuPass::blend_images(scratch, pyramid, scratch, BlendMode::Add, 1.0));
        }
        coarser = Some((scratch, level.halve));
    }
    let (summed, _) = coarser?;
    let spread = gpu.scratch(frame);
    gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(DOUBLE) }, summed, spread));
    Some(spread)
}

fn factor(halved: bool) -> f32 {
    if halved { 2.0 } else { 1.0 }
}

fn mix_rows(mix: &Mix, gain: f32) -> [[f32; 4]; 4] {
    let columns = mix.columns();
    std::array::from_fn(|row| [columns[0][row] * gain, columns[1][row] * gain, columns[2][row] * gain, 0.0])
}

fn diagonal(colour: f32, alpha: f32) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            if row != column {
                0.0
            } else if row < 3 {
                colour
            } else {
                alpha
            }
        })
    })
}

fn composite(gpu: &mut GpuPlan, settings: &Settings, core: &Mix, glow: Option<GpuImage>, work: GpuImage, base: GpuImage, padding: u32) {
    let source = linear(gpu, settings, work);
    let mut pass = |pass: GpuPass| gpu.passes.push(pass);
    if settings.view == View::Final {
        pass(GpuPass::matrix(source, base, diagonal(settings.source_opacity, settings.source_opacity)));
    }
    pass(emission(settings, source, work));
    match settings.view {
        View::GlowSource => pass(GpuPass::matrix(work, work, diagonal(1.0, 0.0))),
        View::GlowOnly | View::Final => {
            pass(GpuPass::matrix(work, work, mix_rows(core, settings.glow_opacity)));
            if let Some(spread) = glow {
                pass(GpuPass::blend_images(work, spread, work, BlendMode::Add, 1.0));
            }
        }
    }
    if settings.view == View::Final {
        match settings.blend {
            Blend::Add => pass(GpuPass::blend_images(base, work, work, BlendMode::Add, 1.0)),
            Blend::Screen => {
                pass(GpuPass::blend_images(base, work, work, BlendMode::Screen, 1.0));
                pass(GpuPass::blend_images(work, base, work, BlendMode::Lighten, 1.0));
            }
        }
    }
    if settings.gamma {
        pass(GpuPass::to_srgb(work, work));
    }
    if let Some(d) = settings.dither {
        pass(GpuPass::dither(work, work, d.seed, d.origin.map(|o| o.wrapping_add(i64::from(padding)))));
    }
    pass(GpuPass::light_alpha(work, GpuImage::Output));
}

#[cfg(test)]
mod tests {
    use craft_plugin::Value;

    use super::*;
    use crate::settings::tests::settings_with;

    fn assert_runs_on_the_host(plan: &GpuPlan, frame: [u32; 2], case: &str) {
        assert_eq!(plan.check(frame), Ok(()), "{case}");
        assert_eq!(plan.passes.last().map(|p| p.out), Some(GpuImage::Output), "{case}");
        assert!(plan.passes.iter().all(|p| p.host.is_some()), "{case}");
    }

    #[test]
    fn every_setting_fits_the_host_limits() {
        for radius in [1.0, 3.0, 10.0, 120.0, 600.0, 3000.0] {
            for aspect in [-1.0, -0.6, 0.0, 0.3, 1.0] {
                for aberration in [0.0, 100.0] {
                    for (size, scale) in [([1, 1], 1.0), ([7, 3000], 1.0), ([1920, 1080], 1.0), ([1920, 1080], 0.37), ([3840, 2160], 1.0)] {
                        let mut settings = settings_with(&[
                            ("Glow/radius", Value::Number(radius)),
                            ("Glow/aspect", Value::Number(aspect)),
                            ("Aberration/amount", Value::Number(aberration)),
                        ]);
                        settings.scale = scale;
                        let padding = settings.frame_padding(size);
                        let frame = size.map(|s| s + 2 * padding);
                        let case = format!("radius {radius} aspect {aspect} aberration {aberration} size {size:?} scale {scale}");
                        let plan = plan(&settings, size, padding).unwrap_or_else(|| panic!("{case}: no plan"));
                        assert_runs_on_the_host(&plan, frame, &case);
                    }
                }
            }
        }
    }

    #[test]
    fn each_level_is_blurred_mixed_and_summed_by_host_kernels() {
        let settings = settings_with(&[("Glow/radius", Value::Number(400.0))]);
        let plan = plan(&settings, [640, 360], settings.frame_padding([640, 360])).expect("a plan");
        let levels = Plan::new(&settings).levels.len();
        let count = |kernel: &str| plan.passes.iter().filter(|p| p.kernel == kernel).count();
        let kernels = ["resample", "convolve", "matrix", "blend", "brightness_curve", "to_linear", "to_srgb", "dither", "light_alpha"].map(count);
        assert_eq!(kernels, [2 * levels, levels, levels + 2, levels + 1, 2, 2, 1, 1, 1]);
        assert!(plan.images.len() <= 2 * levels + 3, "{} scratch images for {levels} levels", plan.images.len());
    }

    #[test]
    fn every_view_and_blend_ends_in_the_output() {
        for (view, blend) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (2.0, 0.0)] {
            let settings = settings_with(&[("Output/view", Value::Number(view)), ("Output/blend", Value::Number(blend))]);
            let plan = plan(&settings, [64, 64], 0).expect("a plan");
            assert_runs_on_the_host(&plan, [64, 64], &format!("view {view} blend {blend}"));
        }
    }

    #[test]
    fn an_empty_frame_renders_on_the_cpu() {
        assert!(plan(&settings_with(&[]), [0, 0], 0).is_none());
    }
}
