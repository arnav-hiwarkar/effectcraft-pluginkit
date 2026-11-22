use craft_plugin::prelude::*;

#[derive(Default)]
struct {{Ident}} {
    pins: Vec<[f64; 2]>,
}

impl Tool for {{Ident}} {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        if event.kind == ToolEventKind::Down {
            self.pins.push(event.comp);
            host::command("layer.newNull", json!({ "name": format!("Pin {}", self.pins.len()) }))?;
        }
        for pin in &self.pins {
            overlay.circle(*pin, 4.0);
        }
        overlay.circle(event.comp, 8.0).text([event.comp[0] + 10.0, event.comp[1]], "{{Title}}");
        Ok(())
    }
}

fn plugin() -> Plugin {
    Plugin::new().tool("{{name}}", "{{Title}}", {{Ident}}::default())
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin::ToolEventKind;
    use craft_plugin_host::testing::{TestPlugin, json, pointer};

    #[test]
    fn a_click_adds_a_null_and_leaves_a_pin_on_the_viewer() {
        let plugin = TestPlugin::of(super::plugin);
        plugin.tool("{{name}}", pointer(ToolEventKind::Down, [40.0, 30.0])).unwrap();
        let overlay = plugin.tool("{{name}}", pointer(ToolEventKind::Move, [90.0, 60.0])).unwrap();
        assert_eq!(plugin.commands(), [("layer.newNull".to_string(), json!({ "name": "Pin 1" }))]);
        assert_eq!(overlay.marks.len(), 3, "the pin, the pointer ring and its label");
    }
}
