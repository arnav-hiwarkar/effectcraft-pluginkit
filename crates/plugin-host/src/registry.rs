use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use craft_plugin::{MANIFEST_FILE, Manifest, ManifestError, Permission, RunMode, native_target};

use crate::package::{self, PackageError};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "state.json";
const DATA_DIR: &str = ".data";
const UNPACKING_DIR: &str = ".unpacking";

pub(crate) fn install_root(data_folder: &Path) -> Option<&Path> {
    data_folder.file_name().is_some_and(|name| name == DATA_DIR).then(|| data_folder.parent()).flatten()
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("this plugin has no build for {0}")]
    NoBuildForThisMachine(String),
    #[error("plugin `{0}` is not installed")]
    NotInstalled(String),
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("plugin `{0}` has no build for this machine, so it cannot run unsandboxed")]
    NoNativeBuild(String),
    #[error("the plugin's entry file {0} is missing")]
    MissingEntry(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub mode: RunMode,
}

impl Bundle {
    pub fn open(dir: &Path) -> Result<Bundle, BundleError> {
        let file = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| BundleError::Io(file, e))?;
        let manifest = Manifest::parse(&text)?;
        let mode = manifest.mode(false);
        Ok(Bundle { dir: dir.to_path_buf(), manifest, mode })
    }

    pub fn manifest_of(source: &Path) -> Result<Manifest, BundleError> {
        if source.is_file() {
            return Ok(Manifest::parse(&package::manifest_text(source)?)?);
        }
        Bundle::open(source).map(|bundle| bundle.manifest)
    }

    pub fn with_mode(mut self, mode: RunMode) -> Bundle {
        self.mode = mode;
        self
    }

    pub fn entry_path(&self) -> Result<PathBuf, BundleError> {
        self.manifest.entry_file(self.mode).map(|file| self.dir.join(file)).ok_or_else(|| BundleError::NoBuildForThisMachine(native_target()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Consent {
    pub enabled: bool,
    #[serde(default)]
    pub granted: BTreeSet<Permission>,
    #[serde(default)]
    pub unsandboxed: Option<String>,
}

pub struct Registry {
    root: PathBuf,
    consents: BTreeMap<String, Consent>,
}

impl Registry {
    pub fn open(root: &Path) -> Registry {
        let consents = std::fs::read_to_string(root.join(STATE_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        Registry { root: root.to_path_buf(), consents }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn bundles(&self) -> Vec<Result<Bundle, BundleError>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.file_name().is_some_and(|name| !name.to_string_lossy().starts_with('.')))
            .filter(|path| path.join(MANIFEST_FILE).is_file())
            .collect();
        dirs.sort();
        dirs.iter().map(|dir| Bundle::open(dir)).collect()
    }

    pub fn consent(&self, id: &str) -> Option<&Consent> {
        self.consents.get(id)
    }

    pub fn mode(&self, manifest: &Manifest) -> RunMode {
        let chosen = self.consents.get(&manifest.id).is_some_and(|consent| consent.unsandboxed.as_ref() == Some(&manifest.version));
        manifest.mode(chosen)
    }

    pub fn grants(&self, manifest: &Manifest) -> Option<BTreeSet<Permission>> {
        let consent = self.consents.get(&manifest.id)?;
        let covered = manifest.permissions.is_subset(&consent.granted);
        let trusted = self.mode(manifest) == RunMode::Sandboxed || consent.unsandboxed.as_ref() == Some(&manifest.version);
        (consent.enabled && covered && trusted).then(|| manifest.permissions.clone())
    }

    pub fn running(&self, bundle: &Bundle) -> Bundle {
        bundle.clone().with_mode(self.mode(&bundle.manifest))
    }

    pub fn data_dir(&self, id: &str) -> PathBuf {
        self.root.join(DATA_DIR).join(id)
    }

    pub fn install(&mut self, source: &Path) -> Result<Bundle, BundleError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |e| BundleError::Io(path, e)
        };
        std::fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let unpacking = self.root.join(UNPACKING_DIR);
        if unpacking.exists() {
            std::fs::remove_dir_all(&unpacking).map_err(io(&unpacking))?;
        }
        let from_file = source.is_file();
        let unpacked = if from_file {
            package::unpack(source, &unpacking).map_err(BundleError::from).and_then(|()| Bundle::open(&unpacking))
        } else {
            Bundle::open(source)
        };
        let incoming = match unpacked.and_then(|bundle| {
            let entry = bundle.entry_path()?;
            if entry.is_file() { Ok(bundle) } else { Err(BundleError::MissingEntry(entry)) }
        }) {
            Ok(bundle) => bundle,
            Err(error) => {
                if from_file {
                    let _ = std::fs::remove_dir_all(&unpacking);
                }
                return Err(error);
            }
        };
        let target = self.root.join(&incoming.manifest.id);
        let staging = self.root.join(format!(".installing-{}", incoming.manifest.id));
        let retired = self.root.join(format!(".replaced-{}", incoming.manifest.id));
        for leftover in [&staging, &retired] {
            if leftover.exists() {
                std::fs::remove_dir_all(leftover).map_err(io(leftover))?;
            }
        }
        if from_file {
            std::fs::rename(&unpacking, &staging).map_err(io(&staging))?;
        } else {
            copy_dir(source, &staging).map_err(io(&staging))?;
        }
        let replaces = target.exists();
        if replaces {
            std::fs::rename(&target, &retired).map_err(io(&target))?;
        }
        if let Err(error) = std::fs::rename(&staging, &target) {
            if replaces {
                std::fs::rename(&retired, &target).map_err(io(&target))?;
            }
            return Err(BundleError::Io(target, error));
        }
        if replaces {
            std::fs::remove_dir_all(&retired).map_err(io(&retired))?;
        }
        Bundle::open(&target)
    }

    pub fn uninstall(&mut self, id: &str) -> Result<(), BundleError> {
        let target = self.installed_dir(id)?;
        let mut consents = self.consents.clone();
        consents.remove(id);
        self.commit(consents)?;
        std::fs::remove_dir_all(&target).map_err(|e| BundleError::Io(target, e))
    }

    pub fn approve(&mut self, manifest: &Manifest) -> Result<(), BundleError> {
        let unsandboxed = (!manifest.has_wasm()).then(|| manifest.version.clone());
        let consent = Consent { enabled: true, granted: manifest.permissions.clone(), unsandboxed };
        let mut consents = self.consents.clone();
        consents.insert(manifest.id.clone(), consent);
        self.commit(consents)
    }

    pub fn set_unsandboxed(&mut self, manifest: &Manifest, unsandboxed: bool) -> Result<(), BundleError> {
        if unsandboxed && !manifest.can_choose_unsandboxed() {
            return Err(BundleError::NoNativeBuild(manifest.id.clone()));
        }
        if !manifest.has_wasm() {
            return Ok(());
        }
        let mut consents = self.consents.clone();
        let consent = consents.entry(manifest.id.clone()).or_default();
        consent.unsandboxed = unsandboxed.then(|| manifest.version.clone());
        self.commit(consents)
    }

    pub fn disable(&mut self, id: &str) -> Result<(), BundleError> {
        self.installed_dir(id)?;
        let mut consents = self.consents.clone();
        consents.entry(id.to_string()).or_default().enabled = false;
        self.commit(consents)
    }

    fn installed_dir(&self, id: &str) -> Result<PathBuf, BundleError> {
        self.bundles()
            .into_iter()
            .flatten()
            .find(|bundle| bundle.manifest.id == id)
            .map(|bundle| bundle.dir)
            .ok_or_else(|| BundleError::NotInstalled(id.to_string()))
    }

    fn commit(&mut self, consents: BTreeMap<String, Consent>) -> Result<(), BundleError> {
        let file = self.root.join(STATE_FILE);
        let staging = self.root.join(format!("{STATE_FILE}.new"));
        let text = serde_json::to_string_pretty(&consents).unwrap_or_default();
        std::fs::create_dir_all(&self.root)
            .and_then(|()| std::fs::write(&staging, text))
            .and_then(|()| std::fs::rename(&staging, &file))
            .map_err(|e| BundleError::Io(file, e))?;
        self.consents = consents;
        Ok(())
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
