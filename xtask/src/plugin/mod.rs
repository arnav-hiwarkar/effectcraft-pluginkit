mod check;
mod dev;
mod look;
mod new;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const USAGE: &str = "usage: cargo xtask plugin <plugin-dir> [--native] [--out DIR]
       cargo xtask plugin new <name> [--kind effect|gpu-effect|panel|tool|task]
       cargo xtask plugin dev <plugin-dir> [--bridge PORT]
       cargo xtask plugin render <plugin-dir> [--effect ID] [--set ID=VALUE]... [--in IN.png | --pattern NAME] [--size WxH] [--time S] [--scale F] [--rounds N] [--native] [--out OUT.png]
       cargo xtask plugin bench <plugin-dir> [--rounds N]
       cargo xtask plugin check <plugin-dir>";
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const WASM_FLAGS: [&str; 3] = ["simd128", "bulk-memory", "nontrapping-fptoint"];
const WASM_RUSTFLAGS: &str = "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS";
const PACKAGE_EXTENSION: &str = "craftplugin";

fn wasm_rustflags() -> String {
    format!("-C target-feature={}", WASM_FLAGS.map(|flag| format!("+{flag}")).join(","))
}

pub(super) struct Flags<'a> {
    pub positional: &'a str,
    given: Vec<(&'a str, &'a str)>,
}

impl<'a> Flags<'a> {
    pub fn parse(args: &'a [String], valued: &[&str], switches: &[&str]) -> Result<Flags<'a>, String> {
        let valid = || valued.iter().map(|flag| format!("{flag} VALUE")).chain(switches.iter().map(|flag| flag.to_string())).collect::<Vec<_>>();
        let valid = || match valid() {
            flags if flags.is_empty() => "this command takes no flags".to_string(),
            flags => format!("valid flags: {}", flags.join(", ")),
        };
        let (mut positional, mut given) = (None, Vec::new());
        let mut rest = args.iter().map(String::as_str);
        while let Some(arg) = rest.next() {
            if valued.contains(&arg) {
                match rest.next() {
                    Some(value) if !value.starts_with("--") => given.push((arg, value)),
                    _ => return Err(format!("`{arg}` needs a value; {}", valid())),
                }
            } else if switches.contains(&arg) {
                given.push((arg, ""));
            } else if arg.starts_with("--") {
                return Err(format!("unknown flag `{arg}`; {}", valid()));
            } else if positional.replace(arg).is_some() {
                return Err(format!("unexpected argument `{arg}`; give one folder or name\n{USAGE}"));
            }
        }
        Ok(Flags { positional: positional.ok_or(USAGE)?, given })
    }

    pub fn one(&self, name: &str) -> Option<&'a str> {
        self.given.iter().rev().find(|(flag, _)| *flag == name).map(|(_, value)| *value)
    }

    pub fn all(&self, name: &str) -> Vec<&'a str> {
        self.given.iter().filter(|(flag, _)| *flag == name).map(|(_, value)| *value).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.given.iter().any(|(flag, _)| *flag == name)
    }
}

fn plugin_dir(root: &Path, given: &str) -> PathBuf {
    let given = PathBuf::from(given);
    if given.is_relative() && !given.exists() { root.join(given) } else { given }
}

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
        .ok_or_else(|| format!("{}: no `cdylib` library target; add `[lib] crate-type = [\"cdylib\"]` to Cargo.toml", manifest.display()))
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

#[derive(Clone, Copy, PartialEq)]
enum Messages {
    Full,
    Short,
}

fn build(dir: &Path, native: bool, messages: Messages) -> Result<PathBuf, String> {
    let cargo_manifest = dir.join("Cargo.toml");
    let name = library_name(&cargo_manifest)?;
    let mut command = Command::new(env!("CARGO"));
    command.args(["build", "--release", "--manifest-path"]).arg(&cargo_manifest);
    if messages == Messages::Short {
        command.args(["--quiet", "--message-format", "short"]);
    }
    let built = if native {
        dir.join("target/release").join(native_file(&name))
    } else {
        command.args(["--target", WASM_TARGET]).env_remove("CARGO_ENCODED_RUSTFLAGS").env_remove("RUSTFLAGS").env(WASM_RUSTFLAGS, wasm_rustflags());
        dir.join("target").join(WASM_TARGET).join("release").join(format!("{name}.wasm"))
    };
    if !command.status().map_err(|e| e.to_string())?.success() {
        return Err(format!("{}: the build failed; the compiler's messages above say where", dir.display()));
    }
    Ok(built)
}

