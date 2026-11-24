use craft_plugin::prelude::*;

params! {
    struct Settings {
        scale: f64 = slider("Scale", 64.0, 2.0..=1024.0),
        octaves: f64 = slider("Octaves", 4.0, 1.0..=8.0),
        seed: f64 = slider("Seed", 1.0, 0.0..=1000.0),
        drift: f64 = slider("Drift", 40.0, -500.0..=500.0),
        tint: [f32; 4] = color("Tint", [1.0, 1.0, 1.0, 1.0]),
    }
}

pub struct Field;

impl Effect for Field {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("field", "Noise Field", "Generate").params(Settings::params()).time_dependent()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        let [r, g, b, a] = settings.tint;
        let origin = [(settings.drift * cx.time * cx.scale) as i64, 0];
        let mut plan = Plan::new();
        let grey = plan.scratch(cx.size);
        let plan = plan
            .pass(PlanPass::noise(grey, (settings.scale * cx.scale) as f32, settings.octaves as u32, settings.seed as u32, origin))
            .pass(PlanPass::matrix(grey, PlanImage::Output, [[r, 0.0, 0.0, 0.0], [0.0, g, 0.0, 0.0], [0.0, 0.0, b, 0.0], [0.0, 0.0, 0.0, a]]));
        Layout::pointwise().ignores_input().passes(plan)
    }
}
