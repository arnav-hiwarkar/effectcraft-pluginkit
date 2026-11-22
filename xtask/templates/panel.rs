use craft_plugin::prelude::*;

#[derive(Default)]
struct {{Ident}} {
    added: u32,
    selected: usize,
}

impl Panel for {{Ident}} {
    fn ui(&mut self, ui: &mut Ui) {
        if let Some(AppEvent::Selection { layers, .. }) = ui.on(Topic::Selection) {
            self.selected = layers.len();
        }
        ui.heading("{{Title}}");
        ui.label(&format!("{} layers selected", self.selected));
        if ui.button("Add Null") {
            self.added += 1;
            match host::command("layer.newNull", json!({ "name": format!("{{Title}} {}", self.added) })) {
                Ok(_) => {}
                Err(error) => log!(error, "could not add a null: {error}"),
            }
        }
        ui.label(&format!("{} nulls added", self.added));
    }
}

fn plugin() -> Plugin {
    Plugin::new().panel("{{name}}", "{{Title}}", {{Ident}}::default())
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests {
    use craft_plugin::AppEvent;
    use craft_plugin_host::testing::{TestPlugin, json};

    #[test]
    fn the_button_adds_a_named_null_and_the_panel_counts_it() {
        let plugin = TestPlugin::of(super::plugin);
        let mut panel = plugin.panel("{{name}}").unwrap();
        panel.click("Add Null").unwrap();
        assert_eq!(plugin.commands(), [("layer.newNull".to_string(), json!({ "name": "{{Title}} 1" }))]);
        assert!(panel.texts().contains(&"1 nulls added".to_string()), "{:?}", panel.texts());
    }

    #[test]
    fn the_panel_follows_the_selection() {
        let plugin = TestPlugin::of(super::plugin);
        let mut panel = plugin.panel("{{name}}").unwrap();
        panel.app_event(AppEvent::Selection { layers: vec![3, 4], properties: vec![], keyframes: 0 }).unwrap();
        assert!(panel.texts().contains(&"2 layers selected".to_string()), "{:?}", panel.texts());
    }
}