fn assemble(dir: &Path, out: &Path, with_native: bool, messages: Messages) -> Result<(PathBuf, Value), String> {
    let manifest = json_file(&dir.join("plugin.json"))?;
    let id = manifest["id"].as_str().ok_or("plugin.json has no `id`; add one such as \"com.example.my-effect\"")?;
    let wasm_entry = manifest["entry"]["wasm"].as_str();
    if wasm_entry.is_none() && manifest["entry"]["native"].is_null() {
        return Err("plugin.json `entry` needs a `wasm` file, such as \"entry\": {\"wasm\": \"plugin.wasm\"}".into());
    }
    let bundle = out.join(id);
    let io = |e: std::io::Error| format!("{}: {e}", bundle.display());
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle).map_err(io)?;
    }
    std::fs::create_dir_all(&bundle).map_err(io)?;

    let mut entry = serde_json::Map::new();
    if let Some(file) = wasm_entry {
        let built = build(dir, false, messages)?;
        std::fs::copy(&built, bundle.join(file)).map_err(|e| format!("{}: {e}", built.display()))?;
        entry.insert("wasm".into(), json!(file));
    }
    if with_native || wasm_entry.is_none() {
        let built = build(dir, true, messages)?;
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

fn pack_folder(folder: &Path, file: &Path) -> Result<usize, String> {
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut names = Vec::new();
    collect_files(folder, "", &mut names).map_err(io)?;
    names.sort();
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut writer = zip::ZipWriter::new(std::fs::File::create(file).map_err(io)?);
    for name in &names {
        writer.start_file(name, options).map_err(|e| e.to_string())?;
        writer.write_all(&std::fs::read(folder.join(name)).map_err(io)?).map_err(io)?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(names.len())
}

fn package_file(out: &Path, manifest: &Value) -> Result<PathBuf, String> {
    let id = manifest["id"].as_str().unwrap_or_default();
    let version = manifest["version"].as_str().ok_or("plugin.json has no `version`; add one such as \"0.1.0\"")?;
    Ok(out.join(format!("{id}-{version}.{PACKAGE_EXTENSION}")))
}

fn package(root: &Path, args: &[String]) -> Result<(), String> {
    let flags = Flags::parse(args, &["--out"], &["--native"])?;
    let dir = plugin_dir(root, flags.positional);
    let native = flags.has("--native");
    let out = flags.one("--out").map(PathBuf::from).unwrap_or_else(|| root.join("plugins/dist"));
    let (bundle, manifest) = assemble(&dir, &out, native, Messages::Full)?;
    let file = package_file(&out, &manifest)?;
    pack_folder(&bundle, &file)?;
    println!("{}", bundle.display());
    println!("{}", file.display());
    Ok(())
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("new") => new::run(root, rest),
        Some("dev") => dev::run(root, rest),
        Some("render") => look::render(root, rest),
        Some("bench") => look::bench(root, rest),
        Some("check") => check::run(root, rest),
        Some("help" | "--help" | "-h") | None => Err(USAGE.into()),
        _ => package(root, args),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::time::{Duration, SystemTime};

    use super::*;

    pub(super) fn folder(dir: &Path) {
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
        assert_eq!(pack_folder(&source, &file), Ok(4));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect();
        assert_eq!(names, ["assets/b.txt", "assets/lut/a.cube", "plugin.json", "plugin.wasm"]);
        let mut manifest = String::new();
        archive.by_name("plugin.json").unwrap().read_to_string(&mut manifest).unwrap();
        assert!(manifest.contains("com.example.t"));
        let time = archive.by_name("plugin.wasm").unwrap().last_modified().unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (1980, 1, 1));
    }

    fn rust_blocks(markdown: &str) -> Vec<String> {
        markdown.split("```rust\n").skip(1).filter_map(|rest| rest.split("\n```").next()).map(String::from).collect()
    }

    #[test]
    fn every_rust_block_of_the_quickstart_and_the_cookbook_is_copied_from_code_that_is_compiled_and_tested() {
        let root = crate::root();
        let cookbook = root.join("plugins/cookbook/src");
        let mut compiled: Vec<String> = std::fs::read_dir(&cookbook).unwrap().map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap()).collect();
        for kind in &new::KINDS {
            compiled.extend(new::generate("my-tint", kind).unwrap().into_iter().map(|(_, text)| text));
        }
        for (doc, at_least) in [("docs/plugin-quickstart.md", 3), ("docs/plugin-cookbook.md", 15)] {
            let blocks = rust_blocks(&std::fs::read_to_string(root.join(doc)).unwrap());
            assert!(blocks.len() >= at_least, "{doc} has {} rust blocks", blocks.len());
            for block in blocks {
                assert!(compiled.iter().any(|code| code.contains(&block)), "{doc}: this block is not in plugins/cookbook/src or the templates:\n{block}");
            }
        }
    }

    fn parsed(args: &[&str]) -> Result<Vec<String>, String> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let flags = Flags::parse(&args, &["--effect", "--set"], &["--native"])?;
        Ok([
            flags.positional.to_string(),
            flags.one("--effect").unwrap_or_default().to_string(),
            flags.all("--set").join(" "),
            flags.has("--native").to_string(),
        ]
        .to_vec())
    }

    #[test]
    fn flags_are_read_wherever_they_stand_and_a_wrong_or_incomplete_flag_is_an_error_that_lists_the_valid_ones() {
        assert_eq!(
            parsed(&["--effect", "glow", "plugins/glow", "--set", "radius=2", "--native", "--set", "gain=-1"]).unwrap(),
            ["plugins/glow", "glow", "radius=2 gain=-1", "true"]
        );
        assert_eq!(parsed(&["plugins/glow", "--efect", "glow"]).unwrap_err(), "unknown flag `--efect`; valid flags: --effect VALUE, --set VALUE, --native");
        assert_eq!(parsed(&["plugins/glow", "--effect"]).unwrap_err(), "`--effect` needs a value; valid flags: --effect VALUE, --set VALUE, --native");
        assert_eq!(
            parsed(&["plugins/glow", "--effect", "--native"]).unwrap_err(),
            "`--effect` needs a value; valid flags: --effect VALUE, --set VALUE, --native"
        );
        assert!(parsed(&["plugins/glow", "plugins/halo"]).unwrap_err().starts_with("unexpected argument `plugins/halo`"));
        assert_eq!(parsed(&["--native"]).unwrap_err(), USAGE);
    }
}
