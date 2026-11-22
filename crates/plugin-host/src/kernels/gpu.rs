use craft_plugin::{BlendMode, Filter, GpuImage, GpuPass, GpuPlan, HostKernel, Measure, Taps};

use super::fuse::{Fused, Op, Ref, Step, live_after, steps};
use super::{blur_taps, resample_scale};

const FIRST_REGISTER: f32 = 2.0;
const OP_HEADER: usize = 4;

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
            Step::Fused(fused) => lowered.push(Lowered::Host(pointwise(fused, "craft_pointwise", Vec::new()))),
            Step::Single(index) => {
                let index = *index;
                let pass = &plan.passes[index];
                let Some(kernel) = &pass.host else {
                    lowered.push(Lowered::Plugin(pass));
                    continue;
                };
                let (src, out) = (StepImage::Plan(pass.src), StepImage::Plan(pass.out));
                let aux = pass.aux.map(StepImage::Plan);
                let step = |entry, f, u, data| GpuStep { entry, src, aux, out, f, u, data };
                let mut separable = |x: Vec<f32>, y: Vec<f32>| {
                    let temp = StepImage::Temp(temps.len());
                    temps.push(size(pass.out));
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
                        let aux_ref = pass.aux.map(|_| Ref::Input(1));
                        let op = Op { kernel: pointwise_kernel, src: Ref::Input(0), aux: aux_ref };
                        vec![GpuStep { data: program(std::slice::from_ref(&op)), ..step("craft_pointwise", Vec::new(), Vec::new(), Vec::new()) }]
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
    let mut step = pointwise(fused, "craft_pointwise", resample_scale(*scale, size(resample.src), size(resample.out)).to_vec());
    step.u = vec![slot as u32 + 1, u32::from(*filter == Filter::Bicubic)];
    if slot == 0 {
        step.src = StepImage::Plan(resample.src);
    } else {
        step.aux = Some(StepImage::Plan(resample.src));
    }
    Some(step)
}

fn pointwise(fused: &Fused, entry: &'static str, f: Vec<f32>) -> GpuStep {
    GpuStep {
        entry,
        src: StepImage::Plan(fused.inputs[0]),
        aux: fused.inputs.get(1).copied().map(StepImage::Plan),
        out: StepImage::Plan(fused.out),
        f,
        u: Vec::new(),
        data: program(&fused.ops),
    }
}

fn register(r: Ref) -> f32 {
    match r {
        Ref::Input(i) => i as f32,
        Ref::Pass(j) => FIRST_REGISTER + j as f32,
    }
}

fn halves(value: u32) -> [f32; 2] {
    [(value & 0xffff) as f32, (value >> 16) as f32]
}

fn program(ops: &[Op]) -> Vec<f32> {
    let mut data = vec![ops.len() as f32];
    data.resize(1 + OP_HEADER * ops.len(), 0.0);
    for (k, op) in ops.iter().enumerate() {
        let (code, params): (f32, Vec<f32>) = match op.kernel {
            HostKernel::Blend { mode, amount } => (0.0, vec![BlendMode::ALL.iter().position(|m| m == mode).unwrap_or(0) as f32, *amount]),
            HostKernel::Curves { table } => (1.0, std::iter::once(table.len() as f32).chain(table.iter().flatten().copied()).collect()),
            HostKernel::Matrix { rows } => (2.0, rows.iter().flatten().copied().collect()),
            HostKernel::ToLinear => (3.0, Vec::new()),
            HostKernel::ToSrgb => (4.0, Vec::new()),
            HostKernel::BrightnessCurve { measure, curve, range } => {
                let steps_per_unit = (curve.len() - 1) as f32 / range;
                (5.0, [f32::from(*measure == Measure::Luma), curve.len() as f32, steps_per_unit].into_iter().chain(curve.iter().copied()).collect())
            }
            HostKernel::Dither { seed, origin } => (6.0, [*seed, origin[0] as u32, origin[1] as u32].into_iter().flat_map(halves).collect()),
            HostKernel::LightAlpha => (7.0, Vec::new()),
            HostKernel::Blur { .. } | HostKernel::Resample { .. } | HostKernel::Convolve { .. } | HostKernel::Warp { .. } => (-1.0, Vec::new()),
        };
        let header = [code, register(op.src), op.aux.map_or(1.0, register), data.len() as f32];
        data[1 + OP_HEADER * k..1 + OP_HEADER * (k + 1)].copy_from_slice(&header);
        data.extend(params);
    }
    data
}
