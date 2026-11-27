pub mod engine;

use std::path::{Path, PathBuf};
use std::process::Command;

pub const CRT: &str = "org.craftkit.retro.crt";
pub const NEON: &str = "org.craftkit.retro.neon";

const BARS: [[f32; 3]; 7] = [
    [0.75, 0.75, 0.75],
    [0.75, 0.75, 0.0],
    [0.0, 0.75, 0.75],
    [0.0, 0.75, 0.0],
    [0.75, 0.0, 0.75],
    [0.75, 0.0, 0.0],
    [0.0, 0.0, 0.75],
];

const GLYPHS: [[u8; 5]; 5] = [
    [0b111, 0b101, 0b111, 0b101, 0b101],
    [0b110, 0b101, 0b110, 0b101, 0b110],
    [0b111, 0b100, 0b100, 0b100, 0b111],
    [0b101, 0b101, 0b111, 0b101, 0b101],
    [0b111, 0b010, 0b010, 0b010, 0b111],
];

pub fn test_card(width: u32, height: u32) -> Vec<[f32; 4]> {
    let (w, h) = (width as i64, height as i64);
    let cell = (h / 18).max(2);
    let text_top = h * 70 / 100;
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let rgb = if y < h * 60 / 100 {
                BARS[((x * 7) / w) as usize]
            } else if y < h * 66 / 100 {
                let ramp = x as f32 / w as f32;
                [ramp, ramp, ramp]
            } else {
                let gx = x / cell;
                let gy = (y - text_top) / cell;
                let glyph = (gx / 4) as usize % GLYPHS.len();
                let column = gx % 4;
                let lit = y >= text_top
                    && (0..5).contains(&gy)
                    && column < 3
                    && x >= cell
                    && GLYPHS[glyph][gy as usize] >> (2 - column) & 1 == 1;
                if lit { [1.0, 1.0, 1.0] } else { [0.04, 0.04, 0.06] }
            };
            pixels.push([rgb[0], rgb[1], rgb[2], 1.0]);
        }
    }
    pixels
}

pub fn plugin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn build_bundle(dir: &Path, native: bool) -> PathBuf {
    let plugin = plugin_dir();
    let manifest = plugin.join("Cargo.toml");
    let target = plugin.join("target");
    let mut args = vec!["build", "--release", "--quiet", "--manifest-path", manifest.to_str().expect("utf-8 path"), "--target-dir", target.to_str().expect("utf-8 path")];
    if !native {
        args.extend(["--target", "wasm32-unknown-unknown"]);
    }
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(&args).status().expect("cargo runs");
    assert!(status.success(), "the retro plugin builds");
    let bundle = dir.join(if native { "native" } else { "org.craftkit.retro" });
    std::fs::create_dir_all(&bundle).expect("bundle dir");
    let manifest = std::fs::read_to_string(plugin.join("plugin.json")).expect("manifest");
    if native {
        let file = format!("{}retro.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
        std::fs::copy(plugin.join("target/release").join(&file), bundle.join(&file)).expect("copy the library");
        let entry = format!(r#"{{ "native": {{ "{}": "{file}" }} }}"#, craft_plugin::native_target());
        std::fs::write(bundle.join("plugin.json"), manifest.replace(r#"{ "wasm": "plugin.wasm" }"#, &entry)).expect("manifest");
    } else {
        std::fs::copy(plugin.join("target/wasm32-unknown-unknown/release/retro.wasm"), bundle.join("plugin.wasm")).expect("copy the wasm");
        std::fs::write(bundle.join("plugin.json"), manifest).expect("manifest");
    }
    bundle
}
