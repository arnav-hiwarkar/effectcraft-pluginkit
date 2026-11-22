use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_plugin::{Pixel, RenderCx, RunMode, Value, native_target};
use craft_plugin_host::{BundleError, Limits, PackageError, Plugin, Registry};
use serde_json::json;
use zip::write::SimpleFileOptions;

use super::{Kind, Recorder, bundle};

fn options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

fn pack(dir: &Path, out: &Path) {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    for name in names {
        writer.start_file(&name, options()).unwrap();
        writer.write_all(&std::fs::read(dir.join(&name)).unwrap()).unwrap();
    }
    writer.finish().unwrap();
}

fn crafted(entries: impl FnOnce(&mut zip::ZipWriter<std::fs::File>)) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("crafted.craftplugin");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
    let manifest = json!({"id": "test.crafted", "name": "C", "version": "1", "entry": {"wasm": "c.wasm"}});
    writer.start_file("plugin.json", options()).unwrap();
    writer.write_all(manifest.to_string().as_bytes()).unwrap();
    writer.start_file("c.wasm", options()).unwrap();
    writer.write_all(b"\0asm").unwrap();
    entries(&mut writer);
    writer.finish().unwrap();
    (dir, file)
}

fn refused(entries: impl FnOnce(&mut zip::ZipWriter<std::fs::File>)) -> PackageError {
    let (_dir, file) = crafted(entries);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let Err(BundleError::Package(error)) = registry.install(&file) else { panic!("the archive was accepted") };
    let leftovers: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(leftovers.is_empty(), "a refused archive left {leftovers:?} behind");
    error
}

#[test]
fn a_packed_plugin_installs_loads_and_renders_a_frame() {
    let (source, _) = bundle(Kind::Wasm, &[]);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("probe-1.0.0.craftplugin");
    pack(source.path(), &file);

    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(&file).unwrap();
    assert_eq!(installed.dir, root.path().join("test.probe"));
    registry.approve(&installed.manifest).unwrap();
    let grants = registry.grants(&installed.manifest).unwrap();

    let limits = Limits { interactive: Duration::from_millis(400), work: Duration::from_millis(400) };
    let plugin = Plugin::load(&registry.running(&installed), grants, root.path().join("data"), limits).unwrap();
    let mut cx = RenderCx { size: [2, 1], scale: 1.0, ..Default::default() };
    cx.params.insert("amount".into(), Value::Number(1.0));
    let mut pixels: Vec<Pixel> = vec![[0.25, 0.5, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]];
    plugin.render("invert", &cx, Some(0), &mut pixels, Recorder::default).unwrap();
    assert_eq!(pixels, [[0.75, 0.5, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]]);
}

#[test]
fn an_absolute_path_in_a_package_is_refused() {
    let error = refused(|w| {
        w.start_file("/tmp/escape.txt", options()).unwrap();
    });
    assert!(matches!(error, PackageError::UnsafePath(name) if name == "/tmp/escape.txt"));
}

#[test]
fn a_path_that_climbs_out_of_the_plugin_folder_is_refused() {
    let error = refused(|w| {
        w.start_file("assets/../../escape.txt", options()).unwrap();
    });
    assert!(matches!(error, PackageError::UnsafePath(name) if name == "assets/../../escape.txt"));
}

#[test]
fn a_symbolic_link_in_a_package_is_refused() {
    let error = refused(|w| {
        w.add_symlink("link", "/etc/passwd", options()).unwrap();
    });
    assert!(matches!(error, PackageError::NotPlain(name) if name == "link"));
}

#[test]
fn a_package_with_too_many_entries_is_refused() {
    let error = refused(|w| {
        for index in 0..4096 {
            w.start_file(format!("many/{index}"), options()).unwrap();
        }
    });
    assert!(matches!(error, PackageError::TooManyEntries));
}

#[test]
fn a_package_that_unpacks_to_more_than_512_mib_is_refused_while_unpacking() {
    let error = refused(|w| {
        let block = vec![0u8; 1024 * 1024];
        for part in 0..2 {
            w.start_file(format!("big/{part}"), options()).unwrap();
            for _ in 0..257 {
                w.write_all(&block).unwrap();
            }
        }
    });
    assert!(matches!(error, PackageError::TooLarge));
}

