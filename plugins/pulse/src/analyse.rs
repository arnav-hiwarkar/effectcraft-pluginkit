use craft_plugin::{AudioRequest, Result, Task, host};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dsp::{self, WINDOW};

pub const RATE: u32 = 48_000;
const CHUNK_SECONDS: f64 = 2.0;
pub const SPECTRUM_BARS: usize = 64;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Request {
    #[serde(default)]
    pub layer: Option<String>,
    pub low: f64,
    pub high: f64,
    pub fps: f64,
    pub duration: f64,
    #[serde(default)]
    pub write: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    pub values: Vec<f64>,
    pub spectrum: Vec<f64>,
    pub keys: usize,
    pub target: Option<u64>,
}

pub fn analyse(task: &Task) -> Result<Value> {
    let request: Request = serde_json::from_value(task.args().clone())?;
    if !(request.fps > 0.0 && request.duration > 0.0 && request.high > request.low) {
        return Err("Pulse needs a frame rate, a duration and a band whose top is above its bottom".into());
    }
    let frames = (request.duration * request.fps).round() as usize;
    let hop = f64::from(RATE) / request.fps;
    let per_chunk = ((CHUNK_SECONDS * request.fps).round() as usize).max(1);
    let mut values = Vec::with_capacity(frames);
    let mut spectrum_sum = vec![0.0; WINDOW / 2];
    let mut first = 0;
    while first < frames {
        let last = (first + per_chunk).min(frames);
        task.progress(first as f64 / frames as f64, &format!("Analysing frame {first} of {frames}"))?;
        let lead = (WINDOW / 2) as f64 / f64::from(RATE);
        let start = first as f64 / request.fps - lead;
        let duration = (last - first) as f64 / request.fps + 2.0 * lead;
        let audio = host::audio(AudioRequest { layer: request.layer.clone(), start, duration, rate: RATE })?;
        let mono = dsp::mono(&audio.samples, audio.channels as usize);
        for frame in first..last {
            let center = ((frame - first) as f64 * hop).round() as isize + (WINDOW / 2) as isize;
            let spectrum = dsp::spectrum(&dsp::window_at(&mono, center));
            spectrum_sum.iter_mut().zip(&spectrum).for_each(|(sum, bin)| *sum += bin);
            values.push(dsp::band_energy(&spectrum, f64::from(RATE), request.low, request.high));
        }
        first = last;
    }
    let peak = values.iter().cloned().fold(0.0, f64::max);
    let values: Vec<f64> = values.iter().map(|v| if peak > 0.0 { v / peak * 100.0 } else { 0.0 }).collect();
    let spectrum = bars(&spectrum_sum, frames.max(1) as f64);
    let mut analysis = Analysis { values, spectrum, ..Default::default() };
    if request.write {
        task.progress(1.0, "Writing keyframes")?;
        let target = write_keys(&analysis.values, request.fps, &request)?;
        analysis.keys = analysis.values.len();
        analysis.target = Some(target);
    }
    Ok(serde_json::to_value(analysis)?)
}

fn bars(sum: &[f64], count: f64) -> Vec<f64> {
    let per = sum.len() / SPECTRUM_BARS;
    let raw: Vec<f64> = sum.chunks(per.max(1)).take(SPECTRUM_BARS).map(|bins| bins.iter().sum::<f64>() / count / bins.len() as f64).collect();
    let peak = raw.iter().cloned().fold(0.0, f64::max);
    raw.iter().map(|v| if peak > 0.0 { v / peak } else { 0.0 }).collect()
}

pub fn label(request: &Request) -> String {
    format!("Pulse {:.0}-{:.0} Hz", request.low, request.high)
}

fn write_keys(values: &[f64], fps: f64, request: &Request) -> Result<u64> {
    let made = host::command("layer.newNull", json!({"name": label(request)}))?;
    let layer = made["layer"].as_u64().ok_or("the app made no layer")?;
    host::command("effect.apply", json!({"layers": [layer], "effect": "ec.control.slider"}))?;
    let steps: Vec<Value> = values
        .iter()
        .enumerate()
        .map(|(frame, value)| {
            let params = json!({"layer": layer, "path": "effects/#1/slider", "time": frame as f64 / fps, "timeBase": "comp", "value": value});
            json!({"command": "prop.addKey", "params": params})
        })
        .collect();
    host::command("engine.batch", json!({"label": format!("{}: Keyframes", label(request)), "steps": steps}))?;
    Ok(layer)
}
