mod control;
mod curve;
mod presets;
#[cfg(test)]
mod tests;

use std::sync::Mutex;

use craft_plugin::prelude::*;

use crate::control::ControlState;
use crate::curve::{Curves, Luts};

pub const EFFECT: &str = "curves";
pub const PARAM: &str = "curves";

#[derive(Default)]
struct CurvesEffect {
    control: Mutex<ControlState>,
}

impl Effect for CurvesEffect {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new(EFFECT, "Curves", "Color Correction").param(Param::custom(PARAM, "Curves", &Curves::default_text()))
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let luts = Luts::new(&Curves::parse(cx.text(PARAM)));
        frame.pixels.iter_mut().for_each(|pixel| *pixel = luts.pixel(*pixel));
        Ok(())
    }

    fn controls(&self, param: &str, value: &mut String, ui: &mut Ui) {
        if param != PARAM {
            return;
        }
        let mut state = self.control.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        control::draw(&mut state, value, ui);
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(CurvesEffect::default()).panel("presets", "Curve Presets", presets::PresetPanel::default())
}

craft_plugin::export!(plugin);
