use craft_plugin::prelude::*;

pub fn measure(task: &Task) -> Result<serde_json::Value> {
    let frames = task.args()["frames"].as_u64().unwrap_or(10).max(1);
    let mut total = 0.0;
    for n in 0..frames {
        task.progress(n as f64 / frames as f64, &format!("Frame {} of {frames}", n + 1))?;
        let image = host::frame(FrameRequest { comp: None, layer: None, time: n as f64 / 10.0, scale: 0.25 })?;
        let sum: f32 = image.pixels().iter().map(|p| (p[0] + p[1] + p[2]) / 3.0).sum();
        total += f64::from(sum) / image.pixels().len() as f64;
    }
    Ok(json!({ "brightness": total / frames as f64 }))
}

#[derive(Default)]
pub struct Meter {
    running: Option<u64>,
    state: Option<TaskState>,
}

impl Panel for Meter {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(id) = self.running
            && ui.every("progress", 100)
            && let Ok(state) = host::task(id)
        {
            if state.status != TaskStatus::Running {
                self.running = None;
            }
            self.state = Some(state);
        }
        if let Some(state) = &self.state {
            ui.progress(state.progress as f32, &state.message);
        }
        if self.running.is_none() && ui.button("Measure") {
            self.running = host::start("measure", json!({ "frames": 10 })).ok();
            ui.every("progress", 100);
        }
    }
}
