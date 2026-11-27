use craft_plugin::prelude::*;

params! {
    struct Settings {
        gain: f64 = slider("Gain", 1.0, 0.0..=2.0),
    }
}

struct Gain;

impl Effect for Gain {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gain", "Gain", "Color Correction").params(Settings::params())
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        // cx.f("old") was the parameter before it was renamed to gain
        let gain = Settings::read(cx).gain as f32;
        frame.map(|[r, g, b, a]| [r * gain, g * gain, b * gain, a]);
        Ok(())
    }
}

struct Shouting;

impl Effect for Shouting {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("Bad", "Shouting", "Stylize")
    }
}

struct Broken;

impl Effect for Broken {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("broken", "Broken", "Stylize")
    }

    fn render(&self, _cx: &Cx, _frame: &mut Frame) -> Result<()> {
        Err("the lookup table is empty".into())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Gain).effect(Shouting).effect(Broken)
}

craft_plugin::export!(plugin);
