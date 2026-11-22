use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use craft_plugin::{Contributions, Manifest, Permission};
use craft_plugin_host::testing::TestPlugin;
use craft_plugin_host::{Bundle, PACKAGE_MAX_BYTES, PACKAGE_MAX_ENTRIES};
use serde_json::Value;

use super::look::layers_from_source;
use super::{Messages, USAGE, WASM_FLAGS, assemble, json_file, pack_folder, package_file, plugin_dir, positional};

const PACKAGE_BUDGET: u64 = 32 << 20;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    level: Level,
    what: String,
    fix: String,
}

#[derive(Default)]
struct Report(Vec<Finding>);

impl Report {
    fn error(&mut self, what: impl Into<String>, fix: impl Into<String>) {
        self.0.push(Finding { level: Level::Error, what: what.into(), fix: fix.into() });
    }

    fn warn(&mut self, what: impl Into<String>, fix: impl Into<String>) {
        self.0.push(Finding { level: Level::Warning, what: what.into(), fix: fix.into() });
    }

    fn errors(&self) -> usize {
        self.0.iter().filter(|finding| finding.level == Level::Error).count()
    }
}

fn manifest_fields(raw: &Value, cargo: &str, report: &mut Report) {
    let names: Vec<&str> = Permission::ALL.iter().map(|p| p.name()).collect();
    for asked in raw["permissions"].as_array().into_iter().flatten() {
        let asked = asked.as_str().unwrap_or_default();
        if !names.contains(&asked) {
            report.error(
                format!("plugin.json asks for the unknown permission `{asked}`; the app refuses to load the plugin"),
                format!("use one of {}", names.join(", ")),
            );
        }
    }
    if raw["description"].as_str().is_none_or(|text| text.trim().is_empty()) {
        report.warn("plugin.json has no description", "add \"description\": one sentence that says what the plugin does; the manager shows it");
    }
    let licensed = raw["license"].as_str().is_some_and(|text| !text.trim().is_empty()) || cargo.lines().any(|line| line.trim_start().starts_with("license"));
    if !licensed {
        report.warn("the plugin has no licence", "add \"license\": \"MIT OR Apache-2.0\" (or your own) to plugin.json");
    }
}

pub(super) fn wasm_features(module: &[u8]) -> Option<BTreeSet<String>> {
    let leb = |bytes: &[u8], at: &mut usize| -> Option<usize> {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
    };
    let mut at = 8;
    while at < module.len() {
        let id = module[at];
        at += 1;
        let size = leb(module, &mut at)?;
        let end = at.checked_add(size)?;
        if id == 0 {
            let mut inner = at;
            let name_len = leb(module, &mut inner)?;
            if module.get(inner..inner + name_len)? == b"target_features" {
                inner += name_len;
                let count = leb(module, &mut inner)?;
                let mut features = BTreeSet::new();
                for _ in 0..count {
                    let prefix = *module.get(inner)?;
                    inner += 1;
                    let len = leb(module, &mut inner)?;
                    let name = std::str::from_utf8(module.get(inner..inner + len)?).ok()?;
                    inner += len;
                    if prefix == b'+' {
                        features.insert(name.to_string());
                    }
                }
                return Some(features);
            }
        }
        at = end;
    }
    Some(BTreeSet::new())
}

fn module(bundle: &Path, manifest: &Manifest, report: &mut Report) {
    let Some(file) = &manifest.entry.wasm else { return };
    let Ok(bytes) = std::fs::read(bundle.join(file)) else {
        report.error(format!("the module `{file}` named in plugin.json is missing"), "build the plugin with cargo xtask plugin <dir>");
        return;
    };
    let features = wasm_features(&bytes).unwrap_or_default();
    let missing: Vec<&str> = WASM_FLAGS.iter().copied().filter(|flag| !features.contains(*flag)).collect();
    if !missing.is_empty() {
        report.warn(
            format!("`{file}` was built without {}; it runs, but slower", missing.join(", ")),
            "build it with cargo xtask plugin <dir>, which passes -C target-feature=+simd128,+bulk-memory,+nontrapping-fptoint",
        );
    }
}

