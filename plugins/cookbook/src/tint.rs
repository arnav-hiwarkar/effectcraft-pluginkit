use craft_plugin::prelude::*;

params! {
    struct Settings {
        amount: f64 = percent("Amount", 100.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.55, 0.25, 1.0]),
    }
}

pub struct Tint;

impl Effect for Tint {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("tint", "Tint", "Cookbook")
            .params(Settings::params())
            .preset("Cold", [("tint", Value::List(vec![0.4, 0.7, 1.0, 1.0])), ("amount", Value::Number(80.0))])
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
