mod dither;
mod gpu;
mod image;
mod knee;
mod passes;
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
            Some(plan) => layout.gpu(plan),
            None => layout,
        }
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        pipeline::render(&Settings::from_cx(cx), frame.pixels, frame.width);
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(LumenGlow)
}

craft_plugin::export!(plugin);
