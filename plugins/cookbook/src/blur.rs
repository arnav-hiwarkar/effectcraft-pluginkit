use craft_plugin::prelude::*;

params! {
    struct Settings {
        radius: f64 = slider("Radius", 8.0, 0.0..=200.0),
    }
}

pub struct Blur;

impl Effect for Blur {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("blur", "Blur", "Cookbook").params(Settings::params()).shader("")
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let radius = (Settings::read(cx).radius * cx.scale) as f32;
        let plan = GpuPlan::new().pass(GpuPass::blur(GpuImage::Source, GpuImage::Output, [radius; 2]));
        Layout::whole_frame().passes(plan)
    }
}
