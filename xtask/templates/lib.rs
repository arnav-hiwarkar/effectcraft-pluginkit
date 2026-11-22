use craft_plugin::prelude::*;
{{kernels}}
struct {{Ident}}Effect;

impl Effect for {{Ident}}Effect {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("{{name}}", "{{Title}}", "Color Correction").param(Param::percent("amount", "Amount", 100.0)){{shader}}
    }

    fn layout(&self, {{cx}}: &Cx) -> Layout {
{{layout}}
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let gain = (cx.f("amount") / 100.0) as f32;
        for pixel in frame.pixels.iter_mut() {
            for channel in &mut pixel[..3] {
                *channel *= gain;
            }
        }
        Ok(())
    }
}
{{panel}}
fn plugin() -> Plugin {
    Plugin::new().effect({{Ident}}Effect){{registration}}
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use craft_plugin::{RenderCx, Value};

    use super::*;

    #[test]
    fn half_amount_halves_the_colour_and_keeps_the_alpha() {
        let params = BTreeMap::from([("amount".to_string(), Value::Number(50.0))]);
        let cx = Cx::new(RenderCx { params, size: [2, 1], scale: 1.0, ..Default::default() });
        let mut pixels = vec![[0.5, 0.25, 1.0, 1.0], [0.0, 0.5, 0.5, 0.5]];
        let mut frame = Frame { width: 2, height: 1, origin: [0, 0], pixels: &mut pixels };
        {{Ident}}Effect.render(&cx, &mut frame).unwrap();
        assert_eq!(pixels, [[0.25, 0.125, 0.5, 1.0], [0.0, 0.25, 0.25, 0.5]]);
    }
}
