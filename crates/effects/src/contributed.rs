use std::{borrow::Cow, path::Path, sync::RwLock};

use effectcraft_keyframe::Value;

pub type PresetValues = Vec<(String, Value)>;

#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub owner: String,
    pub name: String,
    pub path: String,
    pub file: String,
}

impl Lut {
    pub fn reference(&self) -> String {
        lut_reference(&self.owner, &self.file)
    }
}

const LUT_REFERENCE_PREFIX: &str = "craft-lut:";

pub fn lut_reference(owner: &str, file: &str) -> String {
    format!("{LUT_REFERENCE_PREFIX}{owner}/{file}")
}

#[derive(Default)]
struct Store {
    presets: Vec<(String, String, String, PresetValues)>,
    luts: Vec<Lut>,
}

static STORE: RwLock<Store> = RwLock::new(Store { presets: Vec::new(), luts: Vec::new() });

pub fn replace(owner: &str, presets: Vec<(String, String, PresetValues)>, luts: Vec<(String, String, String)>) {
    let mut store = STORE.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    store.presets.retain(|(known, ..)| known != owner);
    store.presets.extend(presets.into_iter().map(|(effect, name, values)| (owner.to_string(), effect, name, values)));
    let luts: Vec<_> = luts.into_iter().map(|(name, file, path)| Lut { owner: owner.to_string(), name, path, file }).collect();
    if store.luts.iter().filter(|lut| lut.owner == owner).ne(luts.iter()) {
        store.luts.retain(|lut| lut.owner != owner);
        store.luts.extend(luts);
        crate::file_cache::invalidate_lut_frames();
    }
}

pub fn remove(owner: &str) {
    replace(owner, Vec::new(), Vec::new());
}

pub fn presets(effect: &str) -> Vec<(String, PresetValues)> {
    let store = STORE.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    store.presets.iter().filter(|(_, known, ..)| known == effect).map(|(_, _, name, values)| (name.clone(), values.clone())).collect()
}

pub fn luts() -> Vec<Lut> {
    STORE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).luts.clone()
}

fn legacy_reference(path: &Path) -> Option<(&str, &Path)> {
    if !path.is_absolute() {
        return None;
    }
    let plugin_root = |path: &Path| matches!(path.file_name().and_then(|name| name.to_str()), Some("plugins" | "Plugins" | "effectcraft-plugins"));
    for folder in path.ancestors().skip(1) {
        let name = folder.file_name().and_then(|name| name.to_str());
        let Some(parent) = folder.parent() else { continue };
        if name.is_some_and(|name| name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())) && parent.parent().is_some_and(plugin_root) {
            return Some((parent.file_name()?.to_str()?, path.strip_prefix(folder).ok()?));
        }
        if plugin_root(parent) && name.is_some_and(|name| name.contains('.')) {
            return Some((name?, path.strip_prefix(folder).ok()?));
        }
    }
    None
}

pub fn is_lut_reference(src: &str) -> bool {
    src.starts_with(LUT_REFERENCE_PREFIX) || legacy_reference(Path::new(src.trim())).is_some()
}

pub fn resolve_lut(src: &str) -> Option<Cow<'_, str>> {
    let explicit = src.trim().strip_prefix(LUT_REFERENCE_PREFIX);
    let (owner, file) = if let Some(reference) = explicit {
        let (owner, file) = reference.split_once('/')?;
        (owner, Path::new(file))
    } else if let Some(legacy) = legacy_reference(Path::new(src.trim())) {
        legacy
    } else {
        return Some(Cow::Borrowed(src));
    };
    let store = STORE.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = store.luts.iter().filter(|lut| lut.owner == owner);
    let matching = entries.clone().find(|lut| Path::new(&lut.file) == file).or_else(|| {
        if explicit.is_some() {
            return None;
        }
        let name = file.file_name()?;
        let mut by_name = entries.filter(|lut| Path::new(&lut.file).file_name() == Some(name));
        let first = by_name.next()?;
        by_name.next().is_none().then_some(first)
    });
    matching.map(|lut| Cow::Owned(lut.path.clone()))
}
