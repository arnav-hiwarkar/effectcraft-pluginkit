use serde::Deserialize;
use serde_json::Value;

use crate::curve::Ease;

const SAME_TIME: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq)]
pub enum KeyValue {
    Components(Vec<f64>),
    Opaque,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedKey {
    pub index: usize,
    pub segment_length: Option<f64>,
    pub time: f64,
    pub comp_time: f64,
    pub value: KeyValue,
    pub continuous: bool,
    pub auto_bezier: bool,
    pub roving: bool,
    pub in_ease: Vec<Option<Ease>>,
    pub out_ease: Vec<Option<Ease>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropertyKeys {
    pub layer: u64,
    pub prop: u64,
    pub name: String,
    pub spatial: bool,
    pub keys: Vec<SelectedKey>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair<'a> {
    pub property: &'a PropertyKeys,
    pub start: &'a SelectedKey,
    pub end: &'a SelectedKey,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEase {
    speed: f64,
    influence: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawKey {
    layer: u64,
    prop: u64,
    #[serde(default)]
    name: String,
    index: usize,
    #[serde(default)]
    segment_length: Option<f64>,
    time: f64,
    comp_time: f64,
    value: Value,
    #[serde(default)]
    spatial: Option<String>,
    #[serde(default)]
    continuous: bool,
    #[serde(default)]
    auto_bezier: bool,
    #[serde(default)]
    roving: bool,
    #[serde(default)]
    in_ease: Vec<Option<RawEase>>,
    #[serde(default)]
    out_ease: Vec<Option<RawEase>>,
}

fn ease(raw: Option<RawEase>) -> Option<Ease> {
    raw.map(|e| Ease { speed: e.speed, influence: e.influence / 100.0 })
}

fn key_value(value: &Value) -> KeyValue {
    match value {
        Value::Number(n) => n.as_f64().map_or(KeyValue::Opaque, |v| KeyValue::Components(vec![v])),
        Value::Array(items) => items.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>().map_or(KeyValue::Opaque, KeyValue::Components),
        _ => KeyValue::Opaque,
    }
}

pub fn parse(info: &Value) -> Result<Vec<PropertyKeys>, String> {
    let raw: Vec<RawKey> = serde_json::from_value(info.clone()).map_err(|e| format!("Unexpected keyframe info: {e}"))?;
    let mut properties: Vec<PropertyKeys> = Vec::new();
    for key in raw {
        let selected = SelectedKey {
            index: key.index,
            segment_length: key.segment_length,
            time: key.time,
            comp_time: key.comp_time,
            value: key_value(&key.value),
            continuous: key.continuous,
            auto_bezier: key.auto_bezier,
            roving: key.roving,
            in_ease: key.in_ease.into_iter().map(ease).collect(),
            out_ease: key.out_ease.into_iter().map(ease).collect(),
        };
        match properties.iter_mut().find(|p| p.layer == key.layer && p.prop == key.prop) {
            Some(property) => property.keys.push(selected),
            None => properties.push(PropertyKeys {
                layer: key.layer,
                prop: key.prop,
                name: key.name,
                spatial: key.spatial.is_some(),
                keys: vec![selected],
            }),
        }
    }
    for property in &mut properties {
        property.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        property.keys.dedup_by(|a, b| (a.time - b.time).abs() < SAME_TIME);
    }
    Ok(properties)
}

impl PropertyKeys {
    pub fn pairs(&self) -> Vec<Pair<'_>> {
        self.keys.windows(2).filter(|w| w[1].index == w[0].index + 1).map(|w| Pair { property: self, start: &w[0], end: &w[1] }).collect()
    }
}

impl Pair<'_> {
    pub fn duration(&self) -> f64 {
        self.end.time - self.start.time
    }

    pub fn component_deltas(&self) -> Option<Vec<f64>> {
        match (&self.start.value, &self.end.value) {
            (KeyValue::Components(a), KeyValue::Components(b)) if a.len() == b.len() => {
                Some(a.iter().zip(b).map(|(a, b)| b - a).collect())
            }
            _ => None,
        }
    }

    pub fn chord(&self) -> f64 {
        self.component_deltas().map_or(0.0, |d| d.iter().map(|v| v * v).sum::<f64>().sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> Value {
        json!([
            {"layer": 4, "prop": 15, "name": "Opacity", "index": 1, "time": 2.002, "compTime": 2.002, "value": 100.0, "spatial": null,
             "continuous": false, "autoBezier": false, "roving": false,
             "inEase": [{"speed": 49.95, "influence": 16.666666666666664}], "outEase": [null]},
            {"layer": 4, "prop": 9, "name": "Position", "index": 0, "time": 0.0, "compTime": 0.5, "value": [100.0, 100.0, 0.0], "spatial": "autoBezier",
             "continuous": true, "autoBezier": false, "roving": false, "inEase": [null], "outEase": [{"speed": 224.9, "influence": 50.0}]},
            {"layer": 4, "prop": 15, "name": "Opacity", "index": 0, "time": 0.0, "compTime": 0.0, "value": 0.0, "spatial": null,
             "continuous": false, "autoBezier": true, "roving": false, "inEase": [null], "outEase": [{"speed": 0.0, "influence": 50.0}]}
        ])
    }

    #[test]
    fn selected_keys_group_by_property_in_time_order() {
        let properties = parse(&info()).unwrap_or_default();
        let summary: Vec<_> = properties.iter().map(|p| (p.name.as_str(), p.spatial, p.keys.iter().map(|k| k.time).collect::<Vec<_>>())).collect();
        assert_eq!(summary, vec![("Opacity", false, vec![0.0, 2.002]), ("Position", true, vec![0.0])]);
        let opacity_first = &properties[0].keys[0];
        assert_eq!(opacity_first.out_ease, vec![Some(Ease { speed: 0.0, influence: 0.5 })]);
        assert!(opacity_first.auto_bezier);
        assert_eq!(properties[1].keys[0].comp_time, 0.5);
        assert_eq!(properties[1].keys[0].value, KeyValue::Components(vec![100.0, 100.0, 0.0]));
    }

    #[test]
    fn non_numeric_values_are_eased_as_overall_progress() {
        assert_eq!(key_value(&json!({"vertices": []})), KeyValue::Opaque);
        assert_eq!(key_value(&json!([1.0, "x"])), KeyValue::Opaque);
        assert_eq!(key_value(&json!(3.5)), KeyValue::Components(vec![3.5]));
    }

    #[test]
    fn only_neighbouring_selected_keys_form_pairs() {
        let key = |time: f64| SelectedKey {
            index: time as usize,
            segment_length: None,
            time,
            comp_time: time,
            value: KeyValue::Components(vec![time]),
            continuous: false,
            auto_bezier: false,
            roving: false,
            in_ease: vec![],
            out_ease: vec![],
        };
        let property = PropertyKeys { layer: 1, prop: 2, name: "Rotation".into(), spatial: false, keys: vec![key(0.0), key(1.0), key(3.0), key(4.0)] };
        let pairs = property.pairs();
        let spans: Vec<_> = pairs.iter().map(|p| (p.start.time, p.end.time)).collect();
        assert_eq!(spans, vec![(0.0, 1.0), (3.0, 4.0)]);
    }
}
