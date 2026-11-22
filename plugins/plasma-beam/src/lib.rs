mod beam;
mod glow;
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
