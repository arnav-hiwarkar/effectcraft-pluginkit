use effectcraft_effects::EffectCtx;
use effectcraft_effects::dynamic::{GpuPass, GpuProgram, GpuSlot, Grid, Registered};

use crate::context::{Enc, GpuImage, Kernel, KernelImages, Params};
use crate::effects::GBuf;

const MAX_PASSES: usize = 64;
const MAX_IMAGES: usize = 16;
const MAX_SCRATCH_FRAMES: u64 = 8;
const FLOATS: usize = 48;
const UINTS: usize = 16;
const SOURCE: usize = 0;
const OUTPUT: usize = 1;

pub(crate) fn apply(e: &mut Enc, registered: &Registered, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let effect = &registered.effect;
    let (Some(wgsl), Some(plan)) = (effect.shader.as_deref(), effect.gpu.as_ref()) else { return None };
    let (id, revision) = (registered.spec.id, effect.revision);
    let g = e.g;
    if let Err(why) = g.plugin_pipelines(id, revision, wgsl, &[]) {
        g.plugin_declined(id, &why);
        return None;
    }
    let program = plan(ctx, Grid { width: b.img.width, height: b.img.height, offset: b.offset, scale: b.scale })?;
    let unpadded = [b.img.width, b.img.height];
    b.pad(e, program.padding)?;
    let frame = [b.img.width, b.img.height];
    if frame != unpadded.map(|side| side.saturating_add(program.padding.saturating_mul(2))) {
        g.plugin_declined(id, "the frame could not be padded as far as the plan asks");
        return None;
    }
    let kernels: Vec<&str> = program.passes.iter().map(|pass| pass.kernel.as_str()).collect();
    let pipelines = match check(&program, frame, |w, h| g.fits(w, h)).and_then(|()| g.plugin_pipelines(id, revision, wgsl, &kernels)) {
        Ok(pipelines) => pipelines,
        Err(why) => {
            g.plugin_declined(id, &why);
            return None;
        }
    };
    let sizes: Vec<[u32; 2]> = [frame, frame].into_iter().chain(program.images.iter().copied()).collect();
    let mut images: Vec<Option<GpuImage>> = vec![None; sizes.len()];
    *images.get_mut(SOURCE)? = Some(b.img.clone());
    for (pass, pipeline) in program.passes.iter().zip(&pipelines) {
        let read = |slot: GpuSlot| images.get(index(slot, program.images.len())?).cloned().flatten();
        let src = read(pass.src)?;
        let aux = match pass.aux {
            Some(slot) => Some(read(slot)?),
            None => None,
        };
        let target = index(pass.out, program.images.len())?;
        let [w, h] = *sizes.get(target)?;
        let out = e.image(w, h);
        let images_bound = KernelImages { src: &src, aux: aux.as_ref(), out: &out };
        e.dispatch(Kernel::Plugin { pipeline, label: &pass.kernel }, &params(pass), images_bound, None, (w.div_ceil(16), h.div_ceil(16)));
        *images.get_mut(target)? = Some(out);
    }
    b.img = images.get_mut(OUTPUT)?.take()?;
    log::debug!("gpu {}: plugin effect {id} ran {} passes at {}x{}", g.name, program.passes.len(), frame[0], frame[1]);
    Some(b)
}

fn index(slot: GpuSlot, scratch: usize) -> Option<usize> {
    match slot {
        GpuSlot::Source => Some(SOURCE),
        GpuSlot::Output => Some(OUTPUT),
        GpuSlot::Scratch(i) if i < scratch => Some(2 + i),
        GpuSlot::Scratch(_) => None,
    }
}

fn params(pass: &GpuPass) -> Params {
    let mut p = Params::default();
    for (i, value) in pass.f.iter().enumerate() {
        if let Some(lane) = p.f.get_mut(i / 4).and_then(|row| row.get_mut(i % 4)) {
            *lane = *value;
        }
    }
    for (i, value) in pass.u.iter().enumerate() {
        if let Some(lane) = p.u.get_mut(i / 4).and_then(|row| row.get_mut(i % 4)) {
            *lane = *value;
        }
    }
    p
}

fn check(program: &GpuProgram, frame: [u32; 2], fits: impl Fn(u32, u32) -> bool) -> Result<(), String> {
    let passes = program.passes.len();
    if !(1..=MAX_PASSES).contains(&passes) {
        return Err(format!("a GPU plan has 1 to {MAX_PASSES} passes, this one has {passes}"));
    }
    if program.images.len() > MAX_IMAGES {
        return Err(format!("a GPU plan has at most {MAX_IMAGES} scratch images, this one has {}", program.images.len()));
    }
    let [fw, fh] = frame;
    for (i, &[w, h]) in program.images.iter().enumerate() {
        let within = w > 0 && h > 0 && w <= fw.saturating_mul(2) && h <= fh.saturating_mul(2);
        if !within || !fits(w, h) {
            return Err(format!("scratch image {i} is {w}x{h}; it must be non-empty and at most twice the {fw}x{fh} padded frame"));
        }
    }
    let area = |w: u32, h: u32| u64::from(w) * u64::from(h);
    let scratch_area: u64 = program.images.iter().map(|&[w, h]| area(w, h)).sum();
    if scratch_area > MAX_SCRATCH_FRAMES * area(fw, fh) {
        return Err(format!("the scratch images together exceed {MAX_SCRATCH_FRAMES} times the {fw}x{fh} padded frame"));
    }
    let mut written = vec![false; program.images.len() + 2];
    if let Some(source) = written.get_mut(SOURCE) {
        *source = true;
    }
    for (n, pass) in program.passes.iter().enumerate() {
        if pass.f.len() > FLOATS || pass.u.len() > UINTS {
            return Err(format!("pass {n} has {} floats and {} uints; the limits are {FLOATS} and {UINTS}", pass.f.len(), pass.u.len()));
        }
        if pass.out == GpuSlot::Source {
            return Err(format!("pass {n} writes the source image"));
        }
        for slot in std::iter::once(pass.src).chain(pass.aux) {
            let i = index(slot, program.images.len()).ok_or_else(|| format!("pass {n} reads {slot:?}, which the plan does not declare"))?;
            if !written.get(i).copied().unwrap_or(false) {
                return Err(format!("pass {n} reads {slot:?} before a pass writes it"));
            }
            if slot == pass.out {
                return Err(format!("pass {n} reads the image it writes"));
            }
        }
        let out = index(pass.out, program.images.len()).ok_or_else(|| format!("pass {n} writes {:?}, which the plan does not declare", pass.out))?;
        if let Some(done) = written.get_mut(out) {
            *done = true;
        }
    }
    if !written.get(OUTPUT).copied().unwrap_or(false) {
        return Err("no pass writes the output image".into());
    }
    Ok(())
}
