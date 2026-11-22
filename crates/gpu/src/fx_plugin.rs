use effectcraft_effects::EffectCtx;
use effectcraft_effects::dynamic::{GpuPass, GpuProgram, GpuSlot, Grid, Registered};

use crate::context::{Enc, GpuContext, GpuImage, Kernel, KernelImages, Params};
use crate::effects::GBuf;
use crate::fx_key::{Placed, fit, fitted};
use crate::plugin_draw::{Stage, Target, check_data, check_draw};

const SOURCE: usize = 0;
const OUTPUT: usize = 1;

pub(crate) fn apply(e: &mut Enc, registered: &Registered, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let effect = &registered.effect;
    let (Some(wgsl), Some(plan)) = (effect.shader.as_deref(), effect.gpu.as_ref()) else { return None };
    let (id, revision) = (registered.spec.id, effect.revision);
    let g = e.g;
    let program = plan(ctx, Grid { width: b.img.width, height: b.img.height, offset: b.offset, scale: b.scale })?;
    let mut stage = Stage::new(id, revision, wgsl, &program);
    let prepared = padded_frame(&b, program.padding)
        .and_then(|frame| check(&program, frame, |w, h| g.fits(w, h)).map(|()| frame))
        .and_then(|frame| Ok((frame, compute_pipelines(g, id, revision, wgsl, &program)?)))
        .and_then(|prepared| stage.prepare(g).map(|()| prepared));
    let (frame, pipelines) = match prepared {
        Ok(prepared) => prepared,
        Err(why) => {
            g.plugin_declined(id, &why);
            return None;
        }
    };
    b.pad_sides(e, program.padding)?;
    let sizes: Vec<[u32; 2]> = [frame, frame]
        .into_iter()
        .chain(program.images.iter().copied())
        .chain(program.textures.iter().map(|texture| texture.size))
        .chain(program.layers.iter().map(|_| frame))
        .collect();
    let mut images: Vec<Option<GpuImage>> = vec![None; sizes.len()];
    *images.get_mut(SOURCE)? = Some(b.img.clone());
    for texture in 0..program.textures.len() {
        *images.get_mut(index(GpuSlot::Texture(texture), &program)?)? = Some(stage.texture(g, texture)?);
    }
    for (layer, param) in program.layers.iter().enumerate() {
        *images.get_mut(index(GpuSlot::Layer(layer), &program)?)? = Some(layer_image(e, ctx, &b, param)?);
    }
    if let Err(why) = record(e, &program, &pipelines, &mut stage, &mut images, &sizes) {
        g.plugin_declined(id, &why);
        return None;
    }
    b.img = images.get_mut(OUTPUT)?.take()?;
    log::debug!("gpu {}: plugin effect {id} ran {} passes at {}x{}", g.name, program.passes.len(), frame[0], frame[1]);
    Some(b)
}

fn padded_frame(b: &GBuf, padding: [u32; 4]) -> Result<[u32; 2], String> {
    let [left, top, right, bottom] = padding.map(f64::from);
    let (width, height) = (b.img.width, b.img.height);
    let budgeted = effectcraft_effects::grow_sides([-left, -top], [f64::from(width) + right, f64::from(height) + bottom], width, height).unwrap_or_default();
    let grown = |side: u32, a: u32, b: u32| side.checked_add(a)?.checked_add(b);
    match (grown(width, padding[0], padding[2]), grown(height, padding[1], padding[3])) {
        (Some(w), Some(h)) if budgeted == padding => Ok([w, h]),
        _ => Err("the frame could not be padded as far as the plan asks".into()),
    }
}

fn record(
    e: &mut Enc,
    program: &GpuProgram,
    pipelines: &[wgpu::ComputePipeline],
    stage: &mut Stage,
    images: &mut [Option<GpuImage>],
    sizes: &[[u32; 2]],
) -> Result<(), String> {
    let missing = || "the plan names an image or a kernel it does not have".to_string();
    let mut compute = pipelines.iter();
    for (n, pass) in program.passes.iter().enumerate() {
        let read = |slot: GpuSlot| index(slot, program).and_then(|at| images.get(at).cloned().flatten()).ok_or_else(missing);
        let src = read(pass.src)?;
        let aux = pass.aux.map(read).transpose()?;
        let target = index(pass.out, program).ok_or_else(missing)?;
        let [w, h] = *sizes.get(target).ok_or_else(missing)?;
        let out = match &pass.draw {
            None => {
                let pipeline = compute.next().ok_or_else(missing)?;
                let out = e.image(w, h);
                let images_bound = KernelImages { src: &src, aux: aux.as_ref(), out: &out };
                let data = (!pass.data.is_empty()).then(|| e.data(&pass.data));
                e.dispatch(Kernel::Plugin { pipeline, label: &pass.kernel }, &params(pass), images_bound, data.as_ref(), (w.div_ceil(16), h.div_ceil(16)));
                Some(out)
            }
            Some(_) => {
                let stays_open = program.passes.get(n + 1).is_some_and(|next| next.out == pass.out && next.draw.as_ref().is_some_and(|draw| !draw.clear));
                let below = images.get(target).cloned().flatten();
                stage.draw(e, pass, &params(pass), &src, aux.as_ref(), Target { index: target, size: [w, h], below: below.as_ref(), stays_open })?
            }
        };
        if let Some(out) = out {
            *images.get_mut(target).ok_or_else(missing)? = Some(out);
        }
    }
    Ok(())
}

fn compute_pipelines(g: &GpuContext, id: &str, revision: u64, wgsl: &str, program: &GpuProgram) -> Result<Vec<wgpu::ComputePipeline>, String> {
    let compute = || program.passes.iter().filter(|pass| pass.draw.is_none());
    let plain: Vec<&str> = compute().filter(|pass| pass.host.is_none()).map(|pass| pass.kernel.as_str()).collect();
    let mut plain = g.plugin_pipelines(id, revision, wgsl, &plain)?.into_iter();
    compute()
        .map(|pass| match &pass.host {
            Some(source) => g.host_pipeline(source, &pass.kernel),
            None => plain.next().ok_or_else(|| "a plugin kernel was not built".to_string()),
        })
        .collect()
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

fn check(program: &GpuProgram, frame: [u32; 2], fits: impl Fn(u32, u32) -> bool) -> Result<(), String> {
    if let Some((i, &[w, h])) = std::iter::once(&frame).chain(&program.images).enumerate().find(|&(_, &[w, h])| !fits(w, h)) {
        return Err(format!("image {i} of the plan is {w}x{h}, larger than this GPU holds"));
    }
    check_data(program, &fits)?;
    let mut written: Vec<usize> =
        std::iter::once(SOURCE).chain(2 + program.images.len()..2 + program.images.len() + program.textures.len() + program.layers.len()).collect();
    for (n, pass) in program.passes.iter().enumerate() {
        let read_before_written = |slot: GpuSlot| index(slot, program).is_none_or(|at| !written.contains(&at));
        if read_before_written(pass.src) || pass.aux.is_some_and(read_before_written) {
            return Err(format!("pass {n} reads an image the plan does not have or has not written yet"));
        }
        written.push(index(pass.out, program).ok_or_else(|| format!("pass {n} writes an image the plan does not have"))?);
        if let Some(draw) = &pass.draw {
            check_draw(n, draw, program, true)?;
        }
    }
    Ok(())
}
