use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;

use craft_plugin::{
    Call, Contributions, FrameDesc, GpuImage, GpuPass, GpuPlan, Layout, LogLevel, Manifest, Outcome, Permission, Pixel, Rect, RenderCx, TaskState, Window,
};
use rayon::prelude::*;
use serde::de::DeserializeOwned;

use crate::host::{Host, Origin};
use crate::journal::{LogLine, LogPage};
use crate::kernels::{self, Image, Step};
use crate::registry::Bundle;
use crate::runtime::Engine;

const MIN_BAND_ROWS: usize = 64;
const STAGED_BYTES: usize = 1 << 30;

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub fn validate(manifest: &Manifest, contributions: &Contributions) -> Result<(), String> {
    if manifest.id.starts_with("ec.") {
        return Err("plugin ids starting with `ec.` are reserved for built-in effects".into());
    }
    match contributions.effects.iter().find(|e| !plain_id(&e.id)) {
        Some(effect) => Err(format!("effect id `{}` may only use lower case letters, digits, `-` and `_`", effect.id)),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error("the plugin could not be loaded: {0}")]
    Load(String),
    #[error("the plugin stopped: {0}")]
    Trap(String),
    #[error("{0}")]
    Failed(String),
    #[error("the plugin sent a reply this app does not understand: {0}")]
    Protocol(String),
}

#[cfg_attr(target_family = "wasm", expect(dead_code, reason = "no engine reads pixels in the web build"))]
pub(crate) enum Io<'a> {
    None,
    InPlace(&'a mut [Pixel]),
    Region { input: &'a [Pixel], output: &'a mut [Pixel], skip: usize },
    Render { input: &'a [Pixel], out: &'a mut [Pixel] },
    Pass { src: &'a [Pixel], aux: &'a [Pixel], out: &'a mut [Pixel] },
}

impl Image<'_> {
    fn window(&self, band: Window, reach: Option<u32>) -> Window {
        let (height, out_height) = (u64::from(self.size[1]), u64::from(band.size[1]).max(1));
        let Some(reach) = reach.map(u64::from) else { return Window { size: self.size, first_row: 0, rows: self.size[1] } };
        let first = (u64::from(band.first_row) * height / out_height).saturating_sub(reach).min(height);
        let end = ((u64::from(band.first_row) + u64::from(band.rows)) * height).div_ceil(out_height).saturating_add(reach).min(height);
        Window { size: self.size, first_row: first as u32, rows: end.saturating_sub(first) as u32 }
    }

    fn held(&self, window: Window) -> &[Pixel] {
        let width = self.size[0] as usize;
        &self.pixels[window.first_row as usize * width..(window.first_row + window.rows) as usize * width]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub interactive: Duration,
    pub work: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { interactive: Duration::from_secs(2), work: Duration::from_secs(60) }
    }
}

pub struct Plugin {
    manifest: Manifest,
    engine: Arc<Engine>,
    limits: Limits,
    contributions: Contributions,
}

struct NoHost;
impl Host for NoHost {}

