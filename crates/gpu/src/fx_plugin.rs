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
const PIECE: Duration = Duration::from_millis(10);
const FIRST_PIECE_PIXELS: f64 = 65_536.0;
const MIN_NS_PER_PIXEL: f64 = 1e-3;

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
    let mut clock = Clock::new(e, id);
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
                for rows in clock.bands(e, &pass.kernel, w, h) {
                    let band = e.data(&[rows.start as f32, rows.end as f32]);
                    let images_bound = KernelImages { src: &src, aux: aux.as_ref(), out: &out };
                    e.dispatch(Kernel::Plugin { pipeline, label: &pass.kernel }, &params(pass), images_bound, Some(&band), (w.div_ceil(16), h.div_ceil(16)));
                    if let Err(why) = clock.ran(e, n, &pass.kernel, u64::from(w) * u64::from(rows.end - rows.start)) {
                        g.plugin_stop(id, &why);
                        return None;
                    }
                }
                Some(out)
            }
            Some(_) => {
                let stays_open = program.passes.get(n + 1).is_some_and(|next| next.out == pass.out && next.draw.as_ref().is_some_and(|draw| !draw.clear));
                let below = images.get(target).cloned().flatten();
                let target = Target { index: target, size: [w, h], below: below.as_ref(), stays_open };
                let drawn = match stage.draw(e, pass, &params(pass), &src, aux.as_ref(), target) {
                    Ok(out) => out,
                    Err(why) => {
                        g.plugin_declined(id, &why);
                        return None;
                    }
                };
                if let Err(why) = clock.ran(e, n, &pass.kernel, u64::from(w) * u64::from(h)) {
                    g.plugin_stop(id, &why);
                    return None;
                }
                drawn
            }
        };
        if let Some(out) = out {
            *images.get_mut(target)? = Some(out);
        }
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

struct Clock<'a> {
    effect: &'a str,
    budget: Option<Duration>,
    start: Instant,
    group: Vec<(String, f64)>,
    predicted_ns: f64,
}

impl<'a> Clock<'a> {
    fn new(e: &Enc, effect: &'a str) -> Self {
        let budget = e.g.deferred.is_none().then(|| e.g.plugin_gpu_budget());
        Clock { effect, budget, start: Instant::now(), group: Vec::new(), predicted_ns: 0.0 }
    }

    fn ns_per_pixel(&self, e: &Enc, kernel: &str) -> f64 {
        e.g.plugin_cost(self.effect, kernel).unwrap_or(PIECE.as_nanos() as f64 / FIRST_PIECE_PIXELS)
    }

    fn bands(&self, e: &Enc, kernel: &str, w: u32, h: u32) -> Vec<std::ops::Range<u32>> {
        let h = h.max(1);
        let rows = match self.budget {
            None => h,
            Some(_) => {
                let per_row = self.ns_per_pixel(e, kernel) * f64::from(w.max(1));
                ((PIECE.as_nanos() as f64 / per_row).floor() as u32).clamp(1, h)
            }
        };
        (0..h).step_by(rows as usize).map(|first| first..(first + rows).min(h)).collect()
    }

    fn ran(&mut self, e: &mut Enc, pass: usize, kernel: &str, pixels: u64) -> Result<(), String> {
        let Some(budget) = self.budget else { return Ok(()) };
        let cost = self.ns_per_pixel(e, kernel);
        self.predicted_ns += cost * pixels as f64;
        self.group.push((kernel.to_string(), cost));
        if self.predicted_ns < PIECE.as_nanos() as f64 {
            return Ok(());
        }
        e.submit();
        let began = Instant::now();
        if let Some(submission) = e.last_submission() {
            e.g.poll_wait(submission, budget.saturating_sub(self.start.elapsed()).max(Duration::from_millis(1)));
        }
        let factor = (began.elapsed().as_nanos() as f64 / self.predicted_ns.max(1.0)).clamp(1.0 / 64.0, 64.0);
        for (kernel, cost) in std::mem::take(&mut self.group) {
            e.g.plugin_learn(self.effect, &kernel, (cost * factor).max(MIN_NS_PER_PIXEL));
        }
        self.predicted_ns = 0.0;
        let spent = self.start.elapsed();
        if spent > budget {
            return Err(format!(
                "the plugin effect {} stopped at pass {pass} (`{kernel}`): its GPU work took {} ms, past the {} ms budget for one frame",
                self.effect,
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
