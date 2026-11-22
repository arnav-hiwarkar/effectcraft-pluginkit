mod dither;
mod gpu;
mod knee;
mod passes;
#[cfg(test)]
mod pipeline;
mod pixel;
mod plan;
mod profile;
mod settings;
mod spec;
mod transfer;

use craft_plugin::prelude::*;

use crate::settings::Settings;

struct LumenGlow;

impl Effect for LumenGlow {
    fn spec(&self) -> EffectSpec {
        spec::spec()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::from_cx(cx);
        let padding = settings.frame_padding(cx.size);
        let layout = Layout::whole_frame().padded(padding);
        match gpu::plan(&settings, cx.size, padding) {
            Some(plan) => layout.passes(plan),
            None => layout,
        }
    }

    fn pass(&self, cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
        passes::run(&Settings::from_cx(cx), pass, out)
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        if frame.pixels.is_empty() {
            return Ok(());
        }
        Err("Lumen Glow renders through its passes".into())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(LumenGlow)
}

craft_plugin::export!(plugin);
