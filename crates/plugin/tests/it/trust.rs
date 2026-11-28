use std::collections::BTreeMap;
use std::path::Path;

use effectcraft_plugin::{PluginInfo, PluginState, Plugins};
use serde_json::json;

fn installed(root: &Path, id: &str) -> (Plugins, tempfile::TempDir) {
    let source = tempfile::tempdir().unwrap();
    std::fs::copy(super::probe_wasm(), source.path().join("probe.wasm")).unwrap();
    let manifest = json!({"id": id, "name": "Trust", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}});
    std::fs::write(source.path().join("plugin.json"), manifest.to_string()).unwrap();
    let plugins = Plugins::open(root, false);
    let shown = plugins.install(source.path()).unwrap();
    super::approve(&plugins, &shown);
    assert!(plugins.get(id).is_some());
    (plugins, source)
}

fn listed(plugins: &Plugins, id: &str) -> PluginInfo {
    plugins.list().into_iter().find(|info| info.id == id).unwrap()
}

#[test]
fn plugins_that_were_live_when_the_app_closed_unexpectedly_stay_off_until_turned_back_on() {
    let _exclusive = super::exclusive();
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-crash";
    let (first, _source) = installed(root.path(), id);
    let running = first.get(id).unwrap().bundle.folder_digest().unwrap();
    drop(first);
    craft_plugin_host::Registry::open(root.path()).mark_running(&BTreeMap::from([(id.to_string(), running)])).unwrap();

    let next = Plugins::open(root.path(), false);
    assert!(next.get(id).is_none());
    let info = listed(&next, id);
    assert_eq!(info.state, PluginState::Quarantined);
    assert_eq!(info.error.as_deref(), Some("it was turned off because the app closed unexpectedly while it ran unsandboxed"));
    super::approve(&next, &info);
    assert_eq!(listed(&next, id).state, PluginState::Running);
}

#[test]
fn an_uninstalled_bundle_stays_on_disk_until_its_running_handle_is_dropped() {
    let _exclusive = super::exclusive();
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-lease";
    let (plugins, source) = installed(root.path(), id);
    let running = plugins.get(id).unwrap();
    let digest = running.bundle.folder_digest().unwrap();
    let folder = root.path().join(id).join(digest);
    assert_eq!(running.bundle.dir, folder);

    plugins.uninstall(id).unwrap();
    assert!(plugins.get(id).is_none());
    assert!(folder.join("probe.wasm").is_file());
    drop(running);

    let manifest = source.path().join("plugin.json");
    let mut contents: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    contents["version"] = json!("1.1.0");
    std::fs::write(manifest, contents.to_string()).unwrap();
    plugins.install(source.path()).unwrap();
    assert!(!folder.exists());
}