#[test]
fn a_failed_install_from_a_package_leaves_the_old_version_working() {
    let (source, _) = bundle(Kind::Wasm, &[]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let old = registry.install(source.path()).unwrap();

    let broken = tempfile::tempdir().unwrap();
    let file = broken.path().join("broken.craftplugin");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
    writer.start_file("plugin.json", options()).unwrap();
    writer.write_all(json!({"id": "test.probe", "name": "Probe", "version": "9.9.9", "entry": {"wasm": "probe.wasm"}}).to_string().as_bytes()).unwrap();
    writer.start_file("../escape", options()).unwrap();
    writer.finish().unwrap();

    assert!(registry.install(&file).is_err());
    let kept = registry.bundles().into_iter().flatten().collect::<Vec<_>>();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].manifest.version, "1.0.0");
    assert!(old.dir.join("probe.wasm").is_file());
}

#[test]
fn a_package_without_a_manifest_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.craftplugin");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
    writer.start_file("probe.wasm", options()).unwrap();
    writer.finish().unwrap();
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(Registry::open(root.path()).install(&file), Err(BundleError::Io(path, _)) if path.ends_with("plugin.json")));
    assert!(!root.path().join(".unpacking").exists());
}

fn hybrid_source() -> tempfile::TempDir {
    let (wasm, _) = bundle(Kind::Wasm, &[]);
    let (native, _) = bundle(Kind::Native, &[]);
    let native_file = libloading::library_filename("probe").into_string().unwrap();
    std::fs::copy(native.path().join(&native_file), wasm.path().join(&native_file)).unwrap();
    let manifest = json!({
        "id": "test.probe", "name": "Probe", "version": "1.0.0",
        "entry": {"wasm": "probe.wasm", "native": {native_target(): native_file}},
    });
    std::fs::write(wasm.path().join("plugin.json"), manifest.to_string()).unwrap();
    wasm
}

#[test]
fn a_plugin_with_both_builds_runs_sandboxed_until_the_user_picks_unsandboxed_for_that_version() {
    let source = hybrid_source();
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    let manifest = installed.manifest.clone();
    assert_eq!(registry.grants(&manifest), None, "installed plugins start disabled");

    registry.approve(&manifest).unwrap();
    assert_eq!(registry.grants(&manifest), Some(BTreeSet::new()));
    assert_eq!(registry.mode(&manifest), RunMode::Sandboxed);
    assert_eq!(registry.running(&installed).entry_path().unwrap(), installed.dir.join("probe.wasm"));

    registry.set_unsandboxed(&manifest, true).unwrap();
    assert_eq!(registry.mode(&manifest), RunMode::Unsandboxed);
    assert_eq!(registry.grants(&manifest), Some(BTreeSet::new()));
    let native_file = libloading::library_filename("probe").into_string().unwrap();
    assert_eq!(registry.running(&installed).entry_path().unwrap(), installed.dir.join(native_file));
    assert_eq!(Registry::open(root.path()).mode(&manifest), RunMode::Unsandboxed, "the choice survives a restart");

    let mut next = manifest.clone();
    next.version = "1.0.1".into();
    assert_eq!(registry.mode(&next), RunMode::Sandboxed, "a new version is sandboxed again");
    assert_eq!(registry.grants(&next), Some(BTreeSet::new()), "a new version keeps its sandboxed approval");

    registry.set_unsandboxed(&manifest, false).unwrap();
    assert_eq!(registry.mode(&manifest), RunMode::Sandboxed);
}

#[test]
fn both_builds_of_a_plugin_give_the_same_picture() {
    let source = hybrid_source();
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    registry.approve(&installed.manifest).unwrap();
    let limits = Limits { interactive: Duration::from_millis(400), work: Duration::from_millis(400) };
    let mut cx = RenderCx { size: [2, 1], scale: 1.0, ..Default::default() };
    cx.params.insert("amount".into(), Value::Number(1.0));
    for unsandboxed in [false, true] {
        registry.set_unsandboxed(&installed.manifest, unsandboxed).unwrap();
        let running = registry.running(&installed);
        let plugin = Plugin::load(&running, BTreeSet::new(), root.path().join("data"), limits).unwrap();
        let mut pixels: Vec<Pixel> = vec![[0.25, 0.5, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]];
        plugin.render("invert", &cx, Some(0), &mut pixels, Recorder::default).unwrap();
        assert_eq!(pixels, [[0.75, 0.5, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]], "unsandboxed={unsandboxed}");
    }
}

#[test]
fn unsandboxed_cannot_be_chosen_without_a_native_build_for_this_machine() {
    let (source, _) = bundle(Kind::Wasm, &[]);
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path());
    let installed = registry.install(source.path()).unwrap();
    registry.approve(&installed.manifest).unwrap();
    assert!(matches!(registry.set_unsandboxed(&installed.manifest, true), Err(BundleError::NoNativeBuild(id)) if id == "test.probe"));
    assert_eq!(registry.mode(&installed.manifest), RunMode::Sandboxed);
}
