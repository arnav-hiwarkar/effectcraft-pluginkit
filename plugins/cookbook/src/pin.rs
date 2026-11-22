use craft_plugin::prelude::*;

#[derive(Default)]
pub struct Pin {
    pins: Vec<[f64; 2]>,
}

impl Tool for Pin {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            self.pins.push(event.comp);
            host::command("layer.newNull", json!({ "name": format!("Pin {}", self.pins.len()) }))?;
        }
        for pin in &self.pins {
            overlay.circle(*pin, 4.0);
        }
        overlay.circle(event.comp, 8.0);
        Ok(())
    }
}
