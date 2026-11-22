use craft_plugin::prelude::*;

params! {
    struct Settings {
        gamma: f64 = slider("Gamma", 2.2, 0.2..=5.0),
    }
}

pub struct Gamma;

fn table(gamma: f64) -> Vec<u8> {
    (0..1024).flat_map(|i| ((i as f64 / 1023.0).powf(1.0 / gamma) as f32).to_le_bytes()).collect()
}

impl Effect for Gamma {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("gamma", "Gamma", "Cookbook").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let gamma = Settings::read(cx).gamma;
        let key = format!("table/{gamma}");
        let bytes = match cx.recall(&key) {
            Some(bytes) => bytes,
            None => {
                let bytes = table(gamma);
                cx.remember(&key, &bytes)?;
                bytes
            }
        };
        let table: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        for pixel in frame.pixels.iter_mut() {
            for channel in &mut pixel[..3] {
                *channel = table[(channel.clamp(0.0, 1.0) * 1023.0) as usize];
            }
        }
        Ok(())
    }
}
