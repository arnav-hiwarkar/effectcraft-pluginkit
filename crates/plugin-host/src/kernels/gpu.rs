use std::fmt::Write;

use craft_plugin::{BlendMode, Filter, GpuImage, GpuPass, GpuPlan, HostKernel, Measure, Taps};

use super::fuse::{Fused, Op, Ref, Step, live_after, steps};
use super::{blur_taps, resample_scale};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepImage {
    Plan(GpuImage),
    Temp(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuStep {
    pub entry: &'static str,
    pub src: StepImage,
    pub aux: Option<StepImage>,
    pub out: StepImage,
    pub f: Vec<f32>,
    pub u: Vec<u32>,
    pub data: Vec<f32>,
    pub wgsl: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lowered<'a> {
    Plugin(&'a GpuPass),
    Host(GpuStep),
}

pub fn lower(plan: &GpuPlan, frame: [u32; 2]) -> (Vec<Lowered<'_>>, Vec<[u32; 2]>) {
    let size = |image: GpuImage| plan.size_of(image, frame).unwrap_or(frame);
    let mut temps = Vec::new();
    let mut lowered = Vec::new();
    let steps = steps(plan, frame);
    let mut at = 0;
    while at < steps.len() {
        if let (Step::Single(index), Some(Step::Fused(fused))) = (&steps[at], steps.get(at + 1))
            && let Some(step) = resampled_input(plan, &plan.passes[*index], fused, size)
        {
            lowered.push(Lowered::Host(step));
            at += 2;
            continue;
        }
        let step = &steps[at];
        at += 1;
        match step {
            Step::Fused(fused) => lowered.push(Lowered::Host(pointwise(fused, None, Vec::new()))),
            Step::Single(index) => {
                let index = *index;
                let pass = &plan.passes[index];
                let Some(kernel) = &pass.host else {
                    lowered.push(Lowered::Plugin(pass));
                    continue;
                };
                let (src, out) = (StepImage::Plan(pass.src), StepImage::Plan(pass.out));
                let aux = pass.aux.map(StepImage::Plan);
                let step = |entry, f, u, data| GpuStep { entry, src, aux, out, f, u, data, wgsl: String::new() };
                let mut separable = |x: Vec<f32>, y: Vec<f32>| {
                    let out_size = size(pass.out);
                    let temp = StepImage::Temp(temps.iter().position(|known| *known == out_size).unwrap_or_else(|| {
                        temps.push(out_size);
                        temps.len() - 1
                    }));
                    [
                        GpuStep { out: temp, ..step("craft_convolve_x", Vec::new(), vec![x.len() as u32], x) },
                        GpuStep { src: temp, ..step("craft_convolve_y", Vec::new(), vec![y.len() as u32], y) },
                    ]
                };
                let steps: Vec<GpuStep> = match kernel {
                    HostKernel::Blur { shape, radius } => {
                        let [x, y] = radius.map(|r| blur_taps(*shape, r));
                        separable(x, y).to_vec()
                    }
                    HostKernel::Convolve { taps: Taps::Separable { x, y } } => separable(x.clone(), y.clone()).to_vec(),
                    HostKernel::Convolve { taps: Taps::Full { width, weights } } => {
                        let rows = weights.len() as u32 / (*width).max(1);
                        vec![step("craft_convolve_full", Vec::new(), vec![*width, rows], weights.clone())]
                    }
                    HostKernel::Resample { filter, scale } => {
                        let filter = u32::from(*filter == Filter::Bicubic);
                        vec![step("craft_resample", resample_scale(*scale, size(pass.src), size(pass.out)).to_vec(), vec![filter], Vec::new())]
                    }
                    HostKernel::Warp { amount } => vec![step("craft_warp", amount.to_vec(), Vec::new(), Vec::new())],
                    pointwise_kernel => {
                        let op = Op { kernel: pointwise_kernel, src: Ref::Input(0), aux: pass.aux.map(|_| Ref::Input(1)) };
                        let single =
                            Fused { passes: index..index + 1, inputs: std::iter::once(pass.src).chain(pass.aux).collect(), out: pass.out, ops: vec![op] };
                        vec![pointwise(&single, None, Vec::new())]
                    }
                };
                lowered.extend(steps.into_iter().map(Lowered::Host));
            }
        }
    }
    (lowered, temps)
}

fn resampled_input(plan: &GpuPlan, resample: &GpuPass, fused: &Fused, size: impl Fn(GpuImage) -> [u32; 2]) -> Option<GpuStep> {
    let Some(HostKernel::Resample { filter, scale }) = &resample.host else { return None };
    let slot = fused.inputs.iter().position(|image| *image == resample.out)?;
    if live_after(&plan.passes[fused.passes.end..], resample.out) {
        return None;
    }
    let mut step = pointwise(fused, Some((slot, *filter)), resample_scale(*scale, size(resample.src), size(resample.out)).to_vec());
    if slot == 0 {
        step.src = StepImage::Plan(resample.src);
    } else {
        step.aux = Some(StepImage::Plan(resample.src));
    }
    Some(step)
}

fn pointwise(fused: &Fused, resampled: Option<(usize, Filter)>, f: Vec<f32>) -> GpuStep {
    let mut wgsl = String::from("@compute @workgroup_size(16, 16)\nfn craft_pointwise(@builtin(global_invocation_id) id: vec3<u32>) {\n");
    wgsl.push_str("    if (!inside_out(id.xy)) { return; }\n    let p = vec2<i32>(id.xy);\n");
    for (slot, texture) in ["src", "aux"].into_iter().enumerate().take(fused.inputs.len()) {
        let _ = match resampled {
            Some((at, filter)) if at == slot => {
                let bicubic = filter == Filter::Bicubic;
                writeln!(wgsl, "    let i{slot} = craft_resampled({texture}, p, vec2<f32>(pass_f(0u), pass_f(1u)), {bicubic});")
            }
            _ => writeln!(wgsl, "    let i{slot} = texel({texture}, p);"),
        };
    }
    let mut data = Vec::new();
    for (k, op) in fused.ops.iter().enumerate() {
        let (v, t) = (value(op.src), op.aux.map_or_else(|| "vec4<f32>(0.0)".to_string(), value));
        let at = data.len();
        let expr = match op.kernel {
            HostKernel::Blend { mode, amount } => {
                data.push(*amount);
                format!("craft_amount({v}, {}, craft_data({at}u))", blended(*mode, &v, &t))
            }
            HostKernel::Curves { table } => {
                data.push(table.len() as f32);
                data.extend(table.iter().flatten());
                format!("craft_curves({at}u, {v})")
            }
            HostKernel::Matrix { rows } => {
                data.extend(rows.iter().flatten());
                format!("craft_matrix({at}u, {v})")
            }
            HostKernel::ToLinear => format!("craft_linear({v})"),
            HostKernel::ToSrgb => format!("craft_srgb({v})"),
            HostKernel::BrightnessCurve { measure, curve, range } => {
                let steps_per_unit = (curve.len() - 1) as f32 / range;
                data.extend([f32::from(*measure == Measure::Luma), curve.len() as f32, steps_per_unit].into_iter().chain(curve.iter().copied()));
                format!("craft_brightness_curve({at}u, {v})")
            }
            HostKernel::Dither { seed, origin } => {
                data.extend([*seed, origin[0] as u32, origin[1] as u32].into_iter().flat_map(halves));
                format!("craft_dither({at}u, {v}, id.xy)")
            }
            HostKernel::LightAlpha => format!("craft_light_alpha({v})"),
            HostKernel::Blur { .. } | HostKernel::Resample { .. } | HostKernel::Convolve { .. } | HostKernel::Warp { .. } => v,
        };
        let _ = writeln!(wgsl, "    let v{k} = {expr};");
    }
    let _ = writeln!(wgsl, "    store(id.xy, v{});\n}}", fused.ops.len() - 1);
    GpuStep {
        entry: "craft_pointwise",
        src: StepImage::Plan(fused.inputs[0]),
        aux: fused.inputs.get(1).copied().map(StepImage::Plan),
        out: StepImage::Plan(fused.out),
        f,
        u: Vec::new(),
        data,
        wgsl,
    }
}

fn value(r: Ref) -> String {
    match r {
        Ref::Input(i) => format!("i{i}"),
        Ref::Pass(j) => format!("v{j}"),
    }
}

fn blended(mode: BlendMode, b: &str, t: &str) -> String {
    match mode {
        BlendMode::Over => format!("{t} + {b} * (1.0 - {t}.a)"),
        BlendMode::Add => format!("{b} + {t}"),
        BlendMode::Subtract => format!("{b} - {t}"),
        BlendMode::Multiply => format!("{b} * {t}"),
        BlendMode::Screen => format!("{b} + {t} - {b} * {t}"),
        BlendMode::Overlay => format!("select(1.0 - 2.0 * (1.0 - {b}) * (1.0 - {t}), 2.0 * {b} * {t}, {b} <= vec4<f32>(0.5))"),
        BlendMode::Darken => format!("min({b}, {t})"),
        BlendMode::Lighten => format!("max({b}, {t})"),
        BlendMode::Difference => format!("abs({b} - {t})"),
    }
}

fn halves(value: u32) -> [f32; 2] {
    [(value & 0xffff) as f32, (value >> 16) as f32]
}
