use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

const USAGE: &str = "usage: cargo xtask plugin <plugin-dir> [--native] [--out DIR]\n       cargo xtask plugin new <name> [--panel] [--gpu]\n       cargo xtask plugin dev <plugin-dir> [--bridge PORT]";
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const DEFAULT_BRIDGE: &str = "9877";
const PACKAGE_EXTENSION: &str = "craftplugin";
const POLL: Duration = Duration::from_secs(1);

const TEMPLATE_CARGO: &str = include_str!("../templates/Cargo.toml");
const TEMPLATE_MANIFEST: &str = include_str!("../templates/plugin.json");
const TEMPLATE_LIB: &str = include_str!("../templates/lib.rs");
const TEMPLATE_CPU_LAYOUT: &str = include_str!("../templates/fragments/cpu-layout.rs");
const TEMPLATE_GPU_LAYOUT: &str = include_str!("../templates/fragments/gpu-layout.rs");
const TEMPLATE_GPU_KERNELS: &str = include_str!("../templates/fragments/gpu-kernels.rs");
const TEMPLATE_PANEL: &str = include_str!("../templates/fragments/panel.rs");

fn json_file(path: &Path) -> Result<Value, String> {
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

fn native_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
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

fn build(dir: &Path, native: bool) -> Result<PathBuf, String> {
    let cargo_manifest = dir.join("Cargo.toml");
    let name = library_name(&cargo_manifest)?;
    let mut command = Command::new(env!("CARGO"));
    command.args(["build", "--release", "--manifest-path"]).arg(&cargo_manifest);
    let built = if native {
        dir.join("target/release").join(native_file(&name))
    } else {
        command.args(["--target", WASM_TARGET]);
        dir.join("target").join(WASM_TARGET).join("release").join(format!("{name}.wasm"))
    };
    if !command.status().map_err(|e| e.to_string())?.success() {
        return Err(format!("{}: the build failed", dir.display()));
    }
    Ok(built)
}

fn assemble(dir: &Path, out: &Path, with_native: bool) -> Result<(PathBuf, Value), String> {
    let manifest = json_file(&dir.join("plugin.json"))?;
    let id = manifest["id"].as_str().ok_or("plugin.json has no `id`")?;
    let wasm_entry = manifest["entry"]["wasm"].as_str();
    if wasm_entry.is_none() && manifest["entry"]["native"].is_null() {
        return Err("plugin.json `entry` needs a `wasm` file".into());
    }
    let bundle = out.join(id);
    let io = |e: std::io::Error| format!("{}: {e}", bundle.display());
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle).map_err(io)?;
    }
    std::fs::create_dir_all(&bundle).map_err(io)?;

    let mut entry = serde_json::Map::new();
    if let Some(file) = wasm_entry {
        let built = build(dir, false)?;
        std::fs::copy(&built, bundle.join(file)).map_err(|e| format!("{}: {e}", built.display()))?;
        entry.insert("wasm".into(), json!(file));
    }
    if with_native || wasm_entry.is_none() {
        let built = build(dir, true)?;
        let file = built.file_name().and_then(|name| name.to_str()).ok_or("the native library has no file name")?.to_string();
        std::fs::copy(&built, bundle.join(&file)).map_err(|e| format!("{}: {e}", built.display()))?;
        entry.insert("native".into(), json!({ native_target(): file }));
    }
    let mut bundled = manifest.clone();
    bundled["entry"] = Value::Object(entry);
    let text = serde_json::to_string_pretty(&bundled).map_err(|e| e.to_string())?;
    std::fs::write(bundle.join("plugin.json"), text).map_err(io)?;
    let assets = dir.join("assets");
    if assets.is_dir() {
        copy_tree(&assets, &bundle.join("assets")).map_err(io)?;
    }
    Ok((bundle, bundled))
}

fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), &format!("{name}/"), out)?;
        } else {
            out.push(name);
        }
    }
    Ok(())
}

fn pack_folder(folder: &Path, file: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut names = Vec::new();
    collect_files(folder, "", &mut names).map_err(io)?;
    names.sort();
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut writer = zip::ZipWriter::new(std::fs::File::create(file).map_err(io)?);
    for name in names {
        writer.start_file(&name, options).map_err(|e| e.to_string())?;
        writer.write_all(&std::fs::read(folder.join(&name)).map_err(io)?).map_err(io)?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn package(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from).ok_or(USAGE)?;
    let native = args.iter().any(|a| a == "--native");
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| root.join("plugins/dist"));
    let (bundle, manifest) = assemble(&dir, &out, native)?;
    let id = manifest["id"].as_str().unwrap_or_default();
    let version = manifest["version"].as_str().ok_or("plugin.json has no `version`")?;
    let file = out.join(format!("{id}-{version}.{PACKAGE_EXTENSION}"));
    pack_folder(&bundle, &file)?;
    println!("{}", bundle.display());
    println!("{}", file.display());
    Ok(())
}

