mod dots;
mod field;
mod ramp;
#[cfg(test)]
mod tests;

use craft_plugin::prelude::*;

fn plugin() -> Plugin {
    Plugin::new().effect(ramp::Ramp).effect(field::Field).effect(dots::Dots)
}

craft_plugin::export!(plugin);
