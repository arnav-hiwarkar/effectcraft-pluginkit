use craft_plugin::{BlendMode, Filter, HostKernel, Taps};

use super::{blur_taps, resample_scale};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepImage {
    Pass,
    Temp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuStep {
    pub entry: &'static str,
    pub src: StepImage,
    pub out: StepImage,
    pub f: Vec<f32>,
    pub u: Vec<u32>,
    pub data: Vec<f32>,
}

fn step(entry: &'static str, f: Vec<f32>, u: Vec<u32>, data: Vec<f32>) -> GpuStep {
    GpuStep { entry, src: StepImage::Pass, out: StepImage::Pass, f, u, data }
}

fn separable(x: Vec<f32>, y: Vec<f32>) -> Vec<GpuStep> {
    vec![
        GpuStep { out: StepImage::Temp, ..step("craft_convolve_x", Vec::new(), vec![x.len() as u32], x) },
        GpuStep { src: StepImage::Temp, ..step("craft_convolve_y", Vec::new(), vec![y.len() as u32], y) },
    ]
}

pub fn gpu_steps(kernel: &HostKernel, src: [u32; 2], out: [u32; 2]) -> Vec<GpuStep> {
    match kernel {
        HostKernel::Blur { shape, radius } => {
            let [x, y] = radius.map(|r| blur_taps(*shape, r));
            separable(x, y)
        }
        HostKernel::Convolve { taps: Taps::Separable { x, y } } => separable(x.clone(), y.clone()),
        HostKernel::Convolve { taps: Taps::Full { width, weights } } => {
            let rows = weights.len() as u32 / (*width).max(1);
            vec![step("craft_convolve_full", Vec::new(), vec![*width, rows], weights.clone())]
        }
        HostKernel::Resample { filter, scale } => {
            let filter = u32::from(*filter == Filter::Bicubic);
            vec![step("craft_resample", resample_scale(*scale, src, out).to_vec(), vec![filter], Vec::new())]
        }
        HostKernel::Blend { mode, amount } => {
            let mode = BlendMode::ALL.iter().position(|m| m == mode).unwrap_or(0) as u32;
            vec![step("craft_blend", vec![*amount], vec![mode], Vec::new())]
        }
        HostKernel::Curves { table } => vec![step("craft_curves", Vec::new(), vec![table.len() as u32], table.iter().flatten().copied().collect())],
        HostKernel::Warp { amount } => vec![step("craft_warp", amount.to_vec(), Vec::new(), Vec::new())],
        HostKernel::Matrix { rows } => vec![step("craft_matrix", rows.iter().flatten().copied().collect(), Vec::new(), Vec::new())],
    }
}
