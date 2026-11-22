use craft_plugin::prelude::*;

params! {
    struct Settings {
        matte: LayerParam = layer("Matte Layer"),
    }
}

pub struct Matte;

impl Effect for Matte {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("matte", "Luma Matte", "Cookbook").params(Settings::params())
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let Some(matte) = Settings::read(cx).matte.image(cx) else { return Ok(()) };
        for (y, row) in frame.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let [r, g, b, _] = matte.at(x as isize, y as isize);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                pixel.iter_mut().for_each(|c| *c *= luma);
            }
        }
        Ok(())
    }
}
