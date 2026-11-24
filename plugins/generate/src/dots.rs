use craft_plugin::prelude::*;

params! {
    struct Settings {
        centre: [f64; 2] = point("Centre", [0.5, 0.5]),
        columns: f64 = slider("Columns", 5.0, 1.0..=32.0),
        rows: f64 = slider("Rows", 3.0, 1.0..=32.0),
        spacing: f64 = slider("Spacing", 120.0, 4.0..=1000.0),
        radius: f64 = slider("Radius", 30.0, 1.0..=500.0),
        softness: f64 = percent("Softness", 60.0),
        tint: [f32; 4] = color("Tint", [1.0, 0.8, 0.3, 1.0]),
    }
}

pub struct Dots;

impl Effect for Dots {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("dots", "Soft Dots", "Generate").params(Settings::params())
    }

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::pointwise().ignores_input()
    }

    fn render(&self, cx: &Cx, frame: &mut Frame) -> Result<()> {
        let settings = Settings::read(cx);
        let (spacing, radius) = ((settings.spacing * cx.scale) as f32, (settings.radius * cx.scale) as f32);
        let soft = radius * (settings.softness / 100.0) as f32;
        let [columns, rows] = [settings.columns, settings.rows].map(|n| n.round() as f32);
        let first = [0, 1].map(|axis| settings.centre[axis] as f32 - [columns, rows][axis] * 0.5 * spacing + spacing * 0.5);
        let reach = radius + 1.0;
        let covered = Rect {
            left: (first[0] - reach).floor() as i32,
            top: (first[1] - reach).floor() as i32,
            right: (first[0] + (columns - 1.0) * spacing + reach).ceil() as i32,
            bottom: (first[1] + (rows - 1.0) * spacing + reach).ceil() as i32,
        };
        let left = frame.origin[0] as i32;
        let inside = |at: i32| at.clamp(covered.left, covered.right);
        let span = inside(left)..inside(left + frame.width as i32);
        for (y, row) in frame.rows_mut() {
            if !(covered.top..covered.bottom).contains(&(y as i32)) {
                continue;
            }
            let row_nearest = (((y as f32 + 0.5 - first[1]) / spacing).round()).clamp(0.0, rows - 1.0) * spacing + first[1];
            for x in span.clone() {
                let column_nearest = (((x as f32 + 0.5 - first[0]) / spacing).round()).clamp(0.0, columns - 1.0) * spacing + first[0];
                let distance = ((x as f32 + 0.5 - column_nearest).powi(2) + (y as f32 + 0.5 - row_nearest).powi(2)).sqrt();
                let coverage = ((radius - distance) / soft.max(1e-3)).clamp(0.0, 1.0);
                let [r, g, b, a] = settings.tint;
                row[(x - left) as usize] = [r, g, b, a * coverage];
            }
        }
        frame.drawn(covered);
        Ok(())
    }
}