fn package(bundle: &Path, raw: &Value, report: &mut Report) -> Result<(), String> {
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = package_file(scratch.path(), raw)?;
    let entries = pack_folder(bundle, &file)?;
    let unpacked: u64 = walk(bundle).iter().filter_map(|path| path.metadata().ok()).map(|meta| meta.len()).sum();
    let packed = file.metadata().map(|meta| meta.len()).unwrap_or_default();
    if entries > PACKAGE_MAX_ENTRIES {
        report.error(format!("the package holds {entries} files; the app refuses more than {PACKAGE_MAX_ENTRIES}"), "pack assets into fewer files");
    }
    if unpacked > PACKAGE_MAX_BYTES {
        report.error(
            format!("the package unpacks to {} MiB; the app refuses more than {} MiB", unpacked >> 20, PACKAGE_MAX_BYTES >> 20),
            "ship fewer or smaller assets",
        );
    } else if packed > PACKAGE_BUDGET {
        report.warn(
            format!("the package is {} MiB", packed >> 20),
            format!("keep it under {} MiB so it installs quickly: compress or drop assets", PACKAGE_BUDGET >> 20),
        );
    }
    Ok(())
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => out.push(entry.path()),
                Err(_) => {}
            }
        }
    }
    out.sort();
    out
}

fn contributions(contributions: &Contributions, report: &mut Report) {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut each = |kind: &'static str, id: &str| {
        if let Some(before) = seen.insert(format!("{kind} {id}"), kind) {
            report.error(format!("two {before}s share the id `{id}`; the app shows only one"), format!("give each {kind} its own id"));
        }
    };
    contributions.effects.iter().for_each(|e| each("effect", &e.id));
    contributions.panels.iter().for_each(|p| each("panel", &p.id));
    contributions.commands.iter().for_each(|c| each("command", &c.id));
    contributions.tasks.iter().for_each(|t| each("task", &t.id));
    contributions.tools.iter().for_each(|t| each("tool", &t.id));
    for effect in &contributions.effects {
        let mut ids = BTreeSet::new();
        for param in &effect.params {
            if !ids.insert(param.id.as_str()) {
                report.error(format!("effect `{}` declares the parameter `{}` twice", effect.id, param.id), "rename one; projects store values by id");
            }
        }
    }
}

fn other_plugins(root: &Path, dir: &Path, id: &str, report: &mut Report) {
    let own = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    for entry in std::fs::read_dir(root.join("plugins")).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.canonicalize().is_ok_and(|other| other == own) {
            continue;
        }
        let Ok(raw) = json_file(&path.join("plugin.json")) else { continue };
        if raw["id"].as_str() == Some(id) {
            report.error(
                format!("plugin id `{id}` is also used by {}", path.display()),
                "change `id` in plugin.json; two plugins with one id replace each other",
            );
        }
    }
}

fn smoke_renders(plugin: &TestPlugin, report: &mut Report) {
    for spec in &plugin.plugin().contributions().effects {
        let render = plugin.render(&spec.id);
        let [width, height] = [render.source().width, render.source().height];
        let source = render.source();
        if let Err(error) = layers_from_source(plugin, &spec.id, render, &source).run() {
            report.error(
                format!("effect `{}` fails to render at its default settings on a {width}x{height} test image: {error}", spec.id),
                format!("run cargo xtask plugin render <dir> --effect {} and fix the error; the app shows it on the layer instead of the effect", spec.id),
            );
        }
    }
}

