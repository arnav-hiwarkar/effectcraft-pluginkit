use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const USAGE: &str = "usage: cargo xtask plugin <plugin-dir> [--native] [--out DIR]";
const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn library_name(manifest: &Path) -> Result<String, String> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
        .arg(manifest)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into());
    }
    let metadata: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let targets = metadata["packages"][0]["targets"].as_array().cloned().unwrap_or_default();
    targets
        .iter()
        .find(|t| t["kind"].as_array().is_some_and(|kinds| kinds.iter().any(|k| k == "cdylib")))
        .and_then(|t| t["name"].as_str())
        .map(|name| name.replace('-', "_"))
        .ok_or_else(|| format!("{}: no `cdylib` library target", manifest.display()))
}

fn native_file(name: &str) -> String {
    format!("{}{name}{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from).ok_or(USAGE)?;
    let native = args.iter().any(|a| a == "--native");
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let manifest = json(&dir.join("plugin.json"))?;
    let id = manifest["id"].as_str().ok_or("plugin.json has no `id`")?;
    let bundle = out.unwrap_or_else(|| root.join("plugins/dist")).join(id);
    let cargo_manifest = dir.join("Cargo.toml");
    let name = library_name(&cargo_manifest)?;

    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "--release", "--manifest-path"]).arg(&cargo_manifest);
    let (built, entry) = if native {
        let file = native_file(&name);
        (dir.join("target/release").join(&file), file)
    } else {
        build.args(["--target", WASM_TARGET]);
        let entry = manifest["entry"]["wasm"].as_str().ok_or("plugin.json `entry` has no `wasm` file; pass --native")?;
        (dir.join("target").join(WASM_TARGET).join("release").join(format!("{name}.wasm")), entry.to_string())
    };
    if !build.status().map_err(|e| e.to_string())?.success() {
        return Err(format!("{}: the build failed", dir.display()));
    }

    let io = |e: std::io::Error| format!("{}: {e}", bundle.display());
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle).map_err(io)?;
    }
    std::fs::create_dir_all(&bundle).map_err(io)?;
    std::fs::copy(&built, bundle.join(&entry)).map_err(|e| format!("{}: {e}", built.display()))?;
    let mut bundled = manifest.clone();
    if native {
        let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        bundled["entry"] = serde_json::json!({ "native": { target: entry } });
    }
    let text = serde_json::to_string_pretty(&bundled).map_err(|e| e.to_string())?;
    std::fs::write(bundle.join("plugin.json"), text).map_err(io)?;
    let assets = dir.join("assets");
    if assets.is_dir() {
        copy_tree(&assets, &bundle.join("assets")).map_err(io)?;
    }
    println!("{}", bundle.display());
    Ok(())
}
