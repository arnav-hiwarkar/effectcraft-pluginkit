mod analyse;
mod dsp;
mod panel;

#[cfg(test)]
mod tests;

use craft_plugin::Plugin;

use crate::panel::PulsePanel;

fn plugin() -> Plugin {
    Plugin::new().panel("pulse", "Pulse", PulsePanel::default()).task("analyse", "Analyse Audio", analyse::analyse)
}

craft_plugin::export!(plugin);
