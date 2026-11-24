mod panel;
mod stabilise;
mod track;

#[cfg(test)]
mod tests;

use craft_plugin::Plugin;

fn plugin() -> Plugin {
    Plugin::new().effect(stabilise::Stabilise).panel("tracker", "Tracker", panel::TrackerPanel::default()).task("track", "Track Point", track::track)
}

craft_plugin::export!(plugin);
