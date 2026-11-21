use craft_plugin::prelude::*;

use crate::pixel::Mix;
use crate::plan::{Level, Plan};
use crate::settings::{Blend, Settings, View};

const MAX_FLOATS: usize = 48;
const MIX_FLOATS: usize = 12;

pub fn plan(settings: &Settings, size: [u32; 2], padding: u32) -> Option<GpuPlan> {
    let frame = size.map(|s| s.saturating_add(padding.saturating_mul(2)));
    if frame.contains(&0) {
        return None;
    }
    let plan = Plan::new(settings);
    let sizes: Vec<[u32; 2]> = plan
        .sizes(frame.map(|s| s as usize))
        .into_iter()
        .map(|s| s.map(|v| v as u32))
        .collect();
    let mut gpu = GpuPlan::new();
    let glow = match sizes.last() {
        Some(&coarsest) => Some(glow(&mut gpu, settings, &plan, &sizes, coarsest)?),
        None => None,
    };
    gpu.passes
        .push(composite(settings, &plan.core, glow, padding));
    Some(gpu)
}

fn glow(
    gpu: &mut GpuPlan,
    settings: &Settings,
    plan: &Plan,
    sizes: &[[u32; 2]],
    coarsest: [u32; 2],
) -> Option<GpuImage> {
    let slot_sizes = sizes.first().into_iter().chain(sizes).take(sizes.len());
    let pyramid: Vec<GpuImage> = slot_sizes.map(|&size| gpu.scratch(size)).collect();
    let last = gpu.scratch(coarsest);
    gpu.passes.push(
        GpuPass::new("lumen_emit", GpuImage::Source, *pyramid.first()?)
            .floats(&settings.knee.shape())
            .uints(&[u32::from(settings.transfer.is_srgb())]),
    );
    for ((pair, level), &[w, h]) in pyramid
        .windows(2)
        .zip(plan.levels.iter().skip(1))
        .zip(sizes.iter().skip(1))
    {
        let [hx, hy] = level.halve.map(u32::from);
        gpu.passes
            .push(GpuPass::new("lumen_downsample", pair[0], pair[1]).uints(&[hx, hy, w, h]));
    }
    let mut coarser: Option<(GpuImage, [bool; 2])> = None;
    for (i, (level, &src)) in plan.levels.iter().zip(&pyramid).enumerate().rev() {
        let out = pyramid.get(i + 1).copied().unwrap_or(last);
        gpu.passes.push(level_pass(level, src, out, coarser)?);
        coarser = Some((out, level.halve));
    }
    coarser.map(|(image, _)| image)
}

fn level_pass(
    level: &Level,
    src: GpuImage,
    out: GpuImage,
    coarser: Option<(GpuImage, [bool; 2])>,
) -> Option<GpuPass> {
    let [rows, columns] = level.kernels().map(|k| k[k.len() / 2..].to_vec());
    let floats: Vec<f32> = mix_floats(&level.mix)
        .into_iter()
        .chain(rows.iter().copied())
        .chain(columns.iter().copied())
        .collect();
    if floats.len() > MAX_FLOATS {
        return None;
    }
    let reach = [rows.len(), columns.len()].map(|n| n.saturating_sub(1) as u32);
    let halve = coarser.map_or([0, 0], |(_, halve)| halve.map(u32::from));
    let pass = GpuPass::new("lumen_level", src, out)
        .floats(&floats)
        .uints(&[
            reach[0],
            reach[1],
            u32::from(coarser.is_some()),
            halve[0],
            halve[1],
        ]);
    Some(match coarser {
        Some((image, _)) => pass.aux(image),
        None => pass,
    })
}

fn composite(settings: &Settings, core: &Mix, glow: Option<GpuImage>, padding: u32) -> GpuPass {
    let [threshold, width] = settings.knee.shape();
    let floats: Vec<f32> = mix_floats(core)
        .into_iter()
        .chain([
            threshold,
            width,
            settings.glow_opacity,
            settings.source_opacity,
        ])
        .collect();
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
    let uints = [
        view,
        blend,
        u32::from(settings.transfer.is_srgb()),
        dither,
        seed,
        x,
        y,
        u32::from(glow.is_some()),
    ];
    let pass = GpuPass::new("lumen_composite", GpuImage::Source, GpuImage::Output)
        .floats(&floats)
        .uints(&uints);
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

    const MAX_PASSES: usize = 64;
    const MAX_IMAGES: usize = 16;
    const MAX_UINTS: usize = 16;

    fn index(image: GpuImage) -> usize {
        match image {
            GpuImage::Source => 0,
            GpuImage::Output => 1,
            GpuImage::Scratch(i) => 2 + i as usize,
        }
    }

    fn assert_runs_on_the_host(plan: &GpuPlan, frame: [u32; 2], case: &str) {
        assert!(
            (1..=MAX_PASSES).contains(&plan.passes.len()),
            "{case}: {} passes",
            plan.passes.len()
        );
        assert!(
            plan.images.len() <= MAX_IMAGES,
            "{case}: {} images",
            plan.images.len()
        );
        for &[w, h] in &plan.images {
            assert!(
                w > 0 && h > 0 && w <= 2 * frame[0] && h <= 2 * frame[1],
                "{case}: scratch {w}x{h}"
            );
        }
        let mut written = vec![false; plan.images.len() + 2];
        written[0] = true;
        for (n, pass) in plan.passes.iter().enumerate() {
            assert!(
                pass.f.len() <= MAX_FLOATS && pass.u.len() <= MAX_UINTS,
                "{case}: pass {n} values"
            );
            assert_ne!(
                pass.out,
                GpuImage::Source,
                "{case}: pass {n} writes the source"
            );
            for read in std::iter::once(pass.src).chain(pass.aux) {
                assert!(
                    written[index(read)],
                    "{case}: pass {n} reads {read:?} before it is written"
                );
                assert_ne!(read, pass.out, "{case}: pass {n} reads what it writes");
            }
            written[index(pass.out)] = true;
        }
        assert_eq!(
            plan.passes.last().map(|p| p.out),
            Some(GpuImage::Output),
            "{case}"
        );
    }

    #[test]
    fn every_setting_fits_the_host_limits() {
        for radius in [1.0, 3.0, 10.0, 120.0, 600.0, 3000.0] {
            for aspect in [-1.0, -0.6, 0.0, 0.3, 1.0] {
                for aberration in [0.0, 100.0] {
                    for (size, scale) in [
                        ([1, 1], 1.0),
                        ([7, 3000], 1.0),
                        ([1920, 1080], 1.0),
                        ([1920, 1080], 0.37),
                        ([3840, 2160], 1.0),
                    ] {
                        let mut settings = settings_with(&[
                            ("Glow/radius", Value::Number(radius)),
                            ("Glow/aspect", Value::Number(aspect)),
                            ("Aberration/amount", Value::Number(aberration)),
                        ]);
                        settings.scale = scale;
                        let padding = settings.frame_padding(size);
                        let frame = size.map(|s| s + 2 * padding);
                        let case = format!(
                            "radius {radius} aspect {aspect} aberration {aberration} size {size:?} scale {scale}"
                        );
                        let plan = plan(&settings, size, padding)
                            .unwrap_or_else(|| panic!("{case}: no GPU plan"));
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
        assert_eq!(
            [
                count("lumen_emit"),
                count("lumen_downsample"),
                count("lumen_level"),
                count("lumen_composite")
            ],
            [1, levels - 1, levels, 1]
        );
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
