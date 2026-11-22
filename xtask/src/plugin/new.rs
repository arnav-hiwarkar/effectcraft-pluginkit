use std::path::Path;

use super::USAGE;

const TEMPLATE_CARGO: &str = include_str!("../../templates/Cargo.toml");
const TEMPLATE_MANIFEST: &str = include_str!("../../templates/plugin.json");

pub(super) struct Kind {
    pub name: &'static str,
    lib: &'static str,
    permissions: &'static [&'static str],
    description: &'static str,
    next: &'static str,
}

pub(super) const KINDS: [Kind; 5] = [
    Kind {
        name: "effect",
        lib: include_str!("../../templates/effect.rs"),
        permissions: &[],
        description: "tints the layer",
        next: "apply {{Title}} from Effects & Presets, Color Correction",
    },
    Kind {
        name: "gpu-effect",
        lib: include_str!("../../templates/gpu-effect.rs"),
        permissions: &[],
        description: "tints the layer with a GPU kernel and the same pass on the CPU",
        next: "apply {{Title}} from Effects & Presets, Color Correction",
    },
    Kind {
        name: "panel",
        lib: include_str!("../../templates/panel.rs"),
        permissions: &["document.write"],
        description: "a panel that follows the selection and adds nulls",
        next: "open Window > Plug-ins > {{Title}}",
    },
    Kind {
        name: "tool",
        lib: include_str!("../../templates/tool.rs"),
        permissions: &["document.write"],
        description: "a viewer tool that drops a null where you click",
        next: "pick {{Title}} in the toolbar and click in the Composition viewer",
    },
    Kind {
        name: "task",
        lib: include_str!("../../templates/task.rs"),
        permissions: &["document.read"],
        description: "measures the comp's brightness in a background task with progress",
        next: "open Window > Plug-ins > {{Title}} and press Measure",
    },
];

fn valid_name(name: &str) -> bool {
    name.len() <= 40
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
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

pub(super) fn kind(name: &str) -> Result<&'static Kind, String> {
    KINDS.iter().find(|kind| kind.name == name).ok_or_else(|| {
        let known: Vec<&str> = KINDS.iter().map(|kind| kind.name).collect();
        format!("`--kind {name}` is not a kind of plugin; pick one of {}", known.join(", "))
    })
}

pub(super) fn generate(name: &str, kind: &Kind) -> Result<Vec<(&'static str, String)>, String> {
    if !valid_name(name) {
        return Err(format!("`{name}` must start with a lower case letter and use only lower case letters, digits and single `-`, such as `soft-glow`"));
    }
    let words = words(name);
    let title = words.join(" ");
    let permissions = kind.permissions.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    let substitutions = [
        ("{{name}}", name.to_string()),
        ("{{snake}}", name.replace('-', "_")),
        ("{{Title}}", title.clone()),
        ("{{Ident}}", words.concat()),
        ("{{permissions}}", permissions),
        ("{{description}}", format!("{title}: {}.", kind.description)),
    ];
    let fill = |template: &str| substitutions.iter().fold(template.to_string(), |text, (key, value)| text.replace(key, value));
    Ok(vec![("Cargo.toml", fill(TEMPLATE_CARGO)), ("plugin.json", fill(TEMPLATE_MANIFEST)), ("src/lib.rs", fill(kind.lib)), ("next", fill(kind.next))])
}

pub(super) fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let name = super::positional(args, &["--kind"]).ok_or(USAGE)?;
    let kind = kind(super::option(args, "--kind").unwrap_or("effect"))?;
    let files = generate(name, kind)?;
    let dir = root.join("plugins").join(name);
    if dir.exists() {
        return Err(format!("{} already exists; pick another name or remove it", dir.display()));
    }
    let mut next = String::new();
    for (path, text) in files {
        if path == "next" {
            next = text;
            continue;
        }
        let target = dir.join(path);
        let io = |e: std::io::Error| format!("{}: {e}", target.display());
        std::fs::create_dir_all(target.parent().unwrap_or(&dir)).map_err(io)?;
        std::fs::write(&target, text).map_err(io)?;
    }
    println!("made plugins/{name} from the {} template", kind.name);
    println!("  test it:      cargo test --manifest-path plugins/{name}/Cargo.toml");
    println!("  look at it:   cargo xtask plugin render plugins/{name}");
    println!("  run it live:  cargo xtask plugin dev plugins/{name}   (with the app started by: cargo run -p effectcraft -- --control 9877)");
    println!("  then {next}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn a_new_plugin_is_named_from_its_folder_and_every_placeholder_is_filled_for_every_kind() {
        for kind in &KINDS {
            let files: BTreeMap<_, _> = generate("soft-glow", kind).unwrap().into_iter().collect();
            assert!(files["plugin.json"].contains(r#""id": "com.example.soft-glow""#), "{}", kind.name);
            assert!(files["plugin.json"].contains(r#""name": "Soft Glow""#), "{}", kind.name);
            assert!(files.values().all(|text| !text.contains("{{")), "{}", kind.name);
            let manifest: serde_json::Value = serde_json::from_str(&files["plugin.json"]).unwrap();
            assert_eq!(manifest["permissions"], serde_json::json!(kind.permissions), "{}", kind.name);
        }
        let effect: BTreeMap<_, _> = generate("soft-glow", kind("effect").unwrap()).unwrap().into_iter().collect();
        assert!(effect["src/lib.rs"].contains("struct SoftGlow;"));
        let gpu: BTreeMap<_, _> = generate("soft-glow", kind("gpu-effect").unwrap()).unwrap().into_iter().collect();
        assert!(gpu["src/lib.rs"].contains("fn soft_glow_tint("));
    }

    #[test]
    fn names_that_cannot_be_a_plugin_id_or_a_rust_name_and_unknown_kinds_are_refused_with_the_fix() {
        for name in ["Tint", "1tint", "-tint", "tint-", "tint_x", "ti nt", "a--b", ""] {
            assert!(generate(name, &KINDS[0]).is_err(), "{name}");
        }
        assert_eq!(kind("filter").err().unwrap(), "`--kind filter` is not a kind of plugin; pick one of effect, gpu-effect, panel, tool, task");
    }
}
