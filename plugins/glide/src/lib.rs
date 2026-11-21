mod apply;
mod bridge;
mod curve;
mod panel;
mod presets;
mod selection;
mod shared;

#[cfg(test)]
mod tests;

use craft_plugin::{Plugin, Result, host};

use crate::bridge::LiveHost;
use crate::curve::Curve;
use crate::panel::GlidePanel;
use crate::presets::{EASE_IN, EASE_OUT, EASY_EASE, LINEAR};

fn apply_curve(curve: Curve, label: &str) -> Result<()> {
    let message = apply::apply(&LiveHost, curve, label)?;
    host::info(message);
    Ok(())
}

fn plugin() -> Plugin {
    Plugin::new()
        .panel("glide", "Glide", GlidePanel::new(LiveHost))
        .command("apply", "Apply Glide Curve", || apply_curve(shared::current()?, apply::APPLY_LABEL))
        .command("easy-ease", "Easy Ease", || apply_curve(EASY_EASE, "Glide: Easy Ease"))
        .command("ease-in", "Ease In", || apply_curve(EASE_IN, "Glide: Ease In"))
        .command("ease-out", "Ease Out", || apply_curve(EASE_OUT, "Glide: Ease Out"))
        .command("linear", "Linear", || apply_curve(LINEAR, "Glide: Linear"))
}

craft_plugin::export!(plugin);