impl Plugin {
    pub fn load(bundle: &Bundle, grants: BTreeSet<Permission>, data_dir: PathBuf, limits: Limits) -> Result<Plugin, PluginError> {
        let engine = Engine::open(bundle, grants, data_dir)?;
        Plugin::described(bundle.manifest.clone(), engine, limits)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn in_process(manifest: Manifest, plugin: craft_plugin::Plugin, dir: PathBuf, data_dir: PathBuf, limits: Limits) -> Result<Plugin, PluginError> {
        let engine = Engine::in_process(plugin, &manifest.id, dir, manifest.permissions.clone(), data_dir);
        Plugin::described(manifest, engine, limits)
    }

    fn described(manifest: Manifest, engine: Arc<Engine>, limits: Limits) -> Result<Plugin, PluginError> {
        let mut plugin = Plugin { manifest, engine, limits, contributions: Contributions::default() };
        plugin.contributions = plugin.call(&Call::Describe, None, &mut NoHost)?;
        if let Some(error) = plugin.contributions.effects.iter().find_map(|effect| effect.check().err()) {
            return Err(PluginError::Load(error));
        }
        plugin.engine.know_tasks(plugin.contributions.tasks.iter().map(|task| task.id.clone()).collect());
        Ok(plugin)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn cached(&self, key: &str) -> Option<std::sync::Arc<[u8]>> {
        self.engine.cached(key)
    }

    pub fn image(&self, key: &str) -> Option<std::sync::Arc<crate::images::PluginImage>> {
        self.engine.image(key)
    }

    pub fn cache_generation(&self) -> u64 {
        self.engine.cache_generation()
    }

    pub fn uncache(&self, key: &str) {
        self.engine.uncache(key);
    }

    pub fn contributions(&self) -> &Contributions {
        &self.contributions
    }

    pub fn call<R: DeserializeOwned>(&self, call: &Call, pixels: Option<&mut [Pixel]>, host: &mut dyn Host) -> Result<R, PluginError> {
        if matches!(call, Call::Render { .. }) {
            return Err(PluginError::Failed("a render call stages its frame through Plugin::render; call render".into()));
        }
        let io = pixels.map_or(Io::None, Io::InPlace);
        self.exchange(call, io, host)
    }

    pub fn render<H: Host>(
        &self,
        effect: &str,
        cx: &RenderCx,
        layout: &Layout,
        area: Option<Rect>,
        pixels: &mut Vec<Pixel>,
        make_host: impl Fn() -> H + Sync,
    ) -> Result<Rect, PluginError> {
        let (width, height) = (cx.size[0] as usize, cx.size[1] as usize);
        if cx.size.iter().any(|&side| i32::try_from(side).is_err()) {
            return Err(PluginError::Failed(format!(
                "a {width}x{height} frame is larger than a plugin can address; each side must be under {} pixels",
                1u64 << 31
            )));
        }
        if pixels.len() != width.saturating_mul(height) {
            return Err(PluginError::Protocol(format!("{} pixels for a {width}x{height} frame", pixels.len())));
        }
        let full = Rect::of_size(cx.size);
        let separate = self.contributions.separate_input;
        let mut area = area.map_or(full, |area| area.clipped(cx.size));
        let pointwise = layout.reach == Some(0) && !(separate && layout.maps_input);
        if !separate {
            (area.left, area.right) = (full.left, full.right);
        }
        if pixels.is_empty() || area.is_empty() {
            *pixels = Vec::new();
            return Ok(area);
        }
        let (rows, out_width) = (area.height() as usize, area.width() as usize);
        let bands = (rows / MIN_BAND_ROWS).clamp(1, rayon::current_num_threads());
        let even_rows = rows.div_ceil(bands);
        if pointwise && out_width == width {
            let (top, bottom) = (area.top as usize, area.bottom as usize);
            let call = |band: usize, rows: &[Pixel]| {
                let frame =
                    FrameDesc { width: cx.size[0], height: (rows.len() / width) as u32, origin: [0, (top + band * even_rows) as u32], ..Default::default() };
                Call::Render { effect: effect.into(), cx: cx.clone(), frame }
            };
            let shown = &mut pixels[top * width..bottom * width];
            shown
                .par_chunks_mut(even_rows * width)
                .enumerate()
                .try_for_each(|(band, rows)| self.exchange::<()>(&call(band, rows), Io::InPlace(rows), &mut make_host()))?;
            pixels.truncate(bottom * width);
            pixels.drain(..top * width);
            return Ok(area);
        }
        let reach = layout.reach.map(|reach| reach as usize);
        let band_rows = match reach {
            _ if separate && layout.maps_input => even_rows,
            Some(reach) if separate && self.engine.copies_pixels() => copied_band_rows(area, height, width, reach, bands, STAGED_BYTES),
            Some(_) if separate => even_rows,
            Some(reach) if even_rows.max(reach + 1) + 2 * reach < rows => even_rows.max(reach + 1),
            _ => rows,
        };
        let input: &[Pixel] = pixels;
        let mut output = match pointwise {
            true => {
                input.chunks_exact(width).skip(area.top as usize).take(rows).flat_map(|row| &row[area.left as usize..area.right as usize]).copied().collect()
            }
            false => vec![[0.0; 4]; out_width * rows],
        };
        output.par_chunks_mut(band_rows * out_width).enumerate().try_for_each(|(band, written)| {
            let top = area.top + (band * band_rows) as i32;
            let out = Rect { top, bottom: top + (written.len() / out_width) as i32, ..area };
            let needed = match (layout.maps_input && separate, layout.reach) {
                (true, _) => self.exchange::<Rect>(&Call::Input { effect: effect.into(), cx: cx.clone(), out }, Io::None, &mut make_host())?,
                (false, Some(reach)) => out.grown(reach),
                (false, None) => full,
            };
            let needed = needed.union(out).clipped(cx.size);
            let (first, last) = (needed.top as usize, needed.bottom as usize);
            let held = &input[first * width..last * width];
            if !separate {
                let frame = FrameDesc { width: cx.size[0], height: (last - first) as u32, origin: [0, first as u32], ..Default::default() };
                let call = Call::Render { effect: effect.into(), cx: cx.clone(), frame };
                let io = Io::Region { input: held, output: written, skip: (top as usize - first) * width };
                return self.exchange::<()>(&call, io, &mut make_host());
            }
            let spread = !self.engine.copies_pixels();
            let window = Window { size: cx.size, first_row: first as u32, rows: (last - first) as u32 };
            let origin = [area.left as u32, top as u32];
            let frame = FrameDesc { width: out_width as u32, height: (written.len() / out_width) as u32, origin, input: Some(window), spread };
            let call = Call::Render { effect: effect.into(), cx: cx.clone(), frame };
            if spread {
                let address = |pixels: &[Pixel]| [pixels.as_ptr() as u64, pixels.len() as u64];
                let mut table: [Pixel; 3] = bytemuck::cast([address(held), address(&[]), [written.as_mut_ptr() as u64, written.len() as u64]]);
                self.exchange::<()>(&call, Io::InPlace(&mut table), &mut make_host())
            } else {
                self.exchange::<()>(&call, Io::Render { input: held, out: written }, &mut make_host())
            }
        })?;
        *pixels = output;
        Ok(area)
    }

    pub fn accept_drop(&self, path: &std::path::Path) -> Result<(), String> {
        self.engine.accept_drop(path)
    }

    pub fn start(&self, task: &str, args: serde_json::Value, host: &dyn Host) -> Result<u64, PluginError> {
        self.engine.start(task, args, host.project_closed()).map_err(PluginError::Failed)
    }

    pub fn tasks(&self) -> Vec<TaskState> {
        self.engine.tasks()
    }

    pub fn running_tasks(&self) -> usize {
        self.engine.running_tasks()
    }

    pub fn cancel(&self, id: u64) -> bool {
        self.engine.cancel(id)
    }

    pub fn cancel_all(&self) {
        self.engine.cancel_all();
    }

    pub fn pump(&self, host: &mut dyn Host) -> usize {
        self.engine.pump(host)
    }

    pub fn run_passes<H: Host>(
        &self,
        effect: &str,
        cx: &RenderCx,
        plan: &GpuPlan,
        pixels: &mut Vec<Pixel>,
        make_host: impl Fn() -> H + Sync,
    ) -> Result<(), PluginError> {
        let area = |size: [u32; 2]| (size[0] as usize).saturating_mul(size[1] as usize);
        if pixels.len() != area(cx.size) {
            return Err(PluginError::Protocol(format!("{} pixels for a {}x{} frame", pixels.len(), cx.size[0], cx.size[1])));
        }
        if pixels.is_empty() {
            return Ok(());
        }
        plan.check(cx.size).map_err(PluginError::Failed)?;
        let sizes: Vec<[u32; 2]> = [cx.size].into_iter().chain(plan.images.iter().copied()).collect();
        let mut written: Vec<Vec<Pixel>> = sizes.iter().map(|size| vec![[0.0; 4]; area(*size)]).collect();
        let slot = |image: GpuImage| match image {
            GpuImage::Source => None,
            GpuImage::Output => Some(0),
            GpuImage::Scratch(index) => Some(index as usize + 1),
            GpuImage::Texture(_) | GpuImage::Layer(_) => None,
        };
        for step in kernels::steps(plan, cx.size) {
            let out_image = match &step {
                Step::Single(index) => plan.passes[*index].out,
                Step::Fused(fused) => fused.out,
            };
            let Some(target) = slot(out_image) else { continue };
            let reads_target = matches!(&step, Step::Fused(fused) if fused.inputs.contains(&out_image));
            let mut out = if reads_target { vec![[0.0; 4]; area(sizes[target])] } else { std::mem::take(&mut written[target]) };
            let read = |image: GpuImage| match slot(image) {
                None => Image { size: cx.size, pixels },
                Some(index) => Image { size: sizes[index], pixels: &written[index] },
            };
            let ran = match &step {
                Step::Fused(fused) => {
                    let inputs: Vec<Image> = fused.inputs.iter().map(|image| read(*image)).collect();
                    kernels::run_fused(fused, &inputs, sizes[target][0] as usize, &mut out);
                    Ok(())
                }
                Step::Single(index) => {
                    let pass = &plan.passes[*index];
                    match &pass.host {
                        Some(kernel) => {
                            kernels::run(kernel, read(pass.src), pass.aux.map(read), sizes[target], &mut out);
                            Ok(())
                        }
                        None => self.run_pass(effect, cx, pass, read(pass.src), pass.aux.map(read), sizes[target], &mut out, &make_host),
                    }
                }
            };
            written[target] = out;
            ran?;
        }
        *pixels = std::mem::take(&mut written[0]);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one pass: its plan entry, the images it reads and the image it writes")]
    fn run_pass<H: Host>(
        &self,
        effect: &str,
        cx: &RenderCx,
        pass: &GpuPass,
        src: Image,
        aux: Option<Image>,
        size: [u32; 2],
        out: &mut [Pixel],
        make_host: &(impl Fn() -> H + Sync),
    ) -> Result<(), PluginError> {
        let (width, height) = (size[0] as usize, size[1] as usize);
        let spread = !self.engine.copies_pixels();
        let bands = if pass.reach.is_some() { (height / MIN_BAND_ROWS).clamp(1, rayon::current_num_threads()) } else { 1 };
        let band_rows = height.div_ceil(bands);
        out.par_chunks_mut(band_rows * width).enumerate().try_for_each(|(index, rows)| {
            let band = Window { size, first_row: (index * band_rows) as u32, rows: (rows.len() / width) as u32 };
            let src_window = src.window(band, pass.reach);
            let aux_window = aux.as_ref().map(|aux| aux.window(band, pass.aux_reach.or(pass.reach)));
            let (src_held, aux_held) = (src.held(src_window), aux.as_ref().zip(aux_window).map_or(&[][..], |(aux, window)| aux.held(window)));
            let call = Call::Pass {
                effect: effect.into(),
                cx: cx.clone(),
                kernel: pass.kernel.clone(),
                f: pass.f.clone(),
                u: pass.u.clone(),
                src: src_window,
                aux: aux_window,
                out: band,
                spread,
            };
            if spread {
                let address = |pixels: &[Pixel]| [pixels.as_ptr() as u64, pixels.len() as u64];
                let mut table: [Pixel; 3] = bytemuck::cast([address(src_held), address(aux_held), [rows.as_mut_ptr() as u64, rows.len() as u64]]);
                self.exchange::<()>(&call, Io::InPlace(&mut table), &mut make_host())
            } else {
                self.exchange::<()>(&call, Io::Pass { src: src_held, aux: aux_held, out: rows }, &mut make_host())
            }
        })
    }

    fn budget(&self, call: &Call) -> Duration {
        match call {
            Call::Describe
            | Call::Layout { .. }
            | Call::Input { .. }
            | Call::Panel { .. }
            | Call::Controls { .. }
            | Call::Overlay { .. }
            | Call::Drag { .. }
            | Call::Tool { .. } => self.limits.interactive,
            Call::Render { .. } | Call::Pass { .. } | Call::Command { .. } | Call::Task { .. } => self.limits.work,
        }
    }

    pub fn logs(&self, after: u64) -> Vec<LogLine> {
        self.engine.journal().page(after, None).lines
    }

    pub fn log_page(&self, after: u64, load: Option<u64>) -> LogPage {
        self.engine.journal().page(after, load)
    }

    fn exchange<R: DeserializeOwned>(&self, call: &Call, io: Io, host: &mut dyn Host) -> Result<R, PluginError> {
        let answered = self.answer(call, io, host);
        if let Err(error) = &answered {
            self.engine.journal().push(LogLevel::Error, format!("{} failed: {error}", what(call)));
        }
        answered
    }

    fn answer<R: DeserializeOwned>(&self, call: &Call, io: Io, host: &mut dyn Host) -> Result<R, PluginError> {
        let request = serde_json::to_vec(call).map_err(|e| PluginError::Protocol(e.to_string()))?;
        let reply = self.engine.call(&request, io, host, origin(call), self.budget(call))?;
        let outcome: Outcome<R> = serde_json::from_slice(&reply).map_err(|e| PluginError::Protocol(e.to_string()))?;
        Result::from(outcome).map_err(PluginError::Failed)
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.engine.cancel_all();
    }
}

fn copied_band_rows(area: Rect, height: usize, width: usize, reach: usize, bands: usize, budget: usize) -> usize {
    let (rows, top, bottom) = (area.height() as usize, area.top as usize, area.bottom as usize);
    let staged = |band_rows: usize| {
        (0..rows).step_by(band_rows).fold((rows * area.width() as usize, true), |(pixels, covers), start| {
            let first = (top + start).saturating_sub(reach);
            let last = (top + start + band_rows).min(bottom).saturating_add(reach).min(height);
            (pixels + (last - first) * width, covers && last - first == height)
        })
    };
    (1..=bands)
        .rev()
        .map(|bands| rows.div_ceil(bands))
        .find(|&band_rows| matches!(staged(band_rows), (pixels, false) if pixels.saturating_mul(size_of::<Pixel>()) <= budget))
        .unwrap_or(rows)
}

fn what(call: &Call) -> String {
    match call {
        Call::Describe => "describing the plugin".into(),
        Call::Layout { effect, .. } => format!("layout of effect `{effect}`"),
        Call::Render { effect, .. } => format!("render of effect `{effect}`"),
        Call::Input { effect, .. } => format!("input area of effect `{effect}`"),
        Call::Pass { effect, kernel, .. } => format!("pass `{kernel}` of effect `{effect}`"),
        Call::Panel { panel, .. } => format!("panel `{panel}`"),
        Call::Command { command } => format!("command `{command}`"),
        Call::Task { task, .. } => format!("task `{task}`"),
        Call::Controls { effect, param, .. } => format!("controls of `{param}` in effect `{effect}`"),
        Call::Overlay { effect, .. } => format!("overlay of effect `{effect}`"),
        Call::Drag { effect, key, .. } => format!("drag of handle `{key}` in effect `{effect}`"),
        Call::Tool { tool, .. } => format!("tool `{tool}`"),
    }
}

fn origin(call: &Call) -> Origin {
    match call {
        Call::Describe | Call::Panel { .. } | Call::Controls { .. } | Call::Command { .. } | Call::Overlay { .. } | Call::Drag { .. } | Call::Tool { .. } => {
            Origin::Interactive
        }
        Call::Layout { .. } | Call::Input { .. } | Call::Render { .. } | Call::Pass { .. } => Origin::Render,
        Call::Task { .. } => Origin::Task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandboxed_render_uses_one_band_when_its_reach_covers_the_frame_and_bounds_what_its_bands_stage() {
        let frame = Rect::of_size([64, 256]);
        assert_eq!(copied_band_rows(frame, 256, 64, 1000, 4, usize::MAX), 256, "a reach over the frame");
        assert_eq!(copied_band_rows(frame, 256, 64, 8, 4, usize::MAX), 64, "a short reach");
        assert_eq!(copied_band_rows(frame, 256, 64, 100, 4, 1_064_960), 64, "four bands stage 1,064,960 bytes");
        assert_eq!(copied_band_rows(frame, 256, 64, 100, 4, 800_000), 128, "two bands stage 729,088 bytes");
        assert_eq!(copied_band_rows(frame, 256, 64, 100, 4, 1000), 256, "nothing fits: one band");
    }
}
