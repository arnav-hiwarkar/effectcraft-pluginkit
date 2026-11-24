use std::sync::RwLock;

use effectcraft_keyframe::Value;

pub type PresetValues = Vec<(String, Value)>;

#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub owner: String,
    pub name: String,
    pub path: String,
}

#[derive(Default)]
struct Store {
    presets: Vec<(String, String, String, PresetValues)>,
    luts: Vec<Lut>,
}

static STORE: RwLock<Store> = RwLock::new(Store { presets: Vec::new(), luts: Vec::new() });

pub fn replace(owner: &str, presets: Vec<(String, String, PresetValues)>, luts: Vec<(String, String)>) {
    let mut store = STORE.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    store.presets.retain(|(known, ..)| known != owner);
    store.luts.retain(|lut| lut.owner != owner);
    store.presets.extend(presets.into_iter().map(|(effect, name, values)| (owner.to_string(), effect, name, values)));
    store.luts.extend(luts.into_iter().map(|(name, path)| Lut { owner: owner.to_string(), name, path }));
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
