use craft_plugin::prelude::*;

#[derive(Default)]
pub struct Pin {
    pins: Vec<[f64; 2]>,
}

fn add_null(name: &str, at: [f64; 2]) -> Result<()> {
    let added = host::command("layer.newNull", json!({ "name": name }))?;
    let layer = added["layer"].as_u64().ok_or("layer.newNull did not return the new layer")?;
    host::command("prop.set", json!({ "layer": layer, "path": "transform/position", "value": [at[0], at[1], 0.0] }))?;
    Ok(())
}

impl Tool for Pin {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            add_null(&format!("Pin {}", self.pins.len() + 1), event.comp)?;
            self.pins.push(event.comp);
        }
        for pin in &self.pins {
            overlay.circle(*pin, 4.0);
        }
        overlay.circle(event.comp, 8.0);
        Ok(())
    }
}
