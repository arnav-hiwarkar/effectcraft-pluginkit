use std::path::{Path, PathBuf};
use std::process::Command;

const KIT_URL: &str = "https://github.com/arnav-hiwarkar/craft-libs-plugin-beta";
const KIT_PACKAGES: [&str; 3] = ["craft-plugin", "craft-plugin-host", "craft-plugin-ui"];
const SKIPPED_DIRS: [&str; 3] = ["target", ".git", "node_modules"];

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    match args {
        [flag] if flag == "--check" => check(root),
        [sha] if !sha.starts_with('-') => bump(root, sha, true),
        _ => Err("usage: cargo xtask kit-rev <40-hex sha> | --check".into()),
    }
}

fn bump(root: &Path, sha: &str, refresh: bool) -> Result<(), String> {
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(format!("kit-rev: `{sha}` is not a full 40-character lowercase hex commit"));
    }
    let mut changed = Vec::new();
    for manifest in kit_manifests(root)? {
        let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let (new, count) = replace_revs(&text, sha);
        if new != text {
            std::fs::write(&manifest, new).map_err(|e| format!("{}: {e}", manifest.display()))?;
            println!("{}: {count} pin(s) set to {sha}", display(root, &manifest));
            changed.push(manifest);
        }
    }
    if changed.is_empty() {
        println!("kit-rev: every manifest already pins {sha}");
    }
    if refresh {
        for manifest in kit_manifests(root)? {
            refresh_lock(root, &manifest)?;
        }
    }
    Ok(())
}

fn check(root: &Path) -> Result<(), String> {
    let manifests = kit_manifests(root)?;
    let workspace = root.join("Cargo.toml");
    let want = pinned_revs(&std::fs::read_to_string(&workspace).map_err(|e| format!("Cargo.toml: {e}"))?)
        .into_iter()
        .next()
        .ok_or("kit-rev: the workspace Cargo.toml pins no craft-libs rev")?;
    let mut drift = Vec::new();
    for manifest in &manifests {
        let text = std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let other: Vec<String> = pinned_revs(&text).into_iter().filter(|r| *r != want).collect();
        if !other.is_empty() {
            drift.push(format!("{} pins {}", display(root, manifest), other.join(", ")));
        }
    }
    if drift.is_empty() {
        println!("kit-rev: {} manifests all pin {want}", manifests.len());
        return Ok(());
    }
    Err(format!("kit-rev: the workspace Cargo.toml pins {want} but {}; run `cargo xtask kit-rev {want}` or bump all together", drift.join("; ")))
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

fn kit_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    walk(root, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !SKIPPED_DIRS.contains(&name) {
                walk(&path, found)?;
            }
        } else if path.file_name().is_some_and(|n| n == "Cargo.toml") && !pinned_revs(&std::fs::read_to_string(&path).unwrap_or_default()).is_empty() {
            found.push(path);
        }
    }
    Ok(())
}

fn is_kit_pin(line: &str) -> bool {
    !line.trim_start().starts_with('#') && line.contains(KIT_URL) && line.contains("rev = \"")
}

fn rev_span(line: &str) -> Option<(usize, usize)> {
    let start = line.find("rev = \"")? + "rev = \"".len();
    let len = line[start..].find('"')?;
    Some((start, start + len))
}

fn pinned_revs(text: &str) -> Vec<String> {
    text.lines().filter(|l| is_kit_pin(l)).filter_map(|l| rev_span(l).map(|(a, b)| l[a..b].to_string())).collect()
}

fn replace_revs(text: &str, sha: &str) -> (String, usize) {
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        match rev_span(line).filter(|_| is_kit_pin(line)) {
            Some((a, b)) => {
                out.push_str(&line[..a]);
                out.push_str(sha);
                out.push_str(&line[b..]);
                count += 1;
            }
            None => out.push_str(line),
        }
    }
    (out, count)
}

fn locked_kit_packages(lock: &str) -> Vec<&'static str> {
    KIT_PACKAGES.into_iter().filter(|p| lock.lines().any(|l| l.trim() == format!("name = \"{p}\""))).collect()
}

fn refresh_lock(root: &Path, manifest: &Path) -> Result<(), String> {
    let lock = manifest.with_file_name("Cargo.lock");
    let Ok(text) = std::fs::read_to_string(&lock) else { return Ok(()) };
    let packages = locked_kit_packages(&text);
    if packages.is_empty() {
        return Ok(());
    }
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.arg("update").arg("--manifest-path").arg(manifest);
    for p in &packages {
        cmd.arg("-p").arg(p);
    }
    let status = cmd.status().map_err(|e| format!("cargo update: {e}"))?;
    if !status.success() {
        return Err(format!("cargo update failed for {}", display(root, manifest)));
    }
    println!("{}: refreshed {}", display(root, &lock), packages.join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";

    fn manifest(rev: &str) -> String {
        format!(
            "[dependencies]\ncraft-plugin = {{ git = \"{KIT_URL}\", rev = \"{rev}\" }}\nserde = \"1\"\ncraft-plugin-host = {{ git = \"{KIT_URL}\", rev = \"{rev}\", features = [\"testing\"] }}\n# [patch.\"{KIT_URL}\"]\n"
        )
    }

    fn fixture(second_rev: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), manifest(OLD)).unwrap();
        std::fs::create_dir_all(dir.path().join("plugins/a")).unwrap();
        std::fs::write(dir.path().join("plugins/a/Cargo.toml"), manifest(second_rev)).unwrap();
        std::fs::create_dir_all(dir.path().join("target/x")).unwrap();
        std::fs::write(dir.path().join("target/x/Cargo.toml"), manifest("0000000000000000000000000000000000000000")).unwrap();
        dir
    }

    #[test]
    fn rewrites_every_manifest_and_leaves_target_alone() {
        let dir = fixture(OLD);
        bump(dir.path(), NEW, false).unwrap();
        for rel in ["Cargo.toml", "plugins/a/Cargo.toml"] {
            let text = std::fs::read_to_string(dir.path().join(rel)).unwrap();
            assert_eq!(pinned_revs(&text), [NEW, NEW], "{rel}");
            assert!(text.contains("features = [\"testing\"]") && text.contains("serde = \"1\""), "{rel}");
        }
        let skipped = std::fs::read_to_string(dir.path().join("target/x/Cargo.toml")).unwrap();
        assert!(skipped.contains("0000000000000000000000000000000000000000"));
        assert!(check(dir.path()).is_ok());
    }

    #[test]
    fn check_names_the_drifting_manifest() {
        let dir = fixture("3333333333333333333333333333333333333333");
        let e = check(dir.path()).unwrap_err();
        assert!(e.contains("plugins/a/Cargo.toml") && !e.contains("target"), "{e}");
        assert!(e.contains(OLD) && e.contains("3333333333333333333333333333333333333333"), "{e}");
    }

    #[test]
    fn rejects_a_short_or_non_hex_sha() {
        let dir = fixture(OLD);
        for bad in ["94fa5dc", &"G".repeat(40), &OLD.to_uppercase().replace('1', "A")] {
            assert!(bump(dir.path(), bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_locked_kit_packages_are_refreshed() {
        let lock = "[[package]]\nname = \"craft-plugin\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"craft-plugin-host\"\n";
        assert_eq!(locked_kit_packages(lock), ["craft-plugin", "craft-plugin-host"]);
    }

    #[test]
    fn the_repository_pins_one_rev() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert!(check(&root).is_ok(), "{:?}", check(&root));
    }
}