fn valid_name(name: &str) -> bool {
    name.len() <= 40
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn words(name: &str) -> Vec<String> {
    name.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}

fn generate(name: &str, panel: bool, gpu: bool) -> Result<Vec<(&'static str, String)>, String> {
    if !valid_name(name) {
        return Err(format!("`{name}` must start with a lower case letter and use only lower case letters, digits and `-`"));
    }
    let words = words(name);
    let substitutions: BTreeMap<&str, String> = BTreeMap::from([
        ("{{name}}", name.to_string()),
        ("{{snake}}", name.replace('-', "_")),
        ("{{Title}}", words.join(" ")),
        ("{{Ident}}", words.concat()),
        ("{{cx}}", if gpu { "cx" } else { "_cx" }.to_string()),
        ("{{kernels}}", if gpu { TEMPLATE_GPU_KERNELS } else { "" }.to_string()),
        ("{{shader}}", if gpu { ".shader(KERNELS)" } else { "" }.to_string()),
        ("{{layout}}", if gpu { TEMPLATE_GPU_LAYOUT } else { TEMPLATE_CPU_LAYOUT }.trim_end().to_string()),
        ("{{panel}}", if panel { TEMPLATE_PANEL } else { "" }.to_string()),
        ("{{registration}}", if panel { "\n        .panel(\"{{name}}\", \"{{Title}}\", {{Ident}}Panel::default())" } else { "" }.to_string()),
    ]);
    let fill = |template: &str| {
        let mut text = template.to_string();
        for _ in 0..2 {
            for (key, value) in &substitutions {
                text = text.replace(key, value);
            }
        }
        text
    };
    Ok(vec![("Cargo.toml", fill(TEMPLATE_CARGO)), ("plugin.json", fill(TEMPLATE_MANIFEST)), ("src/lib.rs", fill(TEMPLATE_LIB))])
}

fn new(root: &Path, args: &[String]) -> Result<(), String> {
    let name = args.iter().find(|a| !a.starts_with("--")).ok_or(USAGE)?;
    let files = generate(name, args.iter().any(|a| a == "--panel"), args.iter().any(|a| a == "--gpu"))?;
    let dir = root.join("plugins").join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    for (path, text) in files {
        let target = dir.join(path);
        let io = |e: std::io::Error| format!("{}: {e}", target.display());
        std::fs::create_dir_all(target.parent().unwrap_or(&dir)).map_err(io)?;
        std::fs::write(&target, text).map_err(io)?;
    }
    println!("{}", dir.display());
    println!("next: cargo xtask plugin dev plugins/{name}");
    Ok(())
}

fn newest_change(dir: &Path) -> (usize, SystemTime) {
    let mut count = 0;
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name();
            if kind.is_dir() {
                if name != "target" && name != ".git" {
                    stack.push(entry.path());
                }
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                count += 1;
                newest = newest.max(modified);
            }
        }
    }
    (count, newest)
}

fn cli(root: &Path) -> Command {
    if let Some(path) = std::env::var_os("EFFECTCRAFT_CLI") {
        return Command::new(path);
    }
    let built = root.join("target/debug/effectcraft-cli");
    if built.is_file() {
        return Command::new(built);
    }
    let mut command = Command::new(env!("CARGO"));
    command.args(["run", "--quiet", "--package", "effectcraft-cli", "--"]);
    command
}

fn load_into_app(root: &Path, bundle: &Path, bridge: &str) -> Result<(), String> {
    let params = json!({ "path": bundle }).to_string();
    let out = cli(root).args(["exec", "plugin.load", &params, "--bridge", bridge]).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().replace('\n', " ");
    Err(text)
}

