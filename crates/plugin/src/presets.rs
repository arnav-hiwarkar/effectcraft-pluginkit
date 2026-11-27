use std::sync::Arc;

use craft_ext_store::location;
use craft_plugin_host::{ContributedPreset, EffectSpec, Presets};
use effectcraft_engine::plugin_presets::{PresetValues, UserPresets};

use crate::effects::{preset_values, saved_to_wire};
use crate::manage::effect_owner;

/// User presets for plugin effects in the suite folder, shared with every Craft app.
pub(crate) struct SuitePresets {
    presets: Presets,
}

impl SuitePresets {
    pub(crate) fn open() -> Option<Arc<dyn UserPresets>> {
        match location::default_root() {
            Ok(suite) => Some(Arc::new(SuitePresets { presets: Presets::open(&suite) })),
            Err(error) => {
                log::warn!("plug-in presets are not available: {error}");
                None
            }
        }
    }
}

struct Owned {
    plugin: String,
    spec: EffectSpec,
}

impl Owned {
    fn of(effect: &str) -> Result<Owned, String> {
        let (plugin, loaded, local) = effect_owner(effect)?;
        let spec = loaded
            .plugin
            .contributions()
            .effects
            .iter()
            .find(|spec| spec.id == local)
            .cloned()
            .ok_or_else(|| format!("no running plug-in has the effect `{effect}`"))?;
        Ok(Owned { plugin, spec })
    }

    fn bundled(&self) -> Vec<ContributedPreset> {
        self.spec
            .presets
            .iter()
            .map(|preset| ContributedPreset { effect: self.spec.id.clone(), name: preset.name.clone(), values: preset.values.clone() })
            .collect()
    }
}

impl UserPresets for SuitePresets {
    fn list(&self, effect: &str) -> Vec<(String, PresetValues)> {
        let Ok(owned) = Owned::of(effect) else { return Vec::new() };
        match self.presets.user(&owned.plugin, &owned.spec.id) {
            Ok(found) => found.into_iter().map(|preset| (preset.name, preset_values(&owned.spec, &preset.values))).collect(),
            Err(error) => {
                log::warn!("the presets of `{effect}` were not read: {error}");
                Vec::new()
            }
        }
    }

    fn save(&self, effect: &str, name: &str, values: &PresetValues) -> Result<(), String> {
        let owned = Owned::of(effect)?;
        let wire = values.iter().filter_map(|(id, value)| Some((id.clone(), saved_to_wire(value)?))).collect();
        self.presets.save(&owned.plugin, &owned.spec.id, name, &wire, &owned.bundled()).map_err(|error| error.to_string())
    }

    fn delete(&self, effect: &str, name: &str) -> Result<(), String> {
        let owned = Owned::of(effect)?;
        if owned.bundled().iter().any(|preset| preset.name == name) {
            return Err(format!("the preset `{name}` comes with the plug-in and cannot be removed"));
        }
        self.presets.delete(&owned.plugin, &owned.spec.id, name).map_err(|error| error.to_string())
    }
}
