use std::collections::{BTreeMap, BTreeSet};

use craft_plugin::{PluginRef, Saved};
use effectcraft_effects::dynamic;
use effectcraft_project::{GroupKind, ItemId, Node, PluginUse, Project};
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

/// The plugin effect that effect id `effect` (`"{plugin}.{item}"`) names: the registered one, else
/// the reference the project saved for it, else the id split at the longest plugin id that is
/// registered or recorded in the project. `None` for a built-in effect or an id no plugin claims.
pub fn effect_ref(p: &Project, effect: &str) -> Option<PluginRef> {
    if let Some(registered) = dynamic::get(effect) {
        let owner = &registered.effect.owner;
        let item = effect.strip_prefix(owner.as_str())?.strip_prefix('.')?;
        return Some(PluginRef::new(owner, item, &registered.effect.version));
    }
    if effectcraft_effects::find(effect).is_some() {
        return None;
    }
    let saved = p.plugin_refs.iter().find_map(|text| match PluginRef::parse_saved(text, []) {
        Saved::Ref(found) if names(&found, effect) => Some(found),
        _ => None,
    });
    saved.or_else(|| {
        let mut installed = dynamic::owners();
        installed.extend(p.plugins.iter().map(|used| used.id.clone()));
        match PluginRef::parse_saved(effect, installed.iter().map(String::as_str)) {
            Saved::Ref(found) => {
                let version = p.plugins.iter().find(|used| used.id == found.plugin).map(|used| used.version.as_str()).unwrap_or_default();
                Some(PluginRef::new(&found.plugin, &found.item, version))
            }
            Saved::Raw(_) => None,
        }
    })
}

fn names(found: &PluginRef, effect: &str) -> bool {
    effect.strip_prefix(found.plugin.as_str()).and_then(|rest| rest.strip_prefix('.')) == Some(found.item.as_str())
}

/// What a save writes as [`Project::plugin_refs`]: one reference per plugin effect in use, plus
/// every saved string this version could not read, unchanged.
pub(crate) fn record_refs(p: &Project) -> Vec<String> {
    let mut refs: BTreeSet<String> = effect_ids(p).into_iter().filter_map(|effect| effect_ref(p, effect)).map(|found| found.to_saved()).collect();
    refs.extend(p.plugin_refs.iter().filter(|text| matches!(PluginRef::parse_saved(text, []), Saved::Raw(_))).cloned());
    refs.into_iter().collect()
}

pub fn missing_owner(p: &Project, effect: &str) -> Option<String> {
    if effectcraft_effects::find(effect).is_some() {
        return None;
    }
    effect_ref(p, effect).map(|found| found.plugin)
}

fn missing_effects(p: &Project) -> BTreeMap<&str, Option<String>> {
    let heard = p.heard_layers();
    let reached: Option<BTreeSet<ItemId>> = p.render_queue.iter().filter(|item| item.is_queued()).try_fold(BTreeSet::new(), |mut reached, item| {
        reached.extend(p.pixel_reach(item.comp, &heard).items?);
        Some(reached)
    });
    let rendered = |id: &ItemId| reached.as_ref().is_none_or(|reached| reached.contains(id));
    let groups = p
        .comps()
        .filter(|(id, _)| rendered(id))
        .flat_map(|(_, c)| c.layers.iter())
        .filter(|l| l.switches.effects)
        .filter_map(|l| l.effects())
        .flat_map(|fx| &fx.children);
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
