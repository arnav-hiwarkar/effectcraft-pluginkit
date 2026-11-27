use craft_plugin::{Canvas, Paint, Panel, TaskStatus, Ui, host};
use serde_json::json;

use crate::analyse::{Analysis, Request};

pub const BANDS: [(&str, f64, f64); 7] = [
    ("Sub (20-60 Hz)", 20.0, 60.0),
    ("Bass (60-250 Hz)", 60.0, 250.0),
    ("Low Mid (250-500 Hz)", 250.0, 500.0),
    ("Mid (500 Hz-2 kHz)", 500.0, 2000.0),
    ("High Mid (2-4 kHz)", 2000.0, 4000.0),
    ("Presence (4-6 kHz)", 4000.0, 6000.0),
    ("Brilliance (6-20 kHz)", 6000.0, 20000.0),
];

const PLOT_HEIGHT: f32 = 200.0;
const PAD: f32 = 8.0;
const PROGRESS_MS: u32 = 100;

#[derive(Default)]
pub struct PulsePanel {
    layer: String,
    band: usize,
    running: Option<u64>,
    status: String,
    progress: f64,
    analysis: Analysis,
}

impl PulsePanel {
    fn request(&self) -> Result<Request, String> {
        let info = host::command("comp.info", json!({})).map_err(|e| e.0)?;
        let fps = info["frameRate"].as_f64().ok_or("the composition has no frame rate")?;
        let duration = info["duration"].as_f64().ok_or("the composition has no duration")?;
        let (_, low, high) = BANDS[self.band.min(BANDS.len() - 1)];
        let layer = (!self.layer.trim().is_empty()).then(|| self.layer.trim().to_string());
        Ok(Request { layer, low, high, fps, duration, write: true })
    }

    fn start(&mut self) {
        let started = self.request().and_then(|request| host::start("analyse", request).map_err(|e| e.0));
        match started {
            Ok(id) => {
                self.running = Some(id);
                self.status = "Analysing".into();
            }
            Err(error) => self.status = error,
        }
    }

    fn poll(&mut self) {
        let Some(id) = self.running else { return };
        let Ok(state) = host::task(id) else { return };
        self.progress = state.progress;
        self.status = state.message.clone();
        match state.status {
            TaskStatus::Running => {}
            TaskStatus::Done => {
                self.analysis = state.result.and_then(|result| serde_json::from_value(result).ok()).unwrap_or_default();
                self.status = format!("Wrote {} keyframes. One undo removes them.", self.analysis.keys);
                self.running = None;
            }
            TaskStatus::Failed => {
                self.status = state.error.unwrap_or_default();
                self.running = None;
            }
            TaskStatus::Cancelled => {
                self.status = "Cancelled".into();
                self.running = None;
            }
        }
    }
}

impl Panel for PulsePanel {
    fn ui(&mut self, ui: &mut Ui) {
        self.poll();
        if self.running.is_some() {
            ui.every("progress", PROGRESS_MS);
        }
        ui.heading("Pulse");
        ui.text_edit("layer", &mut self.layer, "Audio layer (empty: the composition mix)");
        let names: Vec<&str> = BANDS.iter().map(|(name, _, _)| *name).collect();
        ui.popup("Band", &mut self.band, &names);
        ui.row(|ui| {
            if ui.button_enabled("Analyse", self.running.is_none()) {
                self.start();
            }
            if ui.button_enabled("Cancel", self.running.is_some())
                && let Some(id) = self.running
            {
                let _ = host::cancel(id);
            }
        });
        match self.running {
            Some(_) => ui.label(&format!("{:.0}% {}", self.progress * 100.0, self.status)),
            None => ui.weak(&self.status),
        }
        let width = ui.available()[0].max(120.0);
        let analysis = &self.analysis;
        ui.canvas("plot", [width, PLOT_HEIGHT], |canvas| draw(canvas, analysis, [width, PLOT_HEIGHT]));
    }
}

fn draw(canvas: &mut Canvas, analysis: &Analysis, size: [f32; 2]) {
    canvas.rect([0.0, 0.0], size, Some(Paint::Surface), Some((Paint::Border, 1.0)), 4.0);
    let half = size[1] / 2.0;
    let inner = size[0] - 2.0 * PAD;
    let bars = analysis.spectrum.len().max(1) as f32;
    for (i, level) in analysis.spectrum.iter().enumerate() {
        let x = PAD + inner * i as f32 / bars;
        let top = half - PAD - (half - 2.0 * PAD) * (*level as f32).sqrt();
        canvas.rect([x, top], [x + inner / bars - 1.0, half - PAD], Some(Paint::Weak), None, 0.0);
    }
    let count = analysis.values.len().max(2) as f32 - 1.0;
    let points: Vec<[f32; 2]> =
        analysis.values.iter().enumerate().map(|(i, v)| [PAD + inner * i as f32 / count, size[1] - PAD - (half - 2.0 * PAD) * (*v as f32 / 100.0)]).collect();
    if points.len() > 1 {
        canvas.line(&points, Paint::Accent, 1.5);
    }
    canvas.text([PAD, PAD], "Spectrum", Paint::Weak, 11.0);
    canvas.text([PAD, half + PAD], "Band energy per frame", Paint::Weak, 11.0);
}