#[test]
fn approval_requires_the_reviewed_digest_and_installation() {
    let app = super::app_exclusive("test.trust-approval", &[]);
    let mut session = app.session;
    let id = "test.trust-reviewed";
    let source = tempfile::tempdir().unwrap();
    std::fs::copy(super::probe_wasm(), source.path().join("probe.wasm")).unwrap();
    let manifest = |version| json!({"id": id, "name": "Reviewed", "version": version, "entry": {"wasm": "probe.wasm"}}).to_string();
    let install = |session: &mut effectcraft_engine::Session| session.execute_checked("plugin.install", json!({"path": source.path()})).unwrap();
    let approval = |shown: &serde_json::Value| json!({"id": id, "digest": shown["digest"], "installation": shown["installation"]});
    std::fs::write(source.path().join("plugin.json"), manifest("1.0.0")).unwrap();
    let old = install(&mut session);

    std::fs::write(source.path().join("plugin.json"), manifest("2.0.0")).unwrap();
    let new = install(&mut session);
    assert_ne!(old["digest"], new["digest"]);
    assert!(session.execute_checked("plugin.approve", json!({"id": id})).is_err());
    assert!(session.execute_checked("plugin.approve", approval(&old)).is_err());
    assert!(app.plugins.get(id).is_none());

    app.plugins.uninstall(id).unwrap();
    let reinstalled = install(&mut session);
    assert_eq!(new["digest"], reinstalled["digest"]);
    assert_ne!(new["installation"], reinstalled["installation"]);
    assert!(session.execute_checked("plugin.approve", approval(&new)).is_err());
    assert!(app.plugins.get(id).is_none());
    let listed = session.execute_checked("plugin.list", json!({})).unwrap();
    let shown = listed.as_array().unwrap().iter().find(|item| item["id"] == id).unwrap();
    assert_eq!(approval(shown), approval(&reinstalled));
    session.execute_checked("plugin.approve", approval(shown)).unwrap();
    assert_eq!(app.plugins.get(id).unwrap().bundle.manifest.version, "2.0.0");
    app.plugins.uninstall(id).unwrap();
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn a_native_run_marker_survives_refresh_and_quarantines_the_run_after_an_unclean_exit() {
    let _exclusive = super::exclusive();
    super::native_helper::stage_helper();
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let library = super::render::probe_native();
    let file = library.file_name().unwrap();
    std::fs::copy(&library, source.path().join(file)).unwrap();
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let id = "test.trust-native-marker";
    let manifest = json!({"id": id, "name": "Native", "version": "1.0.0", "entry": {"native": {target: file.to_str().unwrap()}}});
    std::fs::write(source.path().join("plugin.json"), manifest.to_string()).unwrap();
    let first = Plugins::open(root.path(), false);
    let shown = first.install(source.path()).unwrap();
    assert_eq!(super::approve(&first, &shown).state, PluginState::Running);

    first.reload();
    let concurrent = Plugins::open(root.path(), false);
    assert_eq!(listed(&concurrent, id).state, PluginState::Running);
    concurrent.end_run();
    drop(concurrent);
    drop(first);

    let recovered = Plugins::open(root.path(), false);
    assert_eq!(listed(&recovered, id).state, PluginState::Quarantined);
    assert!(recovered.get(id).is_none());
}

#[test]
fn safe_mode_starts_no_plugin() {
    let _exclusive = super::exclusive();
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
    let _exclusive = super::exclusive();
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

fn effect_ids(plugins: &Plugins, id: &str) -> Option<Vec<String>> {
    plugins.get(id).map(|loaded| loaded.plugin.contributions().effects.iter().map(|effect| effect.id.clone()).collect())
}

#[test]
fn a_bundle_replaced_while_it_loads_never_runs_as_the_approved_one() {
    let _exclusive = super::exclusive();
    let lumen = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/lumen-glow-before-host-kernels.wasm")).unwrap();
    let id = "test.trust-replaced";
    for delay in 0..40u64 {
        let root = tempfile::tempdir().unwrap();
        let (plugins, _source) = installed(root.path(), id);
        let approved = effect_ids(&plugins, id).unwrap();
        let target = listed(&plugins, id).path;
        let (replacement, retired) = (root.path().join(".replacement"), root.path().join(".retired"));
        std::fs::create_dir(&replacement).unwrap();
        std::fs::copy(target.join("plugin.json"), replacement.join("plugin.json")).unwrap();
        std::fs::write(replacement.join("probe.wasm"), &lumen).unwrap();
        let swap = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_micros(delay * 250));
            std::fs::rename(&target, &retired).unwrap();
            std::fs::rename(&replacement, &target).unwrap();
        });
        plugins.reload();
        swap.join().unwrap();
        if let Some(running) = effect_ids(&plugins, id) {
            assert_eq!(running, approved, "the replacement ran as the approved plugin (swap after {delay} × 250 µs)");
        }
    }
}

