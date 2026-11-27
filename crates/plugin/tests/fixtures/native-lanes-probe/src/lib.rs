use craft_plugin::prelude::*;
use serde_json::json;

struct Aim;

impl Effect for Aim {
    fn spec(&self) -> EffectSpec {
        EffectSpec::new("aim", "Aim", "Probe")
    }

    fn overlay(&self, _cx: &Cx, overlay: &mut Overlay) {
        overlay.circle([1.0, 2.0], 1.0);
    }
}

struct Stamp;

impl Tool for Stamp {
    fn event(&mut self, event: &ToolEvent, overlay: &mut Overlay) -> Result<()> {
        let [x, y] = event.comp;
        overlay.text([x, y], &format!("{:?} {x:.1} {y:.1}", event.kind));
        Ok(())
    }
}

#[derive(Default)]
struct Relay {
    said: String,
}

impl Panel for Relay {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Relay") {
            while host::read_file(Root::Data, "hold").is_ok() {
                host::notify("held");
            }
            self.said = match host::command("comp.info", json!({})) {
                Ok(_) => "relayed".into(),
                Err(error) => error.to_string(),
            };
        }
        ui.label(&self.said);
    }
}

fn plugin() -> Plugin {
    Plugin::new().effect(Aim).tool("stamp", "Stamp", Stamp).panel("relay", "Relay", Relay::default()).command("greet", "Greet", || {
        host::notify("3 layers renamed");
        Ok(())
    })
}

craft_plugin::export!(plugin);
