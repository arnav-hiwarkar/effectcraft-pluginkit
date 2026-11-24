mod aux;
mod data;
mod data_only;
mod forge;
mod gate;
mod gpu;
mod lumen;
mod media;
mod panels;
mod params;
mod render;
mod state;
mod trust;
mod viewer;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use effectcraft_engine::Session;
use effectcraft_plugin::Plugins;
use serde_json::json;

fn probe_wasm() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let probe = craft_plugin_host::testing::probe_dir();
        let status = Command::new(env!("CARGO"))
            .args(["build", "--release", "--quiet", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(probe.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(status.success(), "the probe plugin builds");
        probe.join("target/wasm32-unknown-unknown/release/probe.wasm")
    })
}

static SHARED_PLUGINS: Gate = Gate { readers: Mutex::new(Some(0)), changed: Condvar::new() };

struct Gate {
    readers: Mutex<Option<usize>>,
    changed: Condvar,
}

struct Hold(bool);

impl Gate {
    fn enter(&'static self, exclusive: bool) -> Hold {
        let mut readers = self.readers.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match (*readers, exclusive) {
                (Some(_), false) => break,
                (Some(0), true) => break,
                _ => readers = self.changed.wait(readers).unwrap_or_else(PoisonError::into_inner),
            }
        }
        *readers = if exclusive { None } else { readers.map(|count| count + 1) };
        Hold(exclusive)
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut readers = SHARED_PLUGINS.readers.lock().unwrap_or_else(PoisonError::into_inner);
        *readers = if self.0 { Some(0) } else { readers.map(|count| count - 1) };
        SHARED_PLUGINS.changed.notify_all();
    }
}

fn shared() -> Hold {
    SHARED_PLUGINS.enter(false)
}

fn exclusive() -> Hold {
    SHARED_PLUGINS.enter(true)
}

struct App {
    session: Session,
    plugins: &'static Plugins,
    bundle: tempfile::TempDir,
    id: String,
    _hold: Hold,
}

fn app(id: &str, permissions: &[&str]) -> App {
    open(shared(), id, permissions)
}

fn app_exclusive(id: &str, permissions: &[&str]) -> App {
    open(exclusive(), id, permissions)
}

fn open(hold: Hold, id: &str, permissions: &[&str]) -> App {
    static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("temp dir"));
    let mut session = effectcraft_host::session();
    let plugins = effectcraft_plugin::install(&mut session, Some(config.path()));

    let bundle = tempfile::tempdir().expect("temp dir");
    std::fs::copy(probe_wasm(), bundle.path().join("probe.wasm")).expect("copy the probe");
    let manifest = json!({"id": id, "name": "Probe", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": permissions});
    std::fs::write(bundle.path().join("plugin.json"), manifest.to_string()).expect("write the manifest");
    plugins.load_for_session(bundle.path()).expect("the probe loads");

    session.execute("comp.new", json!({"name": "Test", "width": 64, "height": 36, "frameRate": 24, "duration": 2})).expect("comp.new");
    App { session, plugins, bundle, id: id.to_string(), _hold: hold }
}

impl App {
    fn run(&mut self, command: &str) -> Result<(), String> {
        effectcraft_plugin::run_command(&mut self.session, &self.id, command)
    }

    fn relay(&mut self, command: &str, params: serde_json::Value) -> Result<(), String> {
        let request = json!({"command": command, "params": params});
        std::fs::write(self.bundle.path().join("relay.json"), request.to_string()).expect("write the relayed command");
        self.run("relay")
    }

    fn layer_names(&self) -> Vec<String> {
        self.session.active_comp().map(|comp| comp.layers.iter().map(|layer| layer.name.clone()).collect()).unwrap_or_default()
    }
}
