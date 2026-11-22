use craft_plugin::prelude::*;

params! {
    struct Settings {
        centre: [f64; 2] = point("Centre", [0.5, 0.5]),
        radius: f64 = slider("Radius", 60.0, 1.0..=2000.0),
    }
}

pub struct Spotlight;

impl Effect for Spotlight {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("spotlight", "Spotlight", "Cookbook").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let Settings { centre, radius } = Settings::read(cx);
        let radius = radius * cx.scale;
        for (y, row) in frame.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let distance = (x as f64 - centre[0]).hypot(y as f64 - centre[1]);
                let light = (1.0 - distance / radius).clamp(0.0, 1.0) as f32;
                pixel.iter_mut().for_each(|c| *c *= light);
            }
        }
        Ok(())
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let Settings { centre, radius } = Settings::read(cx);
        overlay.point(cx, "centre").radius("radius", centre, radius, 1.0);
    }
}
