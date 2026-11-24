use craft_plugin::{FrameRequest, Image, Place, Result, Task, host};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const KEY: &str = "track";

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Request {
    pub comp: u64,
    pub layer: u64,
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub search: f64,
    pub first: i64,
    pub frames: usize,
    pub fps: f64,
}

pub fn frame_grid(from: f64, to: f64, fps: f64) -> (i64, usize) {
    let index = |time: f64| (time * fps - 1e-6).ceil() as i64;
    (index(from), (index(to) - index(from)).max(0) as usize)
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

pub const MAX_SEARCH: f64 = 200.0;

pub fn track(task: &Task) -> Result<Value> {
    let request: Request = serde_json::from_value(task.args().clone())?;
    if request.frames == 0 || !request.fps.is_finite() || request.fps <= 0.0 || !(0.0..=MAX_SEARCH).contains(&request.search) {
        return Err(format!("Tracker needs at least one frame and a search range from 0 to {MAX_SEARCH} pixels").into());
    }
    let read = |frame: usize| {
        let time = (request.first + frame as i64) as f64 / request.fps;
        let (comp, layer) = (Some(request.comp.to_string()), Some(request.layer.to_string()));
        host::frame(FrameRequest { comp, layer, time, scale: 1.0, ids: true, source: true }).map(|image| Grey::of(&image))
    };
    let start = request.min.map(|v| v.round() as i32);
    let size = [0, 1].map(|i| request.max[i].round() as i32 - start[i]);
    let first = read(0)?;
    if size[0] < 1 || size[1] < 1 || start[0] < 0 || start[1] < 0 || start[0] + size[0] > first.width || start[1] + size[1] > first.height {
        return Err(format!("Tracker needs a box at least one pixel wide and tall inside the {}×{} layer", first.width, first.height).into());
    }
    let template = first.patch(start, size);
    let centre = [0, 1].map(|i| f64::from(start[i]) + f64::from(size[i]) / 2.0);
    let mut points = vec![centre];
    let mut at = start;
    for frame in 1..request.frames {
        task.progress(frame as f64 / request.frames as f64, &format!("Tracking frame {frame} of {}", request.frames))?;
        let found = best_match(&read(frame)?, &template, size, at, request.search.round() as i32);
        at = found.map(|v| v.round() as i32);
        points.push([0, 1].map(|i| centre[i] + found[i] - f64::from(start[i])));
    }
    let track = Track { version: 1, start: request.first as f64 / request.fps, fps: request.fps, points };
    host::set_data(Place::Layer(request.layer), KEY, &serde_json::to_vec(&track)?)?;
    Ok(serde_json::to_value(track)?)
}
