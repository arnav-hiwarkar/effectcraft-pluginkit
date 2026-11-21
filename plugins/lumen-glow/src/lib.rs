mod dither;
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
        Layout::whole_frame().padded(Settings::from_cx(cx).padding())
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
