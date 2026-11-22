use craft_plugin::prelude::*;

#[derive(Default)]
struct MeasurePanel {
    running: Option<u64>,
    state: Option<TaskState>,
    error: Option<String>,
}

impl Panel for MeasurePanel {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(id) = self.running
            && ui.every("progress", 100)
        {
            match host::task(id) {
                Ok(state) => {
                    if state.status != TaskStatus::Running {
                        self.running = None;
                    }
                    self.state = Some(state);
                }
                Err(error) => (self.running, self.error) = (None, Some(format!("Lost the measurement: {error}"))),
            }
        }
        match &self.state {
            Some(TaskState { status: TaskStatus::Running, progress, message, .. }) => ui.progress(*progress as f32, message),
            Some(TaskState { result: Some(result), .. }) => ui.label(&format!("Average brightness {:.3}", result["brightness"].as_f64().unwrap_or_default())),
            Some(TaskState { error: Some(error), .. }) => ui.error(error),
            _ => {}
        }
        if let Some(id) = self.running {
            if ui.button("Cancel")
                && let Err(error) = host::cancel(id)
            {
                self.error = Some(format!("Could not cancel: {error}"));
            }
        } else if ui.button("Measure") {
            match host::start("measure", json!({ "frames": 10 })) {
                Ok(id) => (self.running, self.state, self.error) = (Some(id), None, None),
                Err(error) => self.error = Some(format!("Could not start measuring: {error}")),
            }
            ui.every("progress", 100);
        }
        if let Some(error) = &self.error {
            ui.error(error);
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
    Plugin::new().panel("{{name}}", "{{Title}}", MeasurePanel::default()).task("measure", "Measure Brightness", measure)
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
