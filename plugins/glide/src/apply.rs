use serde_json::{Value, json};

use crate::bridge::Host;
use crate::curve::{Curve, Ease};
use crate::selection::{self, Pair, PropertyKeys, SelectedKey};

pub const APPLY_LABEL: &str = "Glide: Apply Curve";
const PATH_SAMPLES: usize = 128;
const NOTHING_SELECTED: &str = "Select two or more neighbouring keyframes on a property.";
const ONLY_SINGLE_KEYS: &str = "Each property has only one selected keyframe: select two or more neighbouring keyframes.";
const NO_NEIGHBOURS: &str = "The selected keyframes are not neighbours: select keyframes that follow each other.";
const NO_CURVE: &str = "The first selected keyframes hold or keep the same value, so there is no curve to read.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    In,
    Out,
}

pub struct Segment<'a> {
    pub pair: Pair<'a>,
    pub deltas: Vec<f64>,
}

struct Gathered<'a> {
    segments: Vec<Segment<'a>>,
    single: usize,
    apart: usize,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn selected(host: &dyn Host) -> Result<Vec<PropertyKeys>, String> {
    selection::parse(&host.command("keys.info", json!({}))?)
}

fn gather<'a>(host: &dyn Host, properties: &'a [PropertyKeys]) -> Result<Gathered<'a>, String> {
    let mut gathered = Gathered { segments: Vec::new(), single: 0, apart: 0 };
    for property in properties {
        if property.keys.len() < 2 {
            gathered.single += 1;
            continue;
        }
        let times = selection::key_times(&host.command("prop.get", json!({"layer": property.layer, "prop": property.prop}))?)?;
        let pairs = property.pairs(&times);
        if pairs.is_empty() {
            gathered.apart += 1;
        }
        for pair in pairs {
            let deltas = deltas(host, &pair)?;
            gathered.segments.push(Segment { pair, deltas });
        }
    }
    Ok(gathered)
}

fn missing_pairs(gathered: &Gathered) -> &'static str {
    match (gathered.single, gathered.apart) {
        (_, apart) if apart > 0 => NO_NEIGHBOURS,
        (single, _) if single > 0 => ONLY_SINGLE_KEYS,
        _ => NOTHING_SELECTED,
    }
}

fn deltas(host: &dyn Host, pair: &Pair) -> Result<Vec<f64>, String> {
    if pair.property.spatial {
        return Ok(vec![path_length(host, pair)?]);
    }
    Ok(pair.component_deltas().unwrap_or_else(|| vec![1.0]))
}

fn path_length(host: &dyn Host, pair: &Pair) -> Result<f64, String> {
    let (from, to) = (pair.start.comp_time, pair.end.comp_time);
    let steps: Vec<Value> = (0..=PATH_SAMPLES)
        .map(|i| {
            let time = from + (to - from) * i as f64 / PATH_SAMPLES as f64;
            json!({"command": "prop.get", "params": {"layer": pair.property.layer, "prop": pair.property.prop, "time": time}})
        })
        .collect();
    let reply = host.command("engine.batch", json!({"label": "Glide: Measure Path", "steps": steps}))?;
    let points = reply
        .get("results")
        .and_then(Value::as_array)
        .map(|results| results.iter().filter_map(|r| r.get("value").and_then(Value::as_array)).map(|v| v.iter().filter_map(Value::as_f64).collect()).collect::<Vec<Vec<f64>>>())
        .filter(|points| points.len() == PATH_SAMPLES + 1)
        .ok_or_else(|| format!("Could not measure the motion path of {}.", pair.property.name))?;
    Ok(arc_length(&points).max(pair.chord()))
}

fn polyline(points: impl Iterator<Item = Vec<f64>> + Clone) -> f64 {
    points.clone().zip(points.skip(1)).map(|(a, b)| a.iter().zip(&b).map(|(p, q)| (q - p) * (q - p)).sum::<f64>().sqrt()).sum()
}

pub fn arc_length(points: &[Vec<f64>]) -> f64 {
    let fine = polyline(points.iter().cloned());
    let coarse = polyline(points.iter().step_by(2).cloned());
    (4.0 * fine - coarse) / 3.0
}

fn set_ease(property: &PropertyKeys, key: &SelectedKey, side: Side, dim: usize, ease: Ease) -> Value {
    json!({
        "command": "keys.setEase",
        "params": {
            "layer": property.layer,
            "prop": property.prop,
            "time": key.time,
            "side": if side == Side::Out { "out" } else { "in" },
            "dim": dim,
            "speed": ease.speed,
            "influence": ease.influence * 100.0,
        }
    })
}

fn same_key(a: &SelectedKey, b: &SelectedKey) -> bool {
    std::ptr::eq(a, b)
}

