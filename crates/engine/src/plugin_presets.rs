use effectcraft_keyframe::Value;

use crate::Session;

/// A preset's values by param id. Points are fractions of the layer size, as in bundled presets.
pub type PresetValues = Vec<(String, Value)>;

/// The user's presets for plugin effects, kept where every app of the suite reads them (set by the
/// host that loads plugins). An effect's bundled presets are not in here.
pub trait UserPresets: Send + Sync {
    fn list(&self, effect: &str) -> Vec<(String, PresetValues)>;
    fn save(&self, effect: &str, name: &str, values: &PresetValues) -> Result<(), String>;
    fn delete(&self, effect: &str, name: &str) -> Result<(), String>;
}

impl Session {
    /// Effect `effect`'s presets: the ones it comes with first, then the user's.
    pub fn effect_presets(&self, effect: &str) -> Vec<(String, PresetValues)> {
        let mut presets = effectcraft_effects::presets(effect);
        if let Some(user) = &self.user_presets {
            presets.extend(user.list(effect));
        }
        presets
    }
}
