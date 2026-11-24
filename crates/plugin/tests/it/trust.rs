use std::collections::BTreeSet;
use std::path::Path;

use effectcraft_plugin::{PluginInfo, PluginState, Plugins};
use serde_json::json;

fn installed(root: &Path, id: &str) -> (Plugins, tempfile::TempDir) {
    let source = tempfile::tempdir().unwrap();
    std::fs::copy(super::probe_wasm(), source.path().join("probe.wasm")).unwrap();
    let manifest = json!({"id": id, "name": "Trust", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}});
    std::fs::write(source.path().join("plugin.json"), manifest.to_string()).unwrap();
    let plugins = Plugins::open(root, false);
    plugins.install(source.path()).unwrap();
    plugins.approve(id).unwrap();
    assert!(plugins.get(id).is_some());
    (plugins, source)
}

fn listed(plugins: &Plugins, id: &str) -> PluginInfo {
    plugins.list().into_iter().find(|info| info.id == id).unwrap()
}

#[test]
fn plugins_that_were_live_when_the_app_closed_unexpectedly_stay_off_until_turned_back_on() {
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-crash";
    let (first, _source) = installed(root.path(), id);
    drop(first);
    craft_plugin_host::Registry::open(root.path()).mark_running(&BTreeSet::from([id.to_string()])).unwrap();

    let next = Plugins::open(root.path(), false);
    assert!(next.get(id).is_none());
    let info = listed(&next, id);
    assert_eq!(info.state, PluginState::Quarantined);
    assert_eq!(info.error.as_deref(), Some("it was turned off because the app closed unexpectedly while it ran unsandboxed"));
    next.approve(id).unwrap();
    assert_eq!(listed(&next, id).state, PluginState::Running);
}

#[test]
fn safe_mode_starts_no_plugin() {
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-safe";
    let (first, _source) = installed(root.path(), id);
    drop(first);

    let safe = Plugins::open(root.path(), true);
    assert!(safe.get(id).is_none());
    let info = listed(&safe, id);
    assert_eq!(info.state, PluginState::Disabled);
    assert_eq!(info.error.as_deref(), Some("EffectCraft started in safe mode, so no plug-ins were started"));
}

#[test]
fn a_plugin_whose_files_changed_after_approval_waits_for_approval_again() {
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-changed";
    let (plugins, _source) = installed(root.path(), id);
    std::fs::write(listed(&plugins, id).path.join("notes.txt"), "added after approval").unwrap();
    plugins.reload();

    assert!(plugins.get(id).is_none());
    let info = listed(&plugins, id);
    assert_eq!(info.state, PluginState::NeedsApproval);
    assert!(info.error.as_deref().unwrap_or_default().starts_with("its files changed since you approved them (approved 1.0.0 "), "{:?}", info.error);
}
