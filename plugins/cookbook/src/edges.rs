use craft_plugin::prelude::*;

params! {
    struct Settings {
        width: f64 = slider("Width", 2.0, 1.0..=20.0),
    }
}

pub struct Edges;

impl Effect for Edges {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("edges", "Find Edges", "Cookbook").params(Settings::params())
    }

    fn layout(&self, cx: &Cx) -> Layout {
        Layout::neighbourhood(step(cx) as u32)
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let step = step(cx) as isize;
        let source = frame.pixels.to_vec();
        let read = Rows { width: frame.width, height: frame.height, first_row: frame.origin[1], pixels: &source };
        for (y, row) in frame.rows_mut() {
            let y = y as isize;
            for (x, pixel) in row.iter_mut().enumerate() {
                let x = x as isize;
                let dx = read.at(x + step, y)[1] - read.at(x - step, y)[1];
                let dy = read.at(x, y + step)[1] - read.at(x, y - step)[1];
                let edge = (dx * dx + dy * dy).sqrt().min(1.0);
                *pixel = [edge, edge, edge, 1.0];
            }
        }
        Ok(())
    }
}

fn step(cx: &Cx) -> f64 {
    (Settings::read(cx).width * cx.scale).round().max(1.0)
}
