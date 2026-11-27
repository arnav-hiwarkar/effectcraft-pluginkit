use craft_plugin::prelude::*;
use serde_json::{Value, json};

use crate::curve::{Curves, Point, Spline, identity};
use crate::{EFFECT, PARAM};

pub const THUMB: u32 = 56;
const PLUGIN_ID: &str = "org.craftkit.curves";

pub struct Preset {
    pub name: &'static str,
    pub master: &'static [Point],
    pub red: &'static [Point],
    pub blue: &'static [Point],
}

const LINE: &[Point] = &[[0.0, 0.0], [1.0, 1.0]];

pub const PRESETS: &[Preset] = &[
    Preset { name: "Linear", master: LINE, red: LINE, blue: LINE },
    Preset { name: "Soft Contrast", master: &[[0.0, 0.0], [0.25, 0.2], [0.75, 0.8], [1.0, 1.0]], red: LINE, blue: LINE },
    Preset { name: "Strong Contrast", master: &[[0.0, 0.0], [0.25, 0.12], [0.75, 0.88], [1.0, 1.0]], red: LINE, blue: LINE },
    Preset { name: "Lighten", master: &[[0.0, 0.0], [0.5, 0.65], [1.0, 1.0]], red: LINE, blue: LINE },
    Preset { name: "Darken", master: &[[0.0, 0.0], [0.5, 0.35], [1.0, 1.0]], red: LINE, blue: LINE },
    Preset { name: "Negative", master: &[[0.0, 1.0], [1.0, 0.0]], red: LINE, blue: LINE },
    Preset { name: "Fade", master: &[[0.0, 0.15], [1.0, 0.9]], red: LINE, blue: LINE },
    Preset { name: "Crush Shadows", master: &[[0.0, 0.0], [0.2, 0.0], [1.0, 1.0]], red: LINE, blue: LINE },
    Preset { name: "Warm", master: LINE, red: &[[0.0, 0.0], [0.5, 0.58], [1.0, 1.0]], blue: &[[0.0, 0.0], [0.5, 0.42], [1.0, 1.0]] },
    Preset { name: "Cool", master: LINE, red: &[[0.0, 0.0], [0.5, 0.42], [1.0, 1.0]], blue: &[[0.0, 0.0], [0.5, 0.58], [1.0, 1.0]] },
    Preset { name: "Solarize", master: &[[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]], red: LINE, blue: LINE },
    Preset { name: "Matte", master: &[[0.0, 0.1], [0.3, 0.3], [0.8, 0.82], [1.0, 0.92]], red: LINE, blue: LINE },
];

impl Preset {
    pub fn curves(&self) -> Curves {
        let mut curves = Curves::default();
        curves.channels[0] = self.master.to_vec();
        curves.channels[1] = self.red.to_vec();
        curves.channels[3] = self.blue.to_vec();
        curves.channels[2] = identity();
        curves
    }
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / length).clamp(0.0, 1.0) } else { 0.0 };
    ((ap[0] - ab[0] * t).powi(2) + (ap[1] - ab[1] * t).powi(2)).sqrt()
}

fn polyline(points: &[Point], side: f32) -> Vec<[f32; 2]> {
    let spline = Spline::new(points);
    (0..=48).map(|i| i as f64 / 48.0).map(|x| [x as f32 * side, (1.0 - spline.at(x).clamp(0.0, 1.0) as f32) * side]).collect()
}

fn coverage(line: &[[f32; 2]], p: [f32; 2], width: f32) -> f32 {
    let distance = line.windows(2).map(|w| segment_distance(p, w[0], w[1])).fold(f32::MAX, f32::min);
    (width - distance + 0.5).clamp(0.0, 1.0)
}

pub fn thumbnail(preset: &Preset) -> Vec<u8> {
    let side = THUMB as f32;
    let tinted = [(preset.red, [235, 90, 90]), (preset.blue, [100, 150, 245])].into_iter().filter(|(points, _)| *points != LINE);
    let lines: Vec<(Vec<[f32; 2]>, [u8; 3])> = tinted.chain([(preset.master, [240, 240, 240])]).map(|(points, tint)| (polyline(points, side), tint)).collect();
    let mut rgba = Vec::with_capacity((THUMB * THUMB * 4) as usize);
    for y in 0..THUMB {
        for x in 0..THUMB {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let grid = x % (THUMB / 4) == 0 || y % (THUMB / 4) == 0;
            let mut colour = if grid { [52.0f32, 52.0, 58.0] } else { [32.0, 32.0, 36.0] };
            for (line, tint) in &lines {
                let cover = coverage(line, p, 1.0);
                for c in 0..3 {
                    colour[c] += (f32::from(tint[c]) - colour[c]) * cover;
                }
            }
            rgba.extend(colour.map(|c| c.round() as u8));
            rgba.push(255);
        }
    }
    rgba
}

fn curves_path(tree: &Value) -> Option<String> {
    let children = tree.pointer("/properties/children")?.as_array()?;
    let effects = children.iter().find(|c| c["match"] == "effects")?["children"].as_array()?;
    let wanted = format!("{PLUGIN_ID}.{EFFECT}");
    effects.iter().find(|e| e["effect"] == wanted.as_str())?["path"].as_str().map(|path| format!("{path}/{PARAM}"))
}

pub fn apply(preset: &Preset) -> Result<String> {
    let path = match curves_path(&host::command("layer.tree", json!({}))?) {
        Some(path) => path,
        None => {
            host::command("effect.apply", json!({"effect": format!("{PLUGIN_ID}.{EFFECT}")}))?;
            curves_path(&host::command("layer.tree", json!({}))?).ok_or("the Curves effect did not appear on the selected layer")?
        }
    };
    host::command("prop.set", json!({"path": path, "value": preset.curves().to_text()}))?;
    Ok(format!("Applied {} to the selected layer.", preset.name))
}

#[derive(Default)]
pub struct PresetPanel {
    uploaded: bool,
    message: Option<std::result::Result<String, String>>,
}

impl PresetPanel {
    fn upload(&mut self) -> Result<()> {
        for (i, preset) in PRESETS.iter().enumerate() {
            host::upload_image(&format!("preset/{i}"), THUMB, THUMB, &thumbnail(preset))?;
        }
        self.uploaded = true;
        Ok(())
    }
}

impl Panel for PresetPanel {
    fn ui(&mut self, ui: &mut Ui) {
        if !self.uploaded
            && let Err(error) = self.upload()
        {
            self.message = Some(Err(error.0));
        }
        ui.heading("Curve Presets");
        ui.weak("Click a preset to set the Curves effect of the selected layer, adding one if needed.");
        let columns = ((ui.available()[0] - 24.0) / (THUMB as f32 + 16.0)).floor().max(1.0) as u32;
        let mut chosen = None;
        ui.scroll("presets", 260.0, |ui| {
            ui.grid(columns, |ui| {
                for (i, preset) in PRESETS.iter().enumerate() {
                    ui.column(|ui| {
                        if ui.image(&format!("preset/{i}"), [THUMB as f32, THUMB as f32]) {
                            chosen = Some(preset);
                        }
                        ui.tooltip(preset.name);
                        ui.weak(preset.name);
                    });
                }
            });
        });
        if let Some(preset) = chosen {
            self.message = Some(apply(preset).map_err(|e| e.0));
        }
        match &self.message {
            Some(Ok(text)) => ui.label(text),
            Some(Err(text)) => ui.error(text),
            None => {}
        }
    }
}
