use craft_plugin_host::{Bundle, Value as Wire};
use effectcraft_engine::effects::{self, contributed};
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::ParamUi;

const MAX_LUT_BYTES: u64 = 256 << 20;

pub(crate) struct Checked {
    presets: Vec<(String, String, contributed::PresetValues)>,
    luts: Vec<(String, String)>,
}

pub(crate) fn check(bundle: &Bundle) -> Result<Checked, String> {
    let manifest = &bundle.manifest;
    let presets = manifest
        .contributes
        .presets
        .iter()
        .map(|preset| Ok((preset.effect.clone(), preset.name.clone(), preset_values(preset)?)))
        .collect::<Result<_, String>>()?;
    let luts = manifest.contributes.luts.iter().map(|file| lut(bundle, file)).collect::<Result<_, String>>()?;
    Ok(Checked { presets, luts })
}

pub(crate) fn register(owner: &str, checked: Checked) {
    contributed::replace(owner, checked.presets, checked.luts);
}

pub(crate) fn unregister(owner: &str) {
    contributed::remove(owner);
}

fn preset_values(preset: &craft_plugin_host::ContributedPreset) -> Result<contributed::PresetValues, String> {
    let (effect, name) = (&preset.effect, &preset.name);
    let spec = effects::find(effect)
        .ok_or_else(|| format!("preset `{name}` is for effect `{effect}`, which is not a built-in effect or an installed plugin's effect"))?;
    preset
        .values
        .iter()
        .map(|(id, wire)| {
            let param = spec.params.iter().find(|param| param.id == id).ok_or_else(|| {
                let known: Vec<&str> = spec.params.iter().map(|param| param.id).collect();
                format!("preset `{name}` sets `{id}`, but effect `{effect}` has no such parameter; it has {}", known.join(", "))
            })?;
            let value = typed(&param.default, &param.ui, wire)
                .ok_or_else(|| format!("preset `{name}`: `{id}` of effect `{effect}` needs {}", wanted(&param.default)))?;
            Ok((id.clone(), value))
        })
        .collect()
}

fn typed(default: &Value, ui: &ParamUi, wire: &Wire) -> Option<Value> {
    Some(match (default, wire) {
        (Value::Scalar(_), Wire::Number(n)) if n.is_finite() => Value::Scalar(*n),
        (Value::Bool(_), Wire::Bool(b)) => Value::Bool(*b),
        (Value::Enum(_), Wire::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => match ui {
            ParamUi::Popup { options } if *n as usize >= options.len() => return None,
            _ => Value::Enum(*n as u32),
        },
        (Value::Vec2(_), Wire::List(v)) => Value::Vec2(v.as_slice().try_into().ok()?),
        (Value::Vec3(_), Wire::List(v)) => Value::Vec3(v.as_slice().try_into().ok()?),
        (Value::Color(_), Wire::List(v)) => {
            Value::Color(v.as_slice().try_into().ok().or_else(|| <[f64; 3]>::try_from(v.as_slice()).ok().map(|[r, g, b]| [r, g, b, 1.0]))?)
        }
        (Value::Str(_), Wire::Text(text)) => Value::Str(text.clone()),
        _ => return None,
    })
}

fn wanted(default: &Value) -> &'static str {
    match default {
        Value::Scalar(_) => "a number",
        Value::Bool(_) => "true or false",
        Value::Enum(_) => "a whole number that is one of its options",
        Value::Vec2(_) => "a list of 2 numbers",
        Value::Vec3(_) => "a list of 3 numbers",
        Value::Color(_) => "a list of 3 or 4 numbers",
        Value::Str(_) => "text",
        _ => "a value presets cannot set",
    }
}

fn lut(bundle: &Bundle, file: &str) -> Result<(String, String), String> {
    let path = bundle.dir.join(file);
    let size = std::fs::metadata(&path).map_err(|e| format!("LUT `{file}`: {e}"))?.len();
    if size > MAX_LUT_BYTES {
        return Err(format!("LUT `{file}` is {size} bytes; the limit is {MAX_LUT_BYTES}"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("LUT `{file}` is not readable text: {e}"))?;
    effects::parse_cube(&text).ok_or_else(|| {
        format!("LUT `{file}` is not a valid .cube file: it needs LUT_1D_SIZE or LUT_3D_SIZE (2 to 256) and exactly that many rows of three numbers")
    })?;
    let stem = file.strip_suffix(".cube").or_else(|| file.strip_suffix(".CUBE")).unwrap_or(file);
    Ok((format!("{} / {stem}", bundle.manifest.name), path.to_string_lossy().into_owned()))
}
