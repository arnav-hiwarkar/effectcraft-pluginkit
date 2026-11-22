use craft_plugin::prelude::*;

pub fn add_rig() -> Result<()> {
    let steps = json!([
        { "command": "layer.newNull", "params": { "name": "Rig Root" } },
        { "command": "layer.newNull", "params": { "name": "Rig Aim" } },
    ]);
    host::command("engine.batch", json!({ "label": "Add Rig", "steps": steps }))?;
    log!(info, "added a rig of two nulls");
    Ok(())
}

#[derive(Default)]
pub struct Rig {
    error: Option<String>,
}

impl Panel for Rig {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Add Rig") {
            self.error = add_rig().err().map(|error| error.to_string());
        }
        if let Some(error) = &self.error {
            ui.error(error);
        }
    }
}
