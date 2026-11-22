use craft_plugin::prelude::*;

use crate::pixel::Mix;
use crate::plan::{Level, Plan};
use crate::settings::{Blend, Settings, View};

const MAX_FLOATS: usize = 48;
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
    let glow = if sizes.is_empty() { None } else { Some(glow(&mut gpu, settings, &plan, &sizes)?) };
    gpu.passes.push(composite(settings, &plan.core, glow, padding));
    Some(gpu)
}

fn glow(gpu: &mut GpuPlan, settings: &Settings, plan: &Plan, sizes: &[[u32; 2]]) -> Option<GpuImage> {
    let pyramid: Vec<GpuImage> = sizes.iter().map(|&size| gpu.scratch(size)).collect();
    let summed: Vec<GpuImage> = sizes.iter().map(|&size| gpu.scratch(size)).collect();
    gpu.passes.push(
        GpuPass::new("lumen_emit", GpuImage::Source, *pyramid.first()?)
            .floats(&settings.knee.shape())
            .uints(&[u32::from(settings.transfer.is_srgb())])
            .reach(HALVING_REACH),
    );
    for ((pair, level), &[w, h]) in pyramid.windows(2).zip(plan.levels.iter().skip(1)).zip(sizes.iter().skip(1)) {
        let [hx, hy] = level.halve.map(u32::from);
        gpu.passes.push(GpuPass::new("lumen_downsample", pair[0], pair[1]).uints(&[hx, hy, w, h]).reach(HALVING_REACH));
    }
    let mut coarser: Option<(GpuImage, [bool; 2])> = None;
    for ((level, &src), &out) in plan.levels.iter().zip(&pyramid).zip(&summed).rev() {
        gpu.passes.push(level_pass(level, src, out, coarser)?);
        coarser = Some((out, level.halve));
    }
    summed.first().copied()
}

pub fn level_pass(level: &Level, src: GpuImage, out: GpuImage, coarser: Option<(GpuImage, [bool; 2])>) -> Option<GpuPass> {
    let [rows, columns] = level.kernels().map(|k| k[k.len() / 2..].to_vec());
    let floats: Vec<f32> = mix_floats(&level.mix).into_iter().chain(rows.iter().copied()).chain(columns.iter().copied()).collect();
    if floats.len() > MAX_FLOATS {
        return None;
    }
    let reach = [rows.len(), columns.len()].map(|n| n.saturating_sub(1) as u32);
    let halve = coarser.map_or([0, 0], |(_, halve)| halve.map(u32::from));
    let pass = GpuPass::new("lumen_level", src, out)
        .floats(&floats)
        .uints(&[reach[0], reach[1], u32::from(coarser.is_some()), halve[0], halve[1]])
        .reach(reach[1].max(UPSAMPLE_REACH));
    Some(match coarser {
        Some((image, _)) => pass.aux(image),
        None => pass,
    })
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
    let pass = GpuPass::new("lumen_composite", GpuImage::Source, GpuImage::Output).floats(&floats).uints(&uints).reach(UPSAMPLE_REACH);
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
        assert!(plan.passes.iter().all(|p| p.reach.is_some()), "{case}");
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
    fn the_plan_has_a_level_pass_per_cpu_level() {
        let settings = settings_with(&[("Glow/radius", Value::Number(400.0))]);
        let plan = plan(&settings, [640, 360], settings.frame_padding([640, 360])).expect("a plan");
        let levels = Plan::new(&settings).levels.len();
        let count = |kernel: &str| plan.passes.iter().filter(|p| p.kernel == kernel).count();
        assert_eq!([count("lumen_emit"), count("lumen_downsample"), count("lumen_level"), count("lumen_composite")], [1, levels - 1, levels, 1]);
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