fn dev(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = args.iter().find(|a| !a.starts_with("--") && Some(a.as_str()) != bridge_value(args)).map(PathBuf::from).ok_or(USAGE)?;
    let bridge = bridge_value(args).unwrap_or(DEFAULT_BRIDGE);
    let out = root.join("plugins/dist");
    println!("watching {} and loading into the app on port {bridge}", dir.display());
    let mut seen = None;
    loop {
        let stamp = newest_change(&dir);
        if seen != Some(stamp) {
            seen = Some(stamp);
            let started = Instant::now();
            let loaded = assemble(&dir, &out, false).and_then(|(bundle, manifest)| {
                let seconds = started.elapsed().as_secs_f64();
                load_into_app(root, &bundle, bridge).map(|()| (manifest["id"].as_str().unwrap_or_default().to_string(), seconds))
            });
            match loaded {
                Ok((id, seconds)) => println!("reloaded {id}: built in {seconds:.1} s"),
                Err(error) => println!("not reloaded: {error}"),
            }
        }
        std::thread::sleep(POLL);
    }
}

fn bridge_value(args: &[String]) -> Option<&str> {
    args.iter().position(|a| a == "--bridge").and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("new") => new(root, &args[1..]),
        Some("dev") => dev(root, &args[1..]),
        _ => package(root, args),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn folder(dir: &Path) {
        std::fs::create_dir_all(dir.join("assets/lut")).unwrap();
        std::fs::write(dir.join("plugin.json"), r#"{"id":"com.example.t","name":"T","version":"1.2.3","entry":{"wasm":"plugin.wasm"}}"#).unwrap();
        std::fs::write(dir.join("plugin.wasm"), b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(dir.join("assets/lut/a.cube"), "LUT").unwrap();
        std::fs::write(dir.join("assets/b.txt"), "b").unwrap();
    }

    #[test]
    fn the_same_folder_packs_to_the_same_bytes_whatever_the_file_times() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("source");
        folder(&source);
        let first = work.path().join("first.craftplugin");
        pack_folder(&source, &first).unwrap();
        let later = SystemTime::now() + Duration::from_secs(86_400);
        std::fs::File::options().write(true).open(source.join("plugin.wasm")).unwrap().set_modified(later).unwrap();
        let second = work.path().join("second.craftplugin");
        pack_folder(&source, &second).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
    }

    #[test]
    fn a_package_lists_its_files_sorted_with_the_manifest_in_the_root_and_a_fixed_time() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("source");
        folder(&source);
        let file = work.path().join("t.craftplugin");
        pack_folder(&source, &file).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect();
        assert_eq!(names, ["assets/b.txt", "assets/lut/a.cube", "plugin.json", "plugin.wasm"]);
        let mut manifest = String::new();
        archive.by_name("plugin.json").unwrap().read_to_string(&mut manifest).unwrap();
        assert!(manifest.contains("com.example.t"));
        let time = archive.by_name("plugin.wasm").unwrap().last_modified().unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (1980, 1, 1));
    }

    #[test]
    fn a_new_effect_plugin_is_named_from_its_folder_and_has_no_panel_or_kernel() {
        let files = generate("soft-glow", false, false).unwrap();
        let by_path: BTreeMap<_, _> = files.iter().cloned().collect();
        assert!(by_path["plugin.json"].contains(r#""id": "com.example.soft-glow""#));
        assert!(by_path["plugin.json"].contains(r#""name": "Soft Glow""#));
        let lib = &by_path["src/lib.rs"];
        assert!(lib.contains("struct SoftGlowEffect;"));
        assert!(lib.contains("fn layout(&self, _cx: &Cx)"));
        assert!(!lib.contains("Panel") && !lib.contains("KERNELS"));
        assert!(files.iter().all(|(_, text)| !text.contains("{{")));
    }

    #[test]
    fn panel_and_gpu_options_add_a_panel_and_a_kernel_to_the_effect() {
        let files = generate("soft-glow", true, true).unwrap();
        let lib = files.iter().find(|(path, _)| *path == "src/lib.rs").unwrap().1.clone();
        assert!(lib.contains("struct SoftGlowPanel"));
        assert!(lib.contains(".panel(\"soft-glow\", \"Soft Glow\", SoftGlowPanel::default())"));
        assert!(lib.contains("fn soft_glow_apply("));
        assert!(lib.contains(".shader(KERNELS)"));
        assert!(files.iter().all(|(_, text)| !text.contains("{{")));
    }

    #[test]
    fn names_that_cannot_be_a_plugin_id_or_a_rust_name_are_refused() {
        for name in ["Tint", "1tint", "-tint", "tint-", "tint_x", "ti nt", ""] {
            assert!(generate(name, false, false).is_err(), "{name}");
        }
    }
}
