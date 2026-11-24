#[cfg(test)]
mod tests;

use craft_plugin::prelude::*;

params! {
    struct Settings {
        speed: f64 = slider("Speed", 0.5, 0.05..=4.0),
        method: usize = popup("Method", &["Motion vectors", "Frame blend", "Nearest frame"], 0),
    }
}

const SAME_FRAME: f64 = 1e-6;

pub struct TimeWarp;

impl Effect for TimeWarp {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("timewarp", "Time Warp", "Time").params(Settings::params()).time_dependent().reads_other_frames()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        let rate = cx.frame_rate.max(1.0);
        let frame = cx.time * settings.speed * rate;
        let (first, between) = (frame.floor(), frame - frame.floor());
        let mut plan = Plan::new();
        let output = PlanImage::Output;
        let scale = |amount: f32| [[amount, 0.0, 0.0, 0.0], [0.0, amount, 0.0, 0.0], [0.0, 0.0, amount, 0.0], [0.0, 0.0, 0.0, amount]];
        if settings.method == 2 || !(SAME_FRAME..=1.0 - SAME_FRAME).contains(&between) {
            let nearest = plan.source_at(frame.round() / rate);
            return Layout::whole_frame().passes(plan.pass(PlanPass::matrix(nearest, output, scale(1.0))));
        }
        let (before, after) = (plan.source_at(first / rate), plan.source_at((first + 1.0) / rate));
        let t = between as f32;
        let later = plan.scratch(cx.size);
        let plan = if settings.method == 1 {
            plan.pass(PlanPass::matrix(before, output, scale(1.0 - t))).pass(PlanPass::matrix(after, later, scale(t)))
        } else {
            let (forward, backward) = (plan.scratch(cx.size), plan.scratch(cx.size));
            plan.pass(PlanPass::flow(before, after, forward))
                .pass(PlanPass::flow(after, before, backward))
                .pass(PlanPass::warp(before, backward, output, [t, t]))
                .pass(PlanPass::matrix(output, output, scale(1.0 - t)))
                .pass(PlanPass::warp(after, forward, later, [1.0 - t, 1.0 - t]))
                .pass(PlanPass::matrix(later, later, scale(t)))
        };
        Layout::whole_frame().passes(plan.pass(PlanPass::blend_images(output, later, output, BlendMode::Add, 1.0)))
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(TimeWarp)
}

craft_plugin::export!(plugin);
