use craft_plugin::prelude::*;

use crate::pixel::Mix;
use crate::plan::Plan;
use crate::settings::{Blend, Settings, View};

const MIX_FLOATS: usize = 12;
const HALVING_REACH: u32 = 3;
const UPSAMPLE_REACH: u32 = 2;

pub fn plan(settings: &Settings, size: [u32; 2], padding: u32) -> Option<GpuPlan> {
    let frame = size.map(|s| s.saturating_add(padding.saturating_mul(2)));
    if frame.contains(&0) {
        return None;
    }
    let plan = Plan::new(settings);
    let sizes: Vec<[u32; 2]> = plan.sizes(frame.map(|s| s as usize)).into_iter().map(|s| s.map(|v| v as u32)).collect();
    let mut gpu = GpuPlan::new();
    let glow = glow(&mut gpu, settings, &plan, &sizes);
    gpu.passes.push(composite(settings, &plan.core, glow, padding));
    Some(gpu)
}

fn glow(gpu: &mut GpuPlan, settings: &Settings, plan: &Plan, sizes: &[[u32; 2]]) -> Option<GpuImage> {
    let images: Vec<([GpuImage; 2], [u32; 2])> = sizes.iter().map(|&size| ([gpu.scratch(size), gpu.scratch(size)], size)).collect();
    gpu.passes.push(
        GpuPass::new("lumen_emit", GpuImage::Source, images.first()?.0[0])
            .floats(&settings.knee.shape())
            .uints(&[u32::from(settings.transfer.is_srgb())])
            .reach(HALVING_REACH),
    );
    for (pair, level) in images.windows(2).zip(plan.levels.iter().skip(1)) {
        gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(level.halve.map(factor)) }, pair[0].0[0], pair[1].0[0]));
    }
    let mut coarser: Option<(GpuImage, [bool; 2])> = None;
    for (level, &([pyramid, scratch], size)) in plan.levels.iter().zip(&images).rev() {
        let [x, y] = level.kernels();
        gpu.passes.push(GpuPass::convolve(pyramid, scratch, Taps::Separable { x, y }));
        gpu.passes.push(GpuPass::matrix(scratch, pyramid, mix_rows(&level.mix)));
        let summed = match coarser {
            None => pyramid,
            Some((image, halve)) => {
                let summed = gpu.scratch(size);
                let scale = halve.map(|halved| 1.0 / factor(halved));
                gpu.passes.push(GpuPass::host(HostKernel::Resample { filter: Filter::Bilinear, scale: Some(scale) }, image, scratch));
                gpu.passes.push(GpuPass::blend_images(pyramid, scratch, summed, BlendMode::Add, 1.0));
                summed
            }
        };
        coarser = Some((summed, level.halve));
    }
    coarser.map(|(image, _)| image)
}

fn factor(halved: bool) -> f32 {
    if halved { 2.0 } else { 1.0 }
}

fn mix_rows(mix: &Mix) -> [[f32; 4]; 4] {
    let columns = mix.columns();
    std::array::from_fn(|row| [columns[0][row], columns[1][row], columns[2][row], 0.0])
}

fn composite(settings: &Settings, core: &Mix, glow: Option<GpuImage>, padding: u32) -> GpuPass {
    let [threshold, width] = settings.knee.shape();
    let floats: Vec<f32> = mix_floats(core).into_iter().chain([threshold, width, settings.glow_opacity, settings.source_opacity]).collect();
    let view = match settings.view {
        View::Final => 0,
        View::GlowOnly => 1,
        View::GlowSource => 2,
    };
    let blend = match settings.blend {
        Blend::Add => 0,
        Blend::Screen => 1,
    };
    let [dither, seed, x, y] = match settings.dither {
        Some(d) => {
            let origin = d.origin.map(|o| o.wrapping_add(i64::from(padding)) as u32);
            [1, d.seed, origin[0], origin[1]]
        }
        None => [0; 4],
    };
    let uints = [view, blend, u32::from(settings.transfer.is_srgb()), dither, seed, x, y, u32::from(glow.is_some())];
    let pass = GpuPass::new("lumen_composite", GpuImage::Source, GpuImage::Output).floats(&floats).uints(&uints).reach(0).aux_reach(UPSAMPLE_REACH);
    match glow {
        Some(image) => pass.aux(image),
        None => pass,
    }
}

fn mix_floats(mix: &Mix) -> [f32; MIX_FLOATS] {
    let columns = mix.columns();
    std::array::from_fn(|i| columns[i / 4][i % 4])
}

#[cfg(test)]
mod tests {
    use craft_plugin::Value;

    use super::*;
    use crate::settings::tests::settings_with;

    fn assert_runs_on_the_host(plan: &GpuPlan, frame: [u32; 2], case: &str) {
        assert_eq!(plan.check(frame), Ok(()), "{case}");
        assert_eq!(plan.passes.last().map(|p| p.out), Some(GpuImage::Output), "{case}");
        assert!(plan.passes.iter().all(|p| p.reach.is_some() || p.host.is_some()), "{case}");
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
                        let plan = plan(&settings, size, padding).unwrap_or_else(|| panic!("{case}: no GPU plan"));
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
        let kernels = ["lumen_emit", "resample", "convolve", "matrix", "blend", "lumen_composite"].map(count);
        assert_eq!(kernels, [1, 2 * (levels - 1), levels, levels, levels - 1, 1]);
    }

    #[test]
    fn glow_source_view_is_one_pass() {
        let settings = settings_with(&[("Output/view", Value::Number(2.0))]);
        let plan = plan(&settings, [64, 64], 0).expect("a plan");
        assert_runs_on_the_host(&plan, [64, 64], "glow source");
        assert_eq!(plan.passes.len(), 1);
    }

    #[test]
    fn an_empty_frame_renders_on_the_cpu() {
        assert!(plan(&settings_with(&[]), [0, 0], 0).is_none());
    }
}
