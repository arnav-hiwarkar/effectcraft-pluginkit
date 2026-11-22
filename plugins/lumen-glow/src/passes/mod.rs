pub mod blur;
pub mod composite;
pub mod emit;
pub mod resample;

use craft_plugin::prelude::*;

use crate::settings::Settings;

pub fn run(settings: &Settings, pass: &Pass, out: &mut Frame) -> Result<()> {
    match pass.kernel {
        "lumen_emit" => emit::emit(&pass.src, settings.knee, settings.transfer, out),
        "lumen_composite" => composite::composite(settings, pass, out),
        other => return Err(format!("Lumen Glow has no kernel `{other}`").into()),
    }
    Ok(())
}
