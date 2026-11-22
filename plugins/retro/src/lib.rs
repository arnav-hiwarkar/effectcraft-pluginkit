mod crt;
#[cfg(test)]
mod golden;
mod grid;
mod math;
mod neon;

use craft_plugin::prelude::*;

const KERNELS: &str = include_str!("retro.wgsl");

struct CrtDisplay;

impl Effect for CrtDisplay {
    fn spec(&self) -> EffectSpec {
        crt::spec().shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = crt::Crt::from_cx(cx);
        Layout::neighbourhood(settings.reach()).gpu(crt::plan(&settings))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        crt::render(&crt::Crt::from_cx(cx), frame);
        Ok(())
    }
}

struct NeonOutline;

impl Effect for NeonOutline {
    fn spec(&self) -> EffectSpec {
        neon::spec().shader(KERNELS)
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = neon::Neon::from_cx(cx);
        Layout::neighbourhood(settings.reach()).gpu(neon::plan(&settings))
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        neon::render(&neon::Neon::from_cx(cx), frame);
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(CrtDisplay).effect(NeonOutline)
}

craft_plugin::export!(plugin);
