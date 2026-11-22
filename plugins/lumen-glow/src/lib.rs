mod blur;
mod gpu;
mod knee;
mod pixel;
mod plan;
mod profile;
mod settings;
mod spec;

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

}

fn plugin() -> Plugin {
    Plugin::new().effect(LumenGlow)
}

craft_plugin::export!(plugin);
