use craft_plugin::prelude::*;

#[derive(Default)]
pub struct Follow {
    layers: Vec<u64>,
    time: f64,
}

impl Panel for Follow {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(AppEvent::Selection { layers, .. }) = ui.on(Topic::Selection) {
            self.layers = layers.clone();
        }
        if let Some(AppEvent::Time { time, .. }) = ui.on(Topic::Time) {
            self.time = *time;
        }
        ui.label(&format!("{} layers selected at {:.2} s", self.layers.len(), self.time));
    }
}
