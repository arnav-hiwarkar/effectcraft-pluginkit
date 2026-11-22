use craft_plugin::prelude::*;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.55, 0.25, 1.0]),
    }
}

struct {{Ident}};

impl Effect for {{Ident}} {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("{{name}}", "{{Title}}", "Color Correction").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        let mix = (settings.amount / 100.0) as f32;
        for pixel in frame.pixels.iter_mut() {
            for (channel, tint) in pixel[..3].iter_mut().zip(settings.tint) {
                *channel += (*channel * tint - *channel) * mix;
            }
        }
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect({{Ident}})
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    #[test]
    fn full_amount_tints_the_test_image() {
        assert_frame!(TestPlugin::of(super::plugin), "{{name}}", { "amount": 100 }, "full");
    }

    #[test]
    fn zero_amount_leaves_every_pixel_as_it_was() {
        let plugin = TestPlugin::of(super::plugin);
        let render = plugin.render("{{name}}").set("amount", 0.0);
        assert_eq!(render.run().unwrap(), render.source());
    }
}