fn kept_auto_sides(segments: &[Segment]) -> Vec<Value> {
    let mut steps = Vec::new();
    for segment in segments {
        let property = segment.pair.property;
        let (start, end) = (segment.pair.start, segment.pair.end);
        let ends_a_segment = |key: &SelectedKey| segments.iter().any(|s| same_key(s.pair.end, key));
        let starts_a_segment = |key: &SelectedKey| segments.iter().any(|s| same_key(s.pair.start, key));
        if start.auto_bezier && !ends_a_segment(start) {
            steps.extend(start.in_ease.iter().enumerate().filter_map(|(d, e)| e.map(|e| set_ease(property, start, Side::In, d, e))));
        }
        if end.auto_bezier && !starts_a_segment(end) {
            steps.extend(end.out_ease.iter().enumerate().filter_map(|(d, e)| e.map(|e| set_ease(property, end, Side::Out, d, e))));
        }
    }
    steps
}

fn release_flags(segments: &[Segment]) -> Option<Value> {
    let keys = || segments.iter().flat_map(|s| [s.pair.start, s.pair.end]);
    let continuous = keys().any(|k| k.continuous);
    let roving = keys().any(|k| k.roving);
    let mut params = serde_json::Map::new();
    if continuous {
        params.insert("continuous".into(), json!(false));
    }
    if roving {
        params.insert("roving".into(), json!(false));
    }
    (!params.is_empty()).then(|| json!({"command": "keys.interpolation", "params": params}))
}

pub fn plan(curve: Curve, segments: &[Segment]) -> Vec<Value> {
    let mut steps: Vec<Value> = release_flags(segments).into_iter().collect();
    steps.extend(kept_auto_sides(segments));
    for segment in segments {
        let (property, duration) = (segment.pair.property, segment.pair.duration());
        for (dim, delta) in segment.deltas.iter().enumerate() {
            let slope = delta / duration;
            steps.push(set_ease(property, segment.pair.start, Side::Out, dim, curve.outgoing(slope)));
            steps.push(set_ease(property, segment.pair.end, Side::In, dim, curve.incoming(slope)));
        }
    }
    steps
}

fn summary(gathered: &Gathered) -> String {
    let segments = &gathered.segments;
    let mut keys: Vec<(u64, u64, u64)> = segments
        .iter()
        .flat_map(|s| [s.pair.start, s.pair.end].map(|k| (s.pair.property.layer, s.pair.property.prop, k.time.to_bits())))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let mut properties: Vec<(u64, u64)> = keys.iter().map(|(layer, prop, _)| (*layer, *prop)).collect();
    properties.dedup();
    let applied = format!("Applied to {} on {}", plural(keys.len(), "key", "keys"), plural(properties.len(), "property", "properties"));
    let mut skipped = Vec::new();
    if gathered.single > 0 {
        skipped.push(format!("{} with one selected key", plural(gathered.single, "property", "properties")));
    }
    if gathered.apart > 0 {
        skipped.push(format!("{} whose selected keys are not neighbours", plural(gathered.apart, "property", "properties")));
    }
    if skipped.is_empty() { applied } else { format!("{applied}; skipped {}", skipped.join(" and ")) }
}

pub fn apply(host: &dyn Host, curve: Curve, label: &str) -> Result<String, String> {
    let properties = selected(host)?;
    let gathered = gather(host, &properties)?;
    if gathered.segments.is_empty() {
        return Err(missing_pairs(&gathered).into());
    }
    let steps = plan(curve, &gathered.segments);
    host.command("engine.batch", json!({"label": label, "steps": steps}))?;
    Ok(summary(&gathered))
}

pub fn curve_of(segment: &Segment) -> Option<Curve> {
    let (start, end, duration) = (segment.pair.start, segment.pair.end, segment.pair.duration());
    segment.deltas.iter().enumerate().find_map(|(dim, delta)| {
        let outgoing = start.out_ease.get(dim).copied().flatten()?;
        let incoming = end.in_ease.get(dim).copied().flatten()?;
        Curve::from_eases(outgoing, incoming, delta / duration)
    })
}

pub fn read(host: &dyn Host) -> Result<(Curve, String), String> {
    let properties = selected(host)?;
    for property in properties.iter().filter(|p| p.keys.len() >= 2) {
        let times = selection::key_times(&host.command("prop.get", json!({"layer": property.layer, "prop": property.prop}))?)?;
        if let Some(pair) = property.pairs(&times).into_iter().next() {
            let segment = Segment { deltas: deltas(host, &pair)?, pair };
            return curve_of(&segment).map(|curve| (curve, property.name.clone())).ok_or_else(|| NO_CURVE.to_string());
        }
    }
    let any_pairs_possible = properties.iter().any(|p| p.keys.len() >= 2);
    Err(if any_pairs_possible { NO_NEIGHBOURS } else if properties.is_empty() { NOTHING_SELECTED } else { ONLY_SINGLE_KEYS }.into())
}
