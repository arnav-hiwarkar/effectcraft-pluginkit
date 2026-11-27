use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn stage_helper() {
    static STAGED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STAGED.get_or_init(|| {
        if std::env::var_os("CRAFT_PLUGIN_HELPER").is_some() {
            return;
        }
        let built = build_helper();
        let test = std::env::current_exe().expect("the test executable has a path");
        let beside_test = test.with_file_name("craft-plugin-helper");
        let staged = beside_test.with_extension(format!("{}", std::process::id()));
        link(&built, &staged);
        std::fs::rename(&staged, &beside_test).expect("the helper is staged beside the test executable");
    });
}

fn build_helper() -> PathBuf {
    let output = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--package", "effectcraft-plugin-helper", "--message-format=json-render-diagnostics"])
        .stderr(std::process::Stdio::inherit())
        .output()
        .expect("cargo runs");
    assert!(output.status.success(), "effectcraft-plugin-helper builds");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact" && message["target"]["name"] == "craft-plugin-helper")
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .expect("cargo reports the helper executable")
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("the helper link is created");
}

#[cfg(not(unix))]
fn link(target: &Path, link: &Path) {
    std::fs::copy(target, link).expect("the helper is copied");
}
