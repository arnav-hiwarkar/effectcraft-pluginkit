use craft_plugin::{FrameRequest, Image, Place, Result, Task, host};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const KEY: &str = "track";

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Request {
    pub layer: u64,
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub search: f64,
    pub from: f64,
    pub to: f64,
    pub fps: f64,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub version: u32,
    pub start: f64,
    pub fps: f64,
    pub points: Vec<[f64; 2]>,
}

impl Track {
    pub fn at(&self, comp_time: f64) -> Option<[f64; 2]> {
        let last = self.points.len().checked_sub(1)?;
        let frame = ((comp_time - self.start) * self.fps).round().clamp(0.0, last as f64) as usize;
        Some(self.points[frame])
    }
}

pub struct Grey {
    width: i32,
    height: i32,
    values: Vec<f32>,
}

impl Grey {
    pub fn of(image: &Image) -> Grey {
        let values = image.pixels().iter().map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).collect();
        Grey { width: image.width as i32, height: image.height as i32, values }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height { 0.0 } else { self.values[(y * self.width + x) as usize] }
    }

    fn patch(&self, at: [i32; 2], size: [i32; 2]) -> Vec<f32> {
        (0..size[1]).flat_map(|v| (0..size[0]).map(move |u| [u, v])).map(|[u, v]| self.at(at[0] + u, at[1] + v)).collect()
    }

    fn sad(&self, template: &[f32], size: [i32; 2], at: [i32; 2]) -> f32 {
        let mut sum = 0.0;
        for v in 0..size[1] {
            let row = &template[(v * size[0]) as usize..][..size[0] as usize];
            sum += row.iter().zip(0..).map(|(t, u)| (t - self.at(at[0] + u, at[1] + v)).abs()).sum::<f32>();
        }
        sum
    }
}

fn vertex(before: f32, best: f32, after: f32) -> f64 {
    let curve = before - 2.0 * best + after;
    if curve > 0.0 { (f64::from(before - after) / (2.0 * f64::from(curve))).clamp(-0.5, 0.5) } else { 0.0 }
}

pub fn best_match(frame: &Grey, template: &[f32], size: [i32; 2], around: [i32; 2], search: i32) -> [f64; 2] {
    let mut best = (f32::INFINITY, around);
    for dy in -search..=search {
        for dx in -search..=search {
            let at = [around[0] + dx, around[1] + dy];
            let sad = frame.sad(template, size, at);
            if sad < best.0 {
                best = (sad, at);
            }
        }
    }
    let (score, [x, y]) = best;
    let sad = |at| frame.sad(template, size, at);
    let fx = vertex(sad([x - 1, y]), score, sad([x + 1, y]));
    let fy = vertex(sad([x, y - 1]), score, sad([x, y + 1]));
    [f64::from(x) + fx, f64::from(y) + fy]
}

pub fn track(task: &Task) -> Result<Value> {
    let request: Request = serde_json::from_value(task.args().clone())?;
    let frames = ((request.to - request.from) * request.fps).round() as usize;
    if frames == 0 || request.max[0] <= request.min[0] || request.max[1] <= request.min[1] {
        return Err("Tracker needs a box with a size and a time range of at least one frame".into());
    }
    host::remove_data(Place::Layer(request.layer), KEY)?;
    let read = |frame: usize| {
        let time = request.from + frame as f64 / request.fps;
        host::frame(FrameRequest { comp: None, layer: Some(request.layer.to_string()), time, scale: 1.0 }).map(|image| Grey::of(&image))
    };
    let start = request.min.map(|v| v.round() as i32);
    let size = [0, 1].map(|i| (request.max[i] - request.min[i]).round() as i32);
    let template = read(0)?.patch(start, size);
    let centre = [0, 1].map(|i| (request.min[i] + request.max[i]) / 2.0);
    let mut points = vec![centre];
    let mut at = start;
    for frame in 1..frames {
        task.progress(frame as f64 / frames as f64, &format!("Tracking frame {frame} of {frames}"))?;
        let found = best_match(&read(frame)?, &template, size, at, request.search.round() as i32);
        at = found.map(|v| v.round() as i32);
        points.push([0, 1].map(|i| centre[i] + found[i] - f64::from(start[i])));
    }
    let track = Track { version: 1, start: request.from, fps: request.fps, points };
    host::set_data(Place::Layer(request.layer), KEY, &serde_json::to_vec(&track)?)?;
    Ok(serde_json::to_value(track)?)
}
