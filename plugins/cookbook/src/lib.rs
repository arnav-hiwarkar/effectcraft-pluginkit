mod blur;
mod edges;
mod follow;
mod glow;
mod handle;
mod kernel;
mod matte;
mod measure;
mod memo;
mod nulls;
mod pin;
mod tint;

use craft_plugin::prelude::*;

fn plugin() -> Plugin {
    let plugin = Plugin::new()
        .effect(tint::Tint)
        .effect(blur::Blur)
        .effect(glow::Glow)
        .effect(kernel::Posterize)
        .effect(matte::Matte)
        .effect(edges::Edges)
        .effect(memo::Gamma)
        .effect(handle::Spotlight)
        .command("add-rig", "Add Rig", nulls::add_rig)
        .panel("rig", "Rig", nulls::Rig::default())
        .panel("follow", "Follow", follow::Follow::default())
        .panel("meter", "Meter", measure::Meter::default())
        .task("measure", "Measure Brightness", measure::measure)
        .tool("pin", "Pin", pin::Pin::default());
    #[cfg(test)]
    let plugin = plugin.effect(tests::Smear);
    plugin
}

craft_plugin::export!(plugin);

#[cfg(test)]
mod tests;
