use craft_plugin::prelude::*;

#[derive(Default)]
struct ShortcutProbe {
    pressed: bool,
}

impl Panel for ShortcutProbe {
    fn ui(&mut self, ui: &mut Ui) {
        self.pressed |= ui.key("Delete");
        ui.label(if self.pressed { "pressed Delete" } else { "Waiting for Delete" });
    }
}

fn plugin() -> Plugin {
    Plugin::new().panel("keys", "Modal Shortcut Probe", ShortcutProbe::default()).keys(&["Delete"])
}

craft_plugin::export!(plugin);
