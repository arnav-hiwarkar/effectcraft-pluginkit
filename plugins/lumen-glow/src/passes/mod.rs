pub mod blur;
pub mod composite;
pub mod emit;
pub mod level;
pub mod resample;

use craft_plugin::prelude::*;

use crate::settings::Settings;

pub fn run(settings: &Settings, pass: &Pass, out: &mut Frame) -> Result<()> {
    match pass.kernel {
        "lumen_emit" => emit::emit(&pass.src, settings.knee, settings.transfer, out),
        "lumen_downsample" => {
            let halve = [0, 1].map(|axis| pass.u.get(axis).is_some_and(|flag| *flag != 0));
            resample::downsample(&pass.src, out, halve);
        }
        "lumen_level" => level::level(pass, out),
        "lumen_composite" => composite::composite(settings, pass, out),
        other => return Err(format!("Lumen Glow has no kernel `{other}`").into()),
    }
    Ok(())
}
