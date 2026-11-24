use std::collections::{BTreeMap, BTreeSet};

use effectcraft_effects::dynamic;
use effectcraft_project::{GroupKind, Node, PluginUse, Project};
use serde_json::{Value, json};

fn effect_ids(p: &Project) -> BTreeSet<&str> {
    let groups = p.comps().flat_map(|(_, c)| c.layers.iter()).filter_map(|l| l.effects()).flat_map(|fx| &fx.children);
    groups
        .filter_map(|n| match n {
            Node::Group(g) => match &g.kind {
                GroupKind::Effect { effect } => Some(effect.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn recorded_owner<'a>(p: &'a Project, effect: &str) -> Option<&'a PluginUse> {
    let owns = |u: &&PluginUse| effect.strip_prefix(u.id.as_str()).is_some_and(|rest| rest.starts_with('.'));
    p.plugins.iter().filter(owns).max_by_key(|u| u.id.len())
}

pub(crate) fn record(p: &Project) -> Vec<PluginUse> {
    let mut uses = BTreeMap::new();
    for effect in effect_ids(p) {
        let used = match dynamic::get(effect) {
            Some(r) => PluginUse { id: r.effect.owner.clone(), version: r.effect.version.clone(), digest: None },
            None => match recorded_owner(p, effect) {
                Some(kept) => kept.clone(),
                None => continue,
            },
        };
        uses.entry(used.id.clone()).or_insert(used);
    }
    uses.into_values().collect()
}

pub(crate) fn on_open(p: &Project) -> Vec<Value> {
    let mut found = Vec::new();
    for used in &p.plugins {
        let installed = dynamic::plugin_version(&used.id);
        if installed.as_deref() == Some(used.version.as_str()) {
            continue;
        }
        let status = if installed.is_some() { "differentVersion" } else { "missing" };
        found.push(json!({"id": used.id, "version": used.version, "installed": installed, "status": status}));
    }
    found
}

pub(crate) fn open_notice(found: &[Value]) -> Option<String> {
    let line = |f: &Value| match f["installed"].as_str() {
        Some(installed) => {
            format!("{} (saved with {}, installed {installed})", f["id"].as_str().unwrap_or_default(), f["version"].as_str().unwrap_or_default())
        }
        None => format!("{} {} (not installed)", f["id"].as_str().unwrap_or_default(), f["version"].as_str().unwrap_or_default()),
    };
    (!found.is_empty())
        .then(|| format!("This project uses plug-ins that are missing or a different version: {}.", found.iter().map(line).collect::<Vec<_>>().join(", ")))
}

pub fn missing_owner(p: &Project, effect: &str) -> Option<String> {
    if effectcraft_effects::find(effect).is_some() {
        return None;
    }
    recorded_owner(p, effect).map(|u| u.id.clone())
}

fn missing_effects(p: &Project) -> BTreeMap<&str, Option<String>> {
    let groups = p.comps().flat_map(|(_, c)| c.layers.iter()).filter_map(|l| l.effects()).flat_map(|fx| &fx.children);
    groups
        .filter_map(|n| match n {
            Node::Group(g) if g.enabled => match &g.kind {
                GroupKind::Effect { effect } if effectcraft_effects::find(effect).is_none() => Some((effect.as_str(), missing_owner(p, effect))),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(crate) fn render_refusal(p: &Project) -> Result<(), String> {
    let missing = missing_effects(p);
    if missing.is_empty() {
        return Ok(());
    }
    let named = missing
        .into_iter()
        .map(|(effect, owner)| match owner {
            Some(plugin) => format!("{effect} (plug-in {plugin})"),
            None => effect.to_owned(),
        })
        .collect::<Vec<_>>();
    Err(format!(
        "Not rendered: this project uses effects whose plug-ins are not installed: {}. Install the plug-ins, or render without these effects (renderQueue.render {{\"withoutMissingPlugins\": true}}, or --without-missing-plugins on the command line).",
        named.join(", ")
    ))
}

#[cfg(test)]
mod tests;
