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
        let (radius, left) = (radius * cx.scale, frame.origin[0]);
        for (y, read, out) in frame.rows() {
            for (x, (out, pixel)) in out.iter_mut().zip(read).enumerate() {
                let distance = ((left + x) as f64 - centre[0]).hypot(y as f64 - centre[1]);
                let light = (1.0 - distance / radius).clamp(0.0, 1.0) as f32;
                *out = pixel.map(|c| c * light);
            }
        }
        Ok(())
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let Settings { centre, radius } = Settings::read(cx);
        overlay.point(cx, "centre").radius("radius", centre, radius, 1.0);
    }
}
