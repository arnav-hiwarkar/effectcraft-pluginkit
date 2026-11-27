use craft_plugin::prelude::*;

#[derive(Default)]
struct PinTool {
    pins: Vec<[f64; 2]>,
}

fn add_null(name: &str, at: [f64; 2]) -> Result<()> {
    let added = host::command("layer.newNull", json!({ "name": name }))?;
    let layer = added["layer"].as_u64().ok_or("layer.newNull did not return the new layer")?;
    host::command("prop.set", json!({ "layer": layer, "path": "transform/position", "value": [at[0], at[1], 0.0] }))?;
    Ok(())
}

impl Tool for PinTool {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            add_null(&format!("Pin {}", self.pins.len() + 1), event.comp)?;
            self.pins.push(event.comp);
        }
        for pin in &self.pins {
            overlay.circle(*pin, 4.0);
        }
        overlay.circle(event.comp, 8.0).text([event.comp[0] + 10.0, event.comp[1]], "{{Title}}");
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().tool("{{name}}", "{{Title}}", PinTool::default())
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin::ToolEventKind;
    use craft_plugin_host::testing::{TestPlugin, json, pointer};

    #[test]
    fn a_click_adds_a_null_at_the_clicked_point_and_a_pin_only_once_the_null_was_made() {
        let plugin = TestPlugin::of(super::plugin);
        let failed = plugin.tool("{{name}}", pointer(ToolEventKind::Down, [10.0, 10.0]));
        assert_eq!(failed.err().unwrap(), "layer.newNull did not return the new layer");
        plugin.answer("layer.newNull", json!({ "layer": 7 }));
        plugin.tool("{{name}}", pointer(ToolEventKind::Down, [40.0, 30.0])).unwrap();
        let overlay = plugin.tool("{{name}}", pointer(ToolEventKind::Move, [90.0, 60.0])).unwrap();
        assert_eq!(
            plugin.commands()[1..],
            [
                ("layer.newNull".to_string(), json!({ "name": "Pin 1" })),
                ("prop.set".to_string(), json!({ "layer": 7, "path": "transform/position", "value": [40.0, 30.0, 0.0] })),
            ]
        );
        assert_eq!(overlay.marks.len(), 3, "one pin, the pointer ring and its label");
    }
}
