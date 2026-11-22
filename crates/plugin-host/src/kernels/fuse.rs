use std::ops::Range;

use craft_plugin::{GpuImage, GpuPass, GpuPlan, HostKernel, Pixel};

use super::{CLEAR, Image, each_row, pointwise};

const MAX_FUSED_PASSES: usize = 14;
const MAX_FUSED_INPUTS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    Input(usize),
    Pass(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Op<'a> {
    pub kernel: &'a HostKernel,
    pub src: Ref,
    pub aux: Option<Ref>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fused<'a> {
    pub passes: Range<usize>,
    pub inputs: Vec<GpuImage>,
    pub out: GpuImage,
    pub ops: Vec<Op<'a>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step<'a> {
    Single(usize),
    Fused(Fused<'a>),
}

fn pointwise(pass: &GpuPass) -> Option<&HostKernel> {
    pass.host.as_ref().filter(|kernel| kernel.pointwise() && pass.draw.is_none())
}

fn reads_aux(kernel: &HostKernel) -> bool {
    matches!(kernel, HostKernel::Blend { .. })
}

pub(crate) fn live_after(passes: &[GpuPass], image: GpuImage) -> bool {
    for pass in passes {
        if pass.src == image || pass.aux == Some(image) {
            return true;
        }
        if pass.out == image {
            return false;
        }
    }
    image == GpuImage::Output
}

fn group<'a>(plan: &'a GpuPlan, passes: Range<usize>) -> Option<Fused<'a>> {
    let members = &plan.passes[passes.clone()];
    let mut inputs: Vec<GpuImage> = Vec::new();
    let mut ops = Vec::with_capacity(members.len());
    for (k, pass) in members.iter().enumerate() {
        let kernel = pointwise(pass)?;
        let mut resolve = |image: GpuImage| match members[..k].iter().rposition(|earlier| earlier.out == image) {
            Some(j) => Ref::Pass(j),
            None => Ref::Input(inputs.iter().position(|known| *known == image).unwrap_or_else(|| {
                inputs.push(image);
                inputs.len() - 1
            })),
        };
        let src = resolve(pass.src);
        let aux = pass.aux.filter(|_| reads_aux(kernel)).map(&mut resolve);
        ops.push(Op { kernel, src, aux });
    }
    let out = members.last()?.out;
    let later = &plan.passes[passes.end..];
    let written_last = |k: usize| !members[k + 1..].iter().any(|pass| pass.out == members[k].out);
    let intermediate_live = (0..members.len()).any(|k| members[k].out != out && written_last(k) && live_after(later, members[k].out));
    (inputs.len() <= MAX_FUSED_INPUTS && !intermediate_live).then_some(Fused { passes, inputs, out, ops })
}

pub fn steps(plan: &GpuPlan, frame: [u32; 2]) -> Vec<Step<'_>> {
    let size = |pass: &GpuPass| plan.size_of(pass.out, frame);
    let mut steps = Vec::new();
    let mut start = 0;
    while start < plan.passes.len() {
        let first = &plan.passes[start];
        let candidates = plan.passes[start..].iter().take(MAX_FUSED_PASSES).take_while(|pass| pointwise(pass).is_some() && size(pass) == size(first)).count();
        let fused = (1..=candidates).rev().find_map(|len| group(plan, start..start + len));
        match fused {
            Some(fused) => {
                start = fused.passes.end;
                steps.push(Step::Fused(fused));
            }
            None => {
                steps.push(Step::Single(start));
                start += 1;
            }
        }
    }
    steps
}

pub(crate) fn run(fused: &Fused, inputs: &[Image], width: usize, out: &mut [Pixel]) {
    let last = fused.ops.len() - 1;
    each_row(
        out,
        width,
        || (vec![Vec::new(); last], vec![CLEAR; width]),
        |(held, clear), y, row| {
            for (k, op) in fused.ops.iter().enumerate() {
                let (done, rest) = held.split_at_mut(k.min(last));
                let read = |r: Ref| match r {
                    Ref::Input(i) => inputs[i].row(y),
                    Ref::Pass(j) => &done[j][..],
                };
                let (src, top) = (read(op.src), op.aux.map_or(&clear[..], read));
                if k == last {
                    pointwise::row(op.kernel, src, top, y, row);
                } else {
                    let target = &mut rest[0];
                    target.resize(width, CLEAR);
                    pointwise::row(op.kernel, src, top, y, target);
                }
            }
        },
    );
}