fn check(root: &Path, dir: &Path) -> Result<Report, String> {
    let dir = dir.to_path_buf();
    let mut report = Report::default();
    let raw = json_file(&dir.join("plugin.json"))?;
    let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
    manifest_fields(&raw, &cargo, &mut report);
    let names: Vec<&str> = Permission::ALL.iter().map(|p| p.name()).collect();
    let mut known = raw.clone();
    if let Some(asked) = known["permissions"].as_array_mut() {
        asked.retain(|permission| permission.as_str().is_some_and(|name| names.contains(&name)));
    }
    let manifest = match Manifest::parse(&known.to_string()) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            report.error(error.to_string(), "fix plugin.json; the app refuses to load it until then");
            None
        }
    };
    if let Some(manifest) = &manifest {
        other_plugins(root, &dir, &manifest.id, &mut report);
        let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
        let bundle_dir = if cargo.is_empty() { dir.clone() } else { assemble(&dir, scratch.path(), false, Messages::Short)?.0 };
        if !cargo.is_empty() && known != raw {
            let mut bundled = json_file(&bundle_dir.join("plugin.json"))?;
            bundled["permissions"] = known["permissions"].clone();
            std::fs::write(bundle_dir.join("plugin.json"), bundled.to_string()).map_err(|e| e.to_string())?;
        }
        let bundle = Bundle::open(&bundle_dir).map_err(|e| e.to_string())?;
        module(&bundle_dir, &bundle.manifest, &mut report);
        package(&bundle_dir, &raw, &mut report)?;
        match TestPlugin::load(&bundle) {
            Ok(plugin) => {
                let described = plugin.plugin().contributions();
                contributions(described, &mut report);
                if let Err(error) = craft_plugin_host::validate(manifest, described) {
                    report.error(format!("the app refuses the plugin: {error}"), "change the id the message names");
                }
                smoke_renders(&plugin, &mut report);
            }
            Err(error) => report.error(format!("the plugin does not load: {error}"), "run its tests (cargo test) to see where it fails"),
        }
    }
    report.0.sort();
    report.0.dedup();
    Ok(report)
}

pub(super) fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = plugin_dir(root, positional(args, &[]).ok_or(USAGE)?);
    let report = check(root, &dir)?;
    for finding in &report.0 {
        let level = if finding.level == Level::Error { "error" } else { "warning" };
        println!("{level}: {}\n  fix: {}", finding.what, finding.fix);
    }
    let (errors, warnings) = (report.errors(), report.0.len() - report.errors());
    println!("checked {}: {errors} errors, {warnings} warnings", dir.display());
    if errors > 0 { Err(format!("{errors} errors")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_refuses_what_the_app_refuses_and_a_render_that_fails_but_not_a_parameter_named_in_a_comment() {
        let root = crate::root();
        let report = check(&root, &root.join("xtask/tests/check-fixture")).unwrap();
        let errors: Vec<&str> = report.0.iter().filter(|f| f.level == Level::Error).map(|f| f.what.as_str()).collect();
        assert_eq!(
            errors,
            [
                "effect `broken` fails to render at its default settings on a 320x180 test image: the lookup table is empty",
                "the app refuses the plugin: effect id `Bad` may only use lower case letters, digits, `-` and `_`",
            ]
        );
    }

    #[test]
    fn the_target_features_section_of_a_module_lists_the_flags_it_was_built_with() {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        let mut section = vec![15];
        section.extend_from_slice(b"target_features");
        section.push(2);
        section.extend_from_slice(b"+\x07simd128");
        section.extend_from_slice(b"-\x0bbulk-memory");
        module.push(0);
        module.push(section.len() as u8);
        module.extend_from_slice(&section);
        assert_eq!(wasm_features(&module), Some(BTreeSet::from(["simd128".to_string()])));
        assert_eq!(wasm_features(b"\0asm\x01\0\0\0"), Some(BTreeSet::new()));
    }

    #[test]
    fn unknown_permissions_are_errors_and_a_missing_description_or_licence_are_warnings_with_the_fix() {
        let mut report = Report::default();
        manifest_fields(&serde_json::json!({ "permissions": ["clipbard", "clipboard"] }), "[package]\nname = \"x\"\n", &mut report);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.0[0].what, "plugin.json asks for the unknown permission `clipbard`; the app refuses to load the plugin");
        assert_eq!(report.0[0].fix, "use one of document.read, document.write, files.read, files.write, clipboard");
        assert_eq!(report.0.iter().map(|f| f.what.as_str()).skip(1).collect::<Vec<_>>(), ["plugin.json has no description", "the plugin has no licence"]);
    }
}
