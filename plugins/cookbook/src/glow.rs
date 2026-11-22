use craft_plugin::prelude::*;

params! {
    struct Settings {
        radius: f64 = slider("Radius", 24.0, 0.0..=400.0),
        amount: f64 = slider("Amount", 1.0, 0.0..=4.0),
    }
}

pub struct Glow;

impl Effect for Glow {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("glow", "Glow", "Cookbook").params(Settings::params()).shader("")
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        let radius = ((settings.radius * cx.scale) as f32).clamp(0.0, 2048.0);
        let padding = radius.ceil() as u32;
        let frame = cx.padded(padding).size;
        let quarter = frame.map(|side| side.div_ceil(4));
        let mut plan = Plan::new();
        let (small, blurred, glow) = (plan.scratch(quarter), plan.scratch(quarter), plan.scratch(frame));
        let plan = plan
            .pass(PlanPass::resample(PlanImage::Source, small, Filter::Bilinear))
            .pass(PlanPass::blur(small, blurred, [radius / 4.0; 2]))
            .pass(PlanPass::resample(blurred, glow, Filter::Bilinear))
            .pass(PlanPass::blend_images(PlanImage::Source, glow, PlanImage::Output, BlendMode::Screen, settings.amount as f32));
        Layout::whole_frame().padded(padding).passes(plan)
    }
}
