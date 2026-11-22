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
        Layout::whole_frame().passes(crt::plan(&crt::Crt::from_cx(cx)))
    }

    fn pass(&self, cx: &Cx, pass: &Pass, out: &mut Frame) -> Result<()> {
        let crt = crt::Crt::from_cx(cx);
        match pass.kernel {
            "retro_crt_emit" => crt::emit(&crt, pass, out),
            "retro_crt" => return crt::display(&crt, pass, out),
            other => return Err(format!("no kernel `{other}`").into()),
        }
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

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn the_crt_with_a_curved_screen_matches_its_stored_frame() {
        let plugin = TestPlugin::of(super::plugin);
        assert_frame!(plugin.render("crt").set("Screen/curvature", 0.1).set("Signal/noise", 0.0), "crt-default");
    }

    #[test]
    fn the_crt_with_a_slot_mask_and_a_wide_glow_matches_its_stored_frame() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("crt").set("Mask/mask", 1.0).set("Phosphor Glow/glow_radius", 45.0).set("Signal/noise", 0.0);
        assert_frame!(render, "crt-slot-wide-glow");
    }

    #[test]
    fn the_neon_outline_matches_its_stored_frame() {
        assert_frame!(TestPlugin::of(super::plugin), "neon", { "Bloom/intensity": 3 }, "neon");
    }
}
