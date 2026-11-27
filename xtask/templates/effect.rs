use craft_plugin::prelude::*;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.55, 0.25, 1.0]),
    }
}

struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("{{name}}", "{{Title}}", "Color Correction").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        let mix = (settings.amount / 100.0) as f32;
        let gain = settings.tint.map(|tint| 1.0 + (tint - 1.0) * mix);
        frame.map(|[r, g, b, a]| [r * gain[0], g * gain[1], b * gain[2], a]);
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Tint)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn full_amount_tints_the_test_image() {
        let plugin = TestPlugin::of(super::plugin);
        assert_frame!(plugin.render("{{name}}").set("amount", 100.0), "full");
    }

    #[test]
    fn zero_amount_leaves_every_pixel_as_it_was() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("{{name}}").set("amount", 0.0);
        assert!(render.run().unwrap() == render.source(), "an amount of 0 changed the picture");
    }
}
