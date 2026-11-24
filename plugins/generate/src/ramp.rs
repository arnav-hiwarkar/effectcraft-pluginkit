use craft_plugin::prelude::*;

const SHAPES: [&str; 2] = ["Linear", "Radial"];

params! {
    struct Settings {
        shape: usize = popup("Shape", &SHAPES, 0),
        from: [f64; 2] = point("Start", [0.2, 0.5]),
        to: [f64; 2] = point("End", [0.8, 0.5]),
        colors: Gradient = gradient("Ramp", &[(0.0, [0.1, 0.2, 0.9, 1.0]), (1.0, [1.0, 0.6, 0.1, 1.0])]),
    }
}

pub struct Ramp;

impl Effect for Ramp {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("ramp", "Gradient Ramp", "Generate").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise().ignores_input()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        let [x0, y0] = settings.from;
        let axis = [settings.to[0] - x0, settings.to[1] - y0];
        let length_squared = (axis[0] * axis[0] + axis[1] * axis[1]).max(1e-9);
        let left = frame.origin[0];
        for (y, row) in frame.rows_mut() {
            for (i, pixel) in row.iter_mut().enumerate() {
                let offset = [(left + i) as f64 + 0.5 - x0, y as f64 + 0.5 - y0];
                let along = match settings.shape {
                    0 => (offset[0] * axis[0] + offset[1] * axis[1]) / length_squared,
                    _ => (offset[0] * offset[0] + offset[1] * offset[1]).sqrt() / length_squared.sqrt(),
                };
                *pixel = settings.colors.sample(along);
            }
        }
        Ok(())
    }
}
