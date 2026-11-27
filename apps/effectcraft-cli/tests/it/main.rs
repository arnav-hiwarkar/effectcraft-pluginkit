//! The crate's integration tests, one binary (each module is a former `tests/<name>.rs`).

#[path = "../../../../crates/plugin/tests/support/build_plugin.rs"]
mod build_plugin;
mod cli;
#[path = "../../../../crates/plugin/tests/support/process_dir.rs"]
mod process_dir;
mod qa_template;

/// Nextest archives relocate binaries; plain Cargo keeps the compile-time path valid.
fn binary() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_effectcraft_cli").unwrap_or_else(|| env!("CARGO_BIN_EXE_effectcraft-cli").into())
}

/// The CLI with its own empty plug-in suite, so no test loads or changes the person's plug-ins.
fn command() -> std::process::Command {
    static CONFIG: std::sync::OnceLock<process_dir::ProcessDir> = std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(process_dir::ProcessDir::new);
    let mut command = std::process::Command::new(binary());
    command.env("EFFECTCRAFT_CONFIG_DIR", config.path().join("Config"));
    command.env("CRAFT_EXT_DIR", config.path().join("Extensions"));
    command
}