#[test]
fn a_warm_start_runs_no_plugin_and_the_first_use_runs_only_the_one_used() {
    let _exclusive = super::exclusive();
    struct NoHost;
    impl craft_plugin_host::Host for NoHost {}
    let root = tempfile::tempdir().unwrap();
    let ids = ["test.lazy-a", "test.lazy-b", "test.lazy-c"];
    let sources: Vec<_> = ids.iter().map(|id| installed(root.path(), id).1).collect();

    let warm = Plugins::open(root.path(), false);
    assert_eq!(ids.map(|id| warm.get(id).unwrap().plugin.started()), [false; 3]);
    assert_eq!(listed(&warm, ids[1]).state, PluginState::Running);
    assert!(listed(&warm, ids[1]).effects.iter().any(|effect| effect.id.ends_with("invert")));

    let used = warm.get(ids[1]).unwrap();
    let mut cx = craft_plugin_host::RenderCx { size: [2, 1], scale: 1.0, ..Default::default() };
    cx.params.insert("amount".into(), craft_plugin_host::Value::Number(1.0));
    let mut pixels = vec![[0.25, 0.5, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]];
    used.plugin.render("invert", &cx, &craft_plugin_host::Layout::pointwise(), None, &mut pixels, || NoHost).unwrap();
    assert_eq!(pixels, [[0.75, 0.5, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]]);
    assert_eq!(ids.map(|id| warm.get(id).unwrap().plugin.started()), [false, true, false]);
    drop(sources);
}

#[test]
fn a_session_plugin_reloads_the_entry_its_changed_manifest_names() {
    let _exclusive = super::exclusive();
    let plugins = Plugins::open(tempfile::tempdir().unwrap().path(), false);
    let source = tempfile::tempdir().unwrap();
    let id = "test.trust-session-entry";
    std::fs::copy(super::probe_wasm(), source.path().join("probe.wasm")).unwrap();
    std::fs::write(source.path().join("other.wasm"), b"not a module").unwrap();
    let manifest = |entry: &str| json!({"id": id, "name": "Trust", "version": "1.0.0", "entry": {"wasm": entry}}).to_string();
    std::fs::write(source.path().join("plugin.json"), manifest("probe.wasm")).unwrap();
    assert_eq!(plugins.load_for_session(source.path()).unwrap().state, PluginState::Running);

    std::fs::write(source.path().join("plugin.json"), manifest("other.wasm")).unwrap();
    plugins.reload();
    assert_eq!(listed(&plugins, id).state, PluginState::Failed, "the old entry ran under the new manifest");
}

#[test]
fn a_first_use_that_cannot_load_its_module_shows_the_plugin_as_failed_with_the_reason() {
    let _exclusive = super::exclusive();
    struct NoHost;
    impl craft_plugin_host::Host for NoHost {}
    let root = tempfile::tempdir().unwrap();
    let id = "test.trust-first-use";
    let (first, _source) = installed(root.path(), id);
    drop(first);
    let compiled: Vec<_> = walk(root.path()).into_iter().filter(|path| path.extension().is_some_and(|extension| extension == "cwasm")).collect();
    compiled.iter().for_each(|path| std::fs::remove_file(path).unwrap());
    let warm = Plugins::open(root.path(), false);
    std::fs::write(listed(&warm, id).path.join("notes.txt"), "added after start").unwrap();
    let used = warm.get(id).unwrap();
    let cx = craft_plugin_host::RenderCx { size: [1, 1], scale: 1.0, ..Default::default() };
    let mut pixels = vec![[0.25, 0.5, 1.0, 1.0]];
    let failed = used.plugin.render("invert", &cx, &craft_plugin_host::Layout::pointwise(), None, &mut pixels, || NoHost).unwrap_err();
    let info = listed(&warm, id);
    assert_eq!(info.state, PluginState::Failed);
    assert_eq!(info.error, Some(failed.to_string()));
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flat_map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() { walk(&path) } else { vec![path] }
        })
        .collect()
}
