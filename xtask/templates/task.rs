use craft_plugin::prelude::*;

#[derive(Default)]
struct {{Ident}} {
    running: Option<u64>,
    state: Option<TaskState>,
}

impl Panel for {{Ident}} {
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
        match &self.state {
            Some(TaskState { status: TaskStatus::Running, progress, message, .. }) => ui.progress(*progress as f32, message),
            Some(TaskState { result: Some(result), .. }) => ui.label(&format!("Average brightness {:.3}", result["brightness"].as_f64().unwrap_or_default())),
            Some(TaskState { error: Some(error), .. }) => ui.error(error),
            _ => {}
        }
        if let Some(id) = self.running {
            if ui.button("Cancel") {
                let _ = host::cancel(id);
            }
        } else if ui.button("Measure") {
            self.running = host::start("measure", json!({ "frames": 10 })).ok();
            ui.every("progress", 100);
        }
    }
}

fn measure(task: &Task) -> Result<serde_json::Value> {
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

fn plugin() -> Plugin {
    Plugin::new().panel("{{name}}", "{{Title}}", {{Ident}}::default()).task("measure", "Measure Brightness", measure)
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin_host::testing::{TestPlugin, json};

    #[test]
    fn the_task_averages_the_brightness_of_the_frames_it_reads() {
        let result = TestPlugin::of(super::plugin).task("measure", json!({ "frames": 3 })).unwrap();
        let brightness = result["brightness"].as_f64().unwrap();
        assert!((0.3..0.6).contains(&brightness), "{brightness}");
    }

    #[test]
    fn the_panel_starts_the_task_and_offers_to_cancel_it() {
        let plugin = TestPlugin::of(super::plugin);
        let mut panel = plugin.panel("{{name}}").unwrap();
        panel.click("Measure").unwrap();
        panel.timer("progress").unwrap();
        assert!(panel.texts().contains(&"Cancel".to_string()), "{:?}", panel.texts());
    }
}
