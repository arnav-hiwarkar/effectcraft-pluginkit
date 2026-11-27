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
        let Some(matte) = Settings::read(cx).matte.image(cx) else {
            frame.map(|pixel| pixel);
            return Ok(());
        };
        let left = frame.origin[0];
        for (y, read, out) in frame.rows() {
            for (x, (out, pixel)) in out.iter_mut().zip(read).enumerate() {
                let [r, g, b, _] = matte.at((left + x) as isize, y as isize);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                *out = pixel.map(|c| c * luma);
            }
        }
        Ok(())
    }
}
