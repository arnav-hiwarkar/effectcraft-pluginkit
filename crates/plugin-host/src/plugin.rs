use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use craft_plugin::{Call, Contributions, FrameDesc, Manifest, Outcome, Permission, Pixel, RenderCx};
use rayon::prelude::*;
use serde::de::DeserializeOwned;

use crate::host::Host;
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
    engine: Engine,
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
        Ok(plugin)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn contributions(&self) -> &Contributions {
        &self.contributions
    }

    pub fn call<R: DeserializeOwned>(&self, call: &Call, pixels: Option<&mut [Pixel]>, host: &mut dyn Host) -> Result<R, PluginError> {
        let io = pixels.map_or(Io::None, Io::InPlace);
        self.exchange(call, io, host)
    }

    pub fn render<H: Host>(
        &self,
        effect: &str,
        cx: &RenderCx,
        reach: Option<u32>,
        pixels: &mut [Pixel],
        make_host: impl Fn() -> H + Sync,
    ) -> Result<(), PluginError> {
        let (width, height) = (cx.size[0] as usize, cx.size[1] as usize);
        if pixels.len() != width * height {
            return Err(PluginError::Protocol(format!("{} pixels for a {width}x{height} frame", pixels.len())));
        }
        let bands = (height / MIN_BAND_ROWS).min(rayon::current_num_threads());
        let (Some(reach), true) = (reach, bands > 1 && self.engine.copies_pixels()) else {
            let frame = FrameDesc { width: cx.size[0], height: cx.size[1], origin: [0, 0] };
            let call = Call::Render { effect: effect.into(), cx: cx.clone(), frame };
            return self.call(&call, Some(pixels), &mut make_host());
        };
        let band_rows = height.div_ceil(bands);
        let rendered: Vec<Vec<Pixel>> = (0..bands)
            .into_par_iter()
            .map(|band| {
                let (first, last) = (band * band_rows, ((band + 1) * band_rows).min(height));
                let top = first.saturating_sub(reach as usize);
                let bottom = (last + reach as usize).min(height);
                let frame = FrameDesc { width: cx.size[0], height: (bottom - top) as u32, origin: [0, top as u32] };
                let call = Call::Render { effect: effect.into(), cx: cx.clone(), frame };
                let mut core = vec![[0.0; 4]; (last - first) * width];
                let io = Io::Region { input: &pixels[top * width..bottom * width], output: &mut core, skip: (first - top) * width };
                self.exchange::<()>(&call, io, &mut make_host()).map(|()| core)
            })
            .collect::<Result<_, _>>()?;
        pixels.par_chunks_mut(band_rows * width).zip(rendered).for_each(|(rows, core)| rows.copy_from_slice(&core));
        Ok(())
    }

    fn budget(&self, call: &Call) -> Duration {
        match call {
            Call::Describe | Call::Layout { .. } | Call::Panel { .. } => self.limits.interactive,
            Call::Render { .. } | Call::Command { .. } => self.limits.work,
        }
    }

    fn exchange<R: DeserializeOwned>(&self, call: &Call, io: Io, host: &mut dyn Host) -> Result<R, PluginError> {
        let request = serde_json::to_vec(call).map_err(|e| PluginError::Protocol(e.to_string()))?;
        let reply = self.engine.call(&request, io, host, stateful(call), self.budget(call))?;
        let outcome: Outcome<R> = serde_json::from_slice(&reply).map_err(|e| PluginError::Protocol(e.to_string()))?;
        Result::from(outcome).map_err(PluginError::Failed)
    }
}

fn stateful(call: &Call) -> bool {
    matches!(call, Call::Describe | Call::Panel { .. } | Call::Command { .. })
}
