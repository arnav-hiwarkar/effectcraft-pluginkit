use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use craft_plugin::{MANIFEST_FILE, Manifest, ManifestError, Permission, native_target};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "state.json";
const DATA_DIR: &str = ".data";

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Bundle {
    pub fn open(dir: &Path) -> Result<Bundle, BundleError> {
        let file = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| BundleError::Io(file, e))?;
        Ok(Bundle { dir: dir.to_path_buf(), manifest: Manifest::parse(&text)? })
    }

    pub fn entry_path(&self) -> Result<PathBuf, BundleError> {
        self.manifest.entry_file().map(|file| self.dir.join(file)).ok_or_else(|| BundleError::NoBuildForThisMachine(native_target()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Consent {
    pub enabled: bool,
    #[serde(default)]
    pub granted: BTreeSet<Permission>,
    #[serde(default)]
    pub unsandboxed: bool,
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

    pub fn grants(&self, manifest: &Manifest) -> Option<BTreeSet<Permission>> {
        let consent = self.consents.get(&manifest.id)?;
        let covered = manifest.permissions.is_subset(&consent.granted);
        let trusted = manifest.sandboxed() || consent.unsandboxed;
        (consent.enabled && covered && trusted).then(|| manifest.permissions.clone())
    }

    pub fn data_dir(&self, id: &str) -> PathBuf {
        self.root.join(DATA_DIR).join(id)
    }

    pub fn install(&mut self, source: &Path) -> Result<Bundle, BundleError> {
        let incoming = Bundle::open(source)?;
        incoming.entry_path()?;
        let target = self.root.join(&incoming.manifest.id);
        let staging = self.root.join(format!(".installing-{}", incoming.manifest.id));
        let retired = self.root.join(format!(".replaced-{}", incoming.manifest.id));
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |e| BundleError::Io(path, e)
        };
        for leftover in [&staging, &retired] {
            if leftover.exists() {
                std::fs::remove_dir_all(leftover).map_err(io(leftover))?;
            }
        }
        copy_dir(source, &staging).map_err(io(&staging))?;
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
        let consent = Consent { enabled: true, granted: manifest.permissions.clone(), unsandboxed: !manifest.sandboxed() };
        let mut consents = self.consents.clone();
        consents.insert(manifest.id.clone(), consent);
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
