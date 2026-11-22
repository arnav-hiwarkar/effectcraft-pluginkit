use std::time::Duration;

use effectcraft_effects::EffectCtx;
use effectcraft_effects::dynamic::{GpuPass, GpuProgram, GpuSlot, Grid, Registered};
use web_time::Instant;

use crate::context::{Enc, GpuImage, Kernel, KernelImages, Params};
use crate::effects::GBuf;
use crate::fx_key::{Placed, fit, fitted};
use crate::plugin_draw::{Stage, Target, check_data, check_draw};

const SOURCE: usize = 0;
const OUTPUT: usize = 1;

pub(crate) fn apply(e: &mut Enc, registered: &Registered, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let effect = &registered.effect;
    if registered.gpu_stopped.get().is_some() {
        return None;
    }
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
    let kernels: Vec<&str> = program.passes.iter().filter(|pass| pass.draw.is_none()).map(|pass| pass.kernel.as_str()).collect();
    let pipelines = match check(&program, |w, h| g.fits(w, h)).and_then(|()| g.plugin_pipelines(id, revision, wgsl, &kernels)) {
        Ok(pipelines) => pipelines,
        Err(why) => {
            g.plugin_declined(id, &why);
            return None;
        }
    };
    let sizes: Vec<[u32; 2]> = [frame, frame]
        .into_iter()
        .chain(program.images.iter().copied())
        .chain(program.textures.iter().map(|texture| texture.size))
        .chain(program.layers.iter().map(|_| frame))
        .collect();
    let mut stage = Stage::new(id, revision, wgsl, &program);
    let mut images: Vec<Option<GpuImage>> = vec![None; sizes.len()];
    *images.get_mut(SOURCE)? = Some(b.img.clone());
    for texture in 0..program.textures.len() {
        *images.get_mut(index(GpuSlot::Texture(texture), &program)?)? = Some(stage.texture(g, texture)?);
    }
    for (layer, param) in program.layers.iter().enumerate() {
        *images.get_mut(index(GpuSlot::Layer(layer), &program)?)? = Some(layer_image(e, ctx, &b, param)?);
    }
    let clock = Clock::start(e);
    let mut compute = pipelines.iter();
    for (n, pass) in program.passes.iter().enumerate() {
        let read = |slot: GpuSlot| images.get(index(slot, &program)?).cloned().flatten();
        let src = read(pass.src)?;
        let aux = match pass.aux {
            Some(slot) => Some(read(slot)?),
            None => None,
        };
        let target = index(pass.out, &program)?;
        let [w, h] = *sizes.get(target)?;
        let out = match &pass.draw {
            None => {
                let pipeline = compute.next()?;
                let out = e.image(w, h);
                let images_bound = KernelImages { src: &src, aux: aux.as_ref(), out: &out };
                e.dispatch(Kernel::Plugin { pipeline, label: &pass.kernel }, &params(pass), images_bound, None, (w.div_ceil(16), h.div_ceil(16)));
                Some(out)
            }
            Some(_) => {
                let stays_open = program.passes.get(n + 1).is_some_and(|next| next.out == pass.out && next.draw.as_ref().is_some_and(|draw| !draw.clear));
                let below = images.get(target).cloned().flatten();
                let target = Target { index: target, size: [w, h], below: below.as_ref(), stays_open };
                match stage.draw(e, pass, &params(pass), &src, aux.as_ref(), target) {
                    Ok(out) => out,
                    Err(why) => {
                        g.plugin_declined(id, &why);
                        return None;
                    }
                }
            }
        };
        if let Some(out) = out {
            *images.get_mut(target)? = Some(out);
        }
    }
    if let Err(why) = clock.ran(e, id, &program) {
        if registered.gpu_stopped.set(why.clone()).is_ok() {
            g.plugin_declined(id, &why);
        }
        return None;
    }
    b.img = images.get_mut(OUTPUT)?.take()?;
    log::debug!("gpu {}: plugin effect {id} ran {} passes at {}x{}", g.name, program.passes.len(), frame[0], frame[1]);
    Some(b)
}

fn layer_image(e: &mut Enc, ctx: &EffectCtx, frame: &GBuf, param: &str) -> Option<GpuImage> {
    let [w, h] = [frame.img.width, frame.img.height];
    if let Some(shared) = ctx.shared_layer_param(param) {
        if shared.buf.img.is_empty() {
            return Some(e.zeros(w, h));
        }
        let src = e.g.upload_buf(&shared.buf)?;
        return Some(fit(e, ctx, frame, &src, Placed { offset: shared.offset, scale: shared.buf.scale, size: shared.size }, false));
    }
    let fitted = fitted(e, ctx, frame, param, true, false)?;
    if fitted.is_some() {
        e.g.transfers.layers_copied.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Some(fitted.unwrap_or_else(|| e.zeros(w, h)))
}

struct Clock {
    budget: Option<Duration>,
    start: Instant,
}

impl Clock {
    fn start(e: &mut Enc) -> Self {
        let budget = e.g.deferred.is_none().then(|| e.g.plugin_gpu_budget());
        if budget.is_some() {
            e.submit();
            if let Some(earlier) = e.last_submission() {
                e.g.poll_wait(earlier, crate::readback::TIMEOUT);
            }
        }
        Clock { budget, start: Instant::now() }
    }

    fn ran(&self, e: &mut Enc, effect: &str, program: &GpuProgram) -> Result<(), String> {
        let Some(budget) = self.budget else { return Ok(()) };
        e.submit();
        if let Some(submission) = e.last_submission() {
            e.g.poll_wait(submission, budget.saturating_sub(self.start.elapsed()).max(Duration::from_millis(1)));
        }
        let spent = self.start.elapsed();
        if spent > budget {
            let kernels: Vec<&str> = program.passes.iter().map(|pass| pass.kernel.as_str()).collect();
            return Err(format!(
                "the plugin effect {effect} was stopped: the GPU work of one frame, passes {kernels:?}, took {} ms, past the {} ms budget. It renders on the CPU until the plugin is reloaded",
                spent.as_millis(),
                budget.as_millis()
            ));
        }
        Ok(())
    }
}

fn index(slot: GpuSlot, program: &GpuProgram) -> Option<usize> {
    let scratch = program.images.len();
    match slot {
        GpuSlot::Source => Some(SOURCE),
        GpuSlot::Output => Some(OUTPUT),
        GpuSlot::Scratch(i) if i < scratch => Some(2 + i),
        GpuSlot::Texture(i) if i < program.textures.len() => Some(2 + scratch + i),
        GpuSlot::Layer(i) if i < program.layers.len() => Some(2 + scratch + program.textures.len() + i),
        GpuSlot::Scratch(_) | GpuSlot::Texture(_) | GpuSlot::Layer(_) => None,
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

fn check(program: &GpuProgram, fits: impl Fn(u32, u32) -> bool) -> Result<(), String> {
    if let Some((i, &[w, h])) = program.images.iter().enumerate().find(|&(_, &[w, h])| !fits(w, h)) {
        return Err(format!("scratch image {i} is {w}x{h}, larger than this GPU holds"));
    }
    check_data(program, &fits)?;
    for (n, pass) in program.passes.iter().enumerate() {
        if let Some(draw) = &pass.draw {
            check_draw(n, draw, program, true)?;
        }
    }
    Ok(())
}
