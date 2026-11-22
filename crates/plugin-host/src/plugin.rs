use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;

use craft_plugin::{Call, Contributions, FrameDesc, GpuImage, GpuPass, GpuPlan, Manifest, Outcome, Permission, Pixel, RenderCx, TaskState, Window};
use rayon::prelude::*;
use serde::de::DeserializeOwned;

use crate::host::{Host, Origin};
use crate::kernels::{self, Image};
use crate::registry::Bundle;
use crate::runtime::Engine;

const MIN_BAND_ROWS: usize = 64;

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
    Pass { src: &'a [Pixel], aux: &'a [Pixel], out: &'a mut [Pixel] },
}

impl Image<'_> {
    fn window(&self, band: Window, reach: Option<u32>) -> Window {
        let (height, out_height) = (u64::from(self.size[1]), u64::from(band.size[1]).max(1));
        let Some(reach) = reach.map(u64::from) else { return Window { size: self.size, first_row: 0, rows: self.size[1] } };
        let matching = |row: u32| u64::from(row) * height / out_height;
        let first = matching(band.first_row).saturating_sub(reach).min(height);
        let end = (matching(band.first_row + band.rows.saturating_sub(1)) + reach + 1).min(height);
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
        let mut plugin = Plugin { manifest: bundle.manifest.clone(), engine, limits, contributions: Contributions::default() };
        plugin.contributions = plugin.call(&Call::Describe, None, &mut NoHost)?;
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
        reach: Option<u32>,
        pixels: &mut Vec<Pixel>,
        make_host: impl Fn() -> H + Sync,
    ) -> Result<(), PluginError> {
        let (width, height) = (cx.size[0] as usize, cx.size[1] as usize);
        if pixels.len() != width * height {
            return Err(PluginError::Protocol(format!("{} pixels for a {width}x{height} frame", pixels.len())));
        }
        if pixels.is_empty() {
            return Ok(());
        }
        let even_rows = height.div_ceil((height / MIN_BAND_ROWS).clamp(1, rayon::current_num_threads()));
        let (band_rows, reach) = match reach.map(|reach| reach as usize) {
            Some(reach) if even_rows.max(reach + 1) + 2 * reach < height => (even_rows.max(reach + 1), reach),
            _ => (height, 0),
        };
        let input: &[Pixel] = pixels;
        let mut output = vec![[0.0; 4]; input.len()];
        output.par_chunks_mut(band_rows * width).enumerate().try_for_each(|(band, rows)| {
            let first = band * band_rows;
            let (top, bottom) = (first.saturating_sub(reach), (first + rows.len() / width + reach).min(height));
            let frame = FrameDesc { width: cx.size[0], height: (bottom - top) as u32, origin: [0, top as u32] };
            let call = Call::Render { effect: effect.into(), cx: cx.clone(), frame };
            let io = Io::Region { input: &input[top * width..bottom * width], output: rows, skip: (first - top) * width };
            self.exchange::<()>(&call, io, &mut make_host())
        })?;
        *pixels = output;
        Ok(())
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
        let area = |size: [u32; 2]| size[0] as usize * size[1] as usize;
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
        for pass in &plan.passes {
            let Some(target) = slot(pass.out) else { continue };
            let mut out = std::mem::take(&mut written[target]);
            let read = |image: GpuImage| match slot(image) {
                None => Image { size: cx.size, pixels },
                Some(index) => Image { size: sizes[index], pixels: &written[index] },
            };
            let ran = match &pass.host {
                Some(kernel) => {
                    kernels::run(kernel, read(pass.src), pass.aux.map(read), sizes[target], &mut out);
                    Ok(())
                }
                None => self.run_pass(effect, cx, pass, read(pass.src), pass.aux.map(read), sizes[target], &mut out, &make_host),
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
            let reach = pass.reach.filter(|_| !spread);
            let src_window = src.window(band, reach);
            let aux_window = aux.as_ref().map(|aux| aux.window(band, pass.aux_reach.or(reach).filter(|_| !spread)));
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
            Call::Describe | Call::Layout { .. } | Call::Panel { .. } | Call::Controls { .. } => self.limits.interactive,
            Call::Render { .. } | Call::Pass { .. } | Call::Command { .. } | Call::Task { .. } => self.limits.work,
        }
    }

    fn exchange<R: DeserializeOwned>(&self, call: &Call, io: Io, host: &mut dyn Host) -> Result<R, PluginError> {
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

fn origin(call: &Call) -> Origin {
    match call {
        Call::Describe | Call::Panel { .. } | Call::Controls { .. } | Call::Command { .. } => Origin::Interactive,
        Call::Layout { .. } | Call::Render { .. } | Call::Pass { .. } => Origin::Render,
        Call::Task { .. } => Origin::Task,
    }
}
