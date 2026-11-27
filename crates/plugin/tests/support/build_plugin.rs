use std::path::{Path, PathBuf};
use std::process::Command;

const WASM: &str = "wasm32-unknown-unknown";

pub(crate) fn wasm(source: &Path, name: &str) -> PathBuf {
    build(source, &format!("{}.wasm", name.replace('-', "_")), Some(WASM))
}

pub(crate) fn build(source: &Path, artifact: &str, target: Option<&str>) -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| source.join("target"), PathBuf::from);
    let mut cargo = Command::new(env!("CARGO"));
    cargo.args(["build", "--release", "--quiet", "--manifest-path"]).arg(source.join("Cargo.toml")).arg("--target-dir").arg(&target_dir);
    if let Some(target) = target {
        cargo.args(["--target", target]);
    }
    let status = cargo.status().expect("cargo runs");
    assert!(status.success(), "{} builds {artifact}", source.display());
    let output = target.map_or_else(|| target_dir.clone(), |target| target_dir.join(target));
    output.join("release").join(artifact)
}
