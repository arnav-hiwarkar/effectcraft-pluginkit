use serde::{Deserialize, Serialize};

use crate::curve::Curve;

pub const PRESETS_FILE: &str = "presets.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub curve: Curve,
}

pub struct BuiltinPreset {
    pub name: &'static str,
    pub curve: Curve,
}

const fn builtin(name: &'static str, x1: f64, y1: f64, x2: f64, y2: f64) -> BuiltinPreset {
    BuiltinPreset { name, curve: Curve::new(x1, y1, x2, y2) }
}

pub const LINEAR: Curve = Curve::new(1.0 / 6.0, 1.0 / 6.0, 5.0 / 6.0, 5.0 / 6.0);
pub const EASY_EASE: Curve = Curve::new(1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0);
pub const EASE_IN: Curve = Curve::new(0.42, 0.0, 1.0, 1.0);
pub const EASE_OUT: Curve = Curve::new(0.0, 0.0, 0.58, 1.0);

pub const BUILTIN: &[BuiltinPreset] = &[
    BuiltinPreset { name: "Linear", curve: LINEAR },
    builtin("Ease", 0.25, 0.1, 0.25, 1.0),
    BuiltinPreset { name: "Ease In", curve: EASE_IN },
    BuiltinPreset { name: "Ease Out", curve: EASE_OUT },
    builtin("Ease In Out", 0.42, 0.0, 0.58, 1.0),
    BuiltinPreset { name: "Easy Ease", curve: EASY_EASE },
    builtin("Quad In", 0.11, 0.0, 0.5, 0.0),
    builtin("Quad Out", 0.5, 1.0, 0.89, 1.0),
    builtin("Quad In Out", 0.45, 0.0, 0.55, 1.0),
    builtin("Cubic In", 0.32, 0.0, 0.67, 0.0),
    builtin("Cubic Out", 0.33, 1.0, 0.68, 1.0),
    builtin("Cubic In Out", 0.65, 0.0, 0.35, 1.0),
    builtin("Quart In", 0.5, 0.0, 0.75, 0.0),
    builtin("Quart Out", 0.25, 1.0, 0.5, 1.0),
    builtin("Quart In Out", 0.76, 0.0, 0.24, 1.0),
    builtin("Expo In", 0.7, 0.0, 0.84, 0.0),
    builtin("Expo Out", 0.16, 1.0, 0.3, 1.0),
    builtin("Expo In Out", 0.87, 0.0, 0.13, 1.0),
    builtin("Back In", 0.36, 0.0, 0.66, -0.56),
    builtin("Back Out", 0.34, 1.56, 0.64, 1.0),
    builtin("Back In Out", 0.68, -0.6, 0.32, 1.6),
];

pub fn decode(bytes: &[u8]) -> Result<Vec<Preset>, String> {
    let presets: Vec<Preset> = serde_json::from_slice(bytes).map_err(|e| format!("Saved presets could not be read: {e}"))?;
    Ok(presets.into_iter().map(|p| Preset { curve: p.curve.clamped(), ..p }).collect())
}

pub fn encode(presets: &[Preset]) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(presets).map_err(|e| format!("Presets could not be saved: {e}"))
}

pub fn save(presets: &mut Vec<Preset>, name: &str, curve: Curve) {
    let name = name.trim().to_string();
    match presets.iter_mut().find(|p| p.name == name) {
        Some(existing) => existing.curve = curve,
        None => presets.push(Preset { name, curve }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_at_half(name: &str) -> f64 {
        let preset = BUILTIN.iter().find(|p| p.name == name).map(|p| p.curve).unwrap_or_default();
        (preset.value_at(0.5) * 1e6).round() / 1e6
    }

    #[test]
    fn built_in_presets_ease_to_their_published_midpoints() {
        let expected = [
            ("Linear", 0.5),
            ("Ease", 0.802403),
            ("Ease In", 0.315357),
            ("Ease Out", 0.684643),
            ("Ease In Out", 0.5),
            ("Easy Ease", 0.5),
            ("Quad In", 0.251265),
            ("Quad Out", 0.748735),
            ("Quad In Out", 0.5),
            ("Cubic In", 0.127798),
            ("Cubic Out", 0.872202),
            ("Cubic In Out", 0.5),
            ("Quart In", 0.065904),
            ("Quart Out", 0.934096),
            ("Quart In Out", 0.5),
            ("Expo In", 0.028221),
            ("Expo Out", 0.971779),
            ("Expo In Out", 0.5),
            ("Back In", -0.087401),
            ("Back Out", 1.087401),
            ("Back In Out", 0.5),
        ];
        for (name, value) in expected {
            assert_eq!(value_at_half(name), value, "{name}");
        }
    }

    #[test]
    fn every_built_in_preset_stays_inside_the_editor_range() {
        for preset in BUILTIN {
            assert_eq!(preset.curve.clamped(), preset.curve, "{}", preset.name);
        }
    }

    #[test]
    fn user_presets_survive_a_json_round_trip() {
        let presets = vec![
            Preset { name: "Snap".into(), curve: Curve::new(0.9, 0.0, 0.1, 1.0) },
            Preset { name: "Bounce In".into(), curve: Curve::new(0.3, -0.4, 0.7, 1.4) },
        ];
        let bytes = encode(&presets).unwrap_or_default();
        assert_eq!(decode(&bytes), Ok(presets));
    }

    #[test]
    fn decoding_clamps_hand_edited_handles() {
        let bytes = br#"[{"name":"Wild","curve":{"x1":-1.0,"y1":9.0,"x2":0.5,"y2":0.5}}]"#;
        assert_eq!(decode(bytes), Ok(vec![Preset { name: "Wild".into(), curve: Curve::new(0.0, 1.6, 0.5, 0.5) }]));
    }

    #[test]
    fn saving_under_an_existing_name_replaces_its_curve() {
        let mut presets = vec![Preset { name: "Snap".into(), curve: Curve::new(0.9, 0.0, 0.1, 1.0) }];
        save(&mut presets, " Snap ", Curve::new(0.8, 0.0, 0.2, 1.0));
        save(&mut presets, "Drift", Curve::new(0.1, 0.2, 0.3, 0.4));
        assert_eq!(presets.iter().map(|p| (p.name.as_str(), p.curve.x1)).collect::<Vec<_>>(), vec![("Snap", 0.8), ("Drift", 0.1)]);
    }
}
