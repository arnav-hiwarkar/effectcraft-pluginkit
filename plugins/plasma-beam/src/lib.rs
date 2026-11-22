mod beam;
mod glow;
#[cfg(test)]
mod golden;
mod image;
mod look;
mod noise;
mod path;
mod plane;
mod render;
mod settings;
mod shade;
mod spec;

use craft_plugin::prelude::*;

use path::Path;
use settings::{Settings, Source};

struct PlasmaBeam;

fn sources(cx: &Cx, settings: &Settings) -> Vec<Path> {
    match settings.source {
        Source::TwoPoints(ends) => vec![Path::new(ends.to_vec(), false)],
        Source::Masks => cx.masks().into_iter().map(|mask| Path::new(mask.points.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), mask.closed)).collect(),
    }
}

impl Effect for PlasmaBeam {
    fn spec(&self) -> EffectSpec {
        spec::effect()
    }

    fn layout(&self, cx: &Cx) -> Layout {
        let settings = Settings::read(cx);
        Layout::pointwise().padded(settings.padding(&sources(cx, &settings), cx.size))
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let core = spec::CoreControls::read(cx);
        if core.kind == 0 {
            overlay.line(&[core.start, core.end]);
            overlay.point(cx, spec::CORE_START).point(cx, spec::CORE_END);
        }
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        render::render(&settings, &sources(cx, &settings), cx.size.map(|side| side as usize), frame);
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(PlasmaBeam)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin::{Bind, RenderCx, Value};
    use craft_plugin_host::assert_frame;
    use craft_plugin_host::testing::TestPlugin;

    use super::*;

    fn overlay(core_type: f64) -> Overlay {
        let params =
            [("Core/type", Value::Number(core_type)), (spec::CORE_START, Value::List(vec![80.0, 72.0])), (spec::CORE_END, Value::List(vec![80.0, 18.0]))];
        let cx = Cx::new(RenderCx { params: params.into_iter().map(|(id, value)| (id.to_string(), value)).collect(), scale: 1.0, ..Default::default() });
        let mut overlay = Overlay::new();
        PlasmaBeam.overlay(&cx, &mut overlay);
        overlay
    }

    #[test]
    fn two_point_beams_get_start_and_end_handles_and_mask_beams_get_none() {
        let handles: Vec<([f64; 2], Bind)> = overlay(0.0).handles.into_iter().map(|handle| (handle.at, handle.bind)).collect();
        let point = |param: &str| Bind::Point { param: param.into() };
        assert_eq!(handles, vec![([80.0, 72.0], point(spec::CORE_START)), ([80.0, 18.0], point(spec::CORE_END))]);
        assert_eq!(overlay(1.0), Overlay::new());
    }

    #[test]
    fn the_default_beam_matches_its_stored_frame() {
        assert_frame!(TestPlugin::of(plugin), "beam", { "preset": 0 }, "default");
    }

    #[test]
    fn the_electric_preset_with_a_wider_distortion_matches_its_stored_frame() {
        let plugin = TestPlugin::of(plugin);
        assert_frame!(plugin.render("beam").time(1.25).set("preset", 2.0).set("Distortion/amount", 250.0), "electric-distorted");
    }
}
