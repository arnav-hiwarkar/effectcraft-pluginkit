use craft_plugin::prelude::*;

use crate::track::{KEY, Track};

pub const NAME: &str = "Track and Stabilise";

params! {
    pub struct Settings {
        pub min: [f64; 2] = point("Box Top Left", [0.45, 0.4]),
        pub max: [f64; 2] = point("Box Bottom Right", [0.55, 0.6]),
        pub search: f64 = slider("Search Range", 16.0, 1.0..=200.0),
        pub stabilise: bool = checkbox("Stabilise", true),
    }
}

pub struct Stabilise;

fn sample(input: Rows, x: f64, y: f64) -> Pixel {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (x0, y0) = (x0 as isize, y0 as isize);
    let mix = |a: Pixel, b: Pixel, t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    mix(mix(input.at(x0, y0), input.at(x0 + 1, y0), fx), mix(input.at(x0, y0 + 1), input.at(x0 + 1, y0 + 1), fx), fy)
}

impl Effect for Stabilise {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("stabilise", NAME, "Distort").version(1).params(Settings::params()).time_dependent()
    }

    fn overlay(&self, cx: &Cx, overlay: &mut Overlay) {
        let Settings { min, max, search, .. } = Settings::read(cx);
        overlay.rect(cx, "min", "max");
        overlay.rectangle([min[0] - search, min[1] - search], [max[0] + search, max[1] + search]);
        overlay.radius("search", [max[0], (min[1] + max[1]) / 2.0], search, 1.0).named("Search Range");
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let track: Option<Track> = match Settings::read(cx).stabilise {
            true => cx.layer_data(KEY)?.map(|bytes| serde_json::from_slice(&bytes)).transpose()?,
            false => None,
        };
        let (Some(first), Some(now)) = (track.as_ref().and_then(|t| t.points.first()), track.as_ref().and_then(|t| t.at(cx.comp_time))) else {
            frame.map(|pixel| pixel);
            return Ok(());
        };
        let shift = [0, 1].map(|i| (now[i] - first[i]) * cx.scale);
        let (input, x0) = (frame.input, frame.origin[0] as f64);
        for (y, row) in frame.rows_mut() {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = sample(input, x0 + x as f64 + shift[0], y as f64 + shift[1]);
            }
        }
        Ok(())
    }
}
