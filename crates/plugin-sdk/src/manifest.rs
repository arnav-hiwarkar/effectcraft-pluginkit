use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "plugin.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    pub entry: Entry,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    Wasm(String),
    Native(BTreeMap<String, String>),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    #[serde(rename = "document.read")]
    DocumentRead,
    #[serde(rename = "document.write")]
    DocumentWrite,
    #[serde(rename = "files.read")]
    FilesRead,
    #[serde(rename = "files.write")]
    FilesWrite,
}

impl Permission {
    pub const ALL: [Permission; 4] = [Permission::DocumentRead, Permission::DocumentWrite, Permission::FilesRead, Permission::FilesWrite];

    pub fn name(self) -> &'static str {
        match self {
            Permission::DocumentRead => "document.read",
            Permission::DocumentWrite => "document.write",
            Permission::FilesRead => "files.read",
            Permission::FilesWrite => "files.write",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Permission::DocumentRead => "Read the open project: compositions, layers, properties and selection",
            Permission::DocumentWrite => "Edit the open project: create and change layers, effects and keyframes",
            Permission::FilesRead => "Read files outside the plugin's own folders",
            Permission::FilesWrite => "Write files outside the plugin's own folders",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    Json(String),
    Id(String),
    Field(&'static str),
    EntryPath(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Json(e) => write!(f, "plugin.json is not valid: {e}"),
            ManifestError::Id(id) => {
                write!(f, "plugin id `{id}` must be reverse-domain, lower case, such as com.example.glow")
            }
            ManifestError::Field(name) => write!(f, "plugin.json field `{name}` is empty"),
            ManifestError::EntryPath(path) => {
                write!(f, "entry `{path}` must be a file name inside the plugin folder")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let manifest: Manifest = serde_json::from_str(text).map_err(|e| ManifestError::Json(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if !valid_id(&self.id) {
            return Err(ManifestError::Id(self.id.clone()));
        }
        if self.name.trim().is_empty() {
            return Err(ManifestError::Field("name"));
        }
        if self.version.trim().is_empty() {
            return Err(ManifestError::Field("version"));
        }
        let paths: Vec<&String> = match &self.entry {
            Entry::Wasm(path) => vec![path],
            Entry::Native(by_target) => by_target.values().collect(),
        };
        if paths.is_empty() {
            return Err(ManifestError::Field("entry"));
        }
        match paths.into_iter().find(|path| !valid_entry_path(path)) {
            Some(path) => Err(ManifestError::EntryPath(path.clone())),
            None => Ok(()),
        }
    }

    pub fn sandboxed(&self) -> bool {
        matches!(self.entry, Entry::Wasm(_))
    }

    pub fn entry_file(&self) -> Option<&str> {
        match &self.entry {
            Entry::Wasm(path) => Some(path),
            Entry::Native(by_target) => by_target.get(&native_target()).map(String::as_str),
        }
    }
}

pub fn native_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn valid_id(id: &str) -> bool {
    let segment_ok = |s: &str| !s.is_empty() && !s.starts_with('-') && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    id.len() <= 128 && id.split('.').count() >= 2 && id.split('.').all(segment_ok)
}

fn valid_entry_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('.') && path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<Manifest, ManifestError> {
        Manifest::parse(json)
    }

    #[test]
    fn a_wasm_plugin_is_sandboxed_and_keeps_its_permissions() {
        let m = manifest(
            r#"{"id":"com.example.glow","name":"Glow","version":"1.0.0","entry":{"wasm":"plugin.wasm"},
                "permissions":["document.read","files.write"]}"#,
        )
        .unwrap();
        assert!(m.sandboxed());
        assert_eq!(m.entry_file(), Some("plugin.wasm"));
        assert_eq!(m.permissions.into_iter().collect::<Vec<_>>(), [Permission::DocumentRead, Permission::FilesWrite]);
    }

    #[test]
    fn a_native_plugin_resolves_the_library_for_this_machine_only() {
        let json = format!(
            r#"{{"id":"com.example.native","name":"N","version":"1","entry":{{"native":{{"{}":"libn.dylib","plan9-mips":"n.so"}}}}}}"#,
            native_target()
        );
        let m = manifest(&json).unwrap();
        assert!(!m.sandboxed());
        assert_eq!(m.entry_file(), Some("libn.dylib"));
    }

    #[test]
    fn ids_and_entry_paths_that_could_escape_the_folder_are_refused() {
        let with = |id: &str, entry: &str| manifest(&format!(r#"{{"id":"{id}","name":"N","version":"1","entry":{{"wasm":"{entry}"}}}}"#));
        assert!(matches!(with("glow", "p.wasm"), Err(ManifestError::Id(_))));
        assert!(matches!(with("Com.Example.Glow", "p.wasm"), Err(ManifestError::Id(_))));
        assert!(matches!(with("com.example/../x", "p.wasm"), Err(ManifestError::Id(_))));
        assert!(matches!(with("com.example.glow", "../p.wasm"), Err(ManifestError::EntryPath(_))));
        assert!(matches!(with("com.example.glow", "/abs/p.wasm"), Err(ManifestError::EntryPath(_))));
        assert!(matches!(with("com.example.glow", "sub/p.wasm"), Err(ManifestError::EntryPath(_))));
    }

    #[test]
    fn an_unknown_permission_is_an_error_not_a_silent_grant() {
        let r = manifest(r#"{"id":"com.example.glow","name":"N","version":"1","entry":{"wasm":"p.wasm"},"permissions":["network"]}"#);
        assert!(matches!(r, Err(ManifestError::Json(_))));
    }
}
