use std::collections::BTreeSet;
use std::path::PathBuf;

use craft_plugin_host::{Access, Host, Permission};
use effectcraft_automation::confine::{self, Confinement, Lookup};
use effectcraft_engine::{Event, Session, commands};
use serde_json::{Value, json};

use crate::manage::Loaded;

const DOCUMENT_FAMILIES: &[&str] = &[
    "anim",
    "camera",
    "command",
    "comp",
    "editor",
    "effect",
    "engine",
    "essential",
    "expr",
    "item",
    "keys",
    "layer",
    "light",
    "markers",
    "mask",
    "material",
    "motion",
    "path",
    "paths",
    "project",
    "prop",
    "puppet",
    "shape",
    "text",
    "time",
    "timeline",
];

const FILE_FAMILIES: &[&str] = &["file"];

const REFUSED: &[&str] = &[
    "anim.browsePresets",
    "anim.clearRecentPresets",
    "comp.copyFrame",
    "comp.deletePreset",
    "comp.flowchart",
    "comp.miniFlowchart",
    "comp.openInEssentialGraphics",
    "comp.savePreset",
    "effect.manage",
    "layer.openLayer",
    "layer.revealInFinder",
    "layer.style.options",
    "project.deleteWithoutConfirmation",
    "prop.editGradient",
];

const WRITES_A_PROPERTY: &[&str] = &["prop.set", "prop.setExpression", "prop.pickWhip", "prop.addKey", "keys.set"];

const MAX_NOTICE: usize = 300;

const FILE_PERMISSIONS: [Permission; 2] = [Permission::FilesRead, Permission::FilesWrite];

pub(crate) struct SessionHost<'a> {
    session: &'a mut Session,
    plugin: &'a Loaded,
}

fn steps(batch: &Value) -> Result<Vec<(&str, &Value)>, String> {
    let steps = batch.get("steps").and_then(Value::as_array).ok_or("`engine.batch` needs a `steps` array")?;
    steps
        .iter()
        .map(|step| {
            let id = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str).ok_or("a batch step names no command")?;
            Ok((id, step.get("params").unwrap_or(&Value::Null)))
        })
        .collect()
}

fn family(id: &str) -> &str {
    id.split('.').next().unwrap_or_default()
}

fn needed(id: &str, params: &Value, needs: &mut BTreeSet<Permission>) -> Result<(), String> {
    if id == "engine.batch" {
        return steps(params)?.into_iter().try_for_each(|(inner, inner_params)| needed(inner, inner_params, needs));
    }
    let spec = commands::find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    let document = DOCUMENT_FAMILIES.contains(&family(id));
    let names_files = confine::names_files(id) && (document || FILE_FAMILIES.contains(&family(id)));
    if REFUSED.contains(&id) || !(document || names_files) {
        return Err(format!("`{id}` is not available to plugins"));
    }
    if params.get("wait").and_then(Value::as_bool) == Some(true) {
        return Err(format!("`{id}` cannot wait for a background job when a plugin runs it"));
    }
    if names_files {
        needs.extend(FILE_PERMISSIONS);
    }
    needs.insert(if spec.journal { Permission::DocumentWrite } else { Permission::DocumentRead });
    Ok(())
}

fn refers_to_a_step(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|text| text.starts_with('$'))
}

impl<'a> SessionHost<'a> {
    pub fn new(session: &'a mut Session, plugin: &'a Loaded) -> Self {
        SessionHost { session, plugin }
    }

    fn confinement(&self) -> Result<Confinement, String> {
        let (mut read, mut write) = (vec![self.plugin.bundle.dir.clone()], vec![self.plugin.data_dir.clone()]);
        if self.holds_file_permissions() {
            let everywhere = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
            read.push(everywhere.clone());
            write.push(everywhere);
        }
        std::fs::create_dir_all(&self.plugin.data_dir).map_err(|e| e.to_string())?;
        Confinement::new(&read, &write).map_err(|e| e.to_string())
    }

    fn holds_file_permissions(&self) -> bool {
        FILE_PERMISSIONS.iter().all(|p| self.plugin.bundle.manifest.permissions.contains(p))
    }

    fn property_write_allowed(&mut self, id: &str, params: &Value, in_batch: bool) -> Result<(), String> {
        if !WRITES_A_PROPERTY.contains(&id) {
            return Ok(());
        }
        let needs_files = |what: &str| format!("`{id}`: {what} needs the `files.read` and `files.write` permissions");
        let path = params.get("path").and_then(Value::as_str);
        let under_effects = path.is_none_or(|path| path.starts_with("effects"));
        if params.get("selected").and_then(Value::as_bool) == Some(true) && under_effects {
            return Err(needs_files("writing an effect property on every selected layer"));
        }
        let created_by_the_batch = in_batch && ["layer", "prop", "path"].iter().any(|key| refers_to_a_step(params.get(*key)));
        if created_by_the_batch {
            return if under_effects { Err(needs_files("writing an effect property on a layer the batch creates")) } else { Ok(()) };
        }
        let mut target = serde_json::Map::new();
        for key in ["layer", "comp", "path", "prop"] {
            if let Some(value) = params.get(key) {
                target.insert(key.to_string(), value.clone());
            }
        }
        if !target.contains_key("path") && !target.contains_key("prop") {
            return Err(needs_files("writing the selected properties"));
        }
        let found = self.session.execute("prop.get", Value::Object(target)).map_err(|e| e.to_string())?;
        if found.get("type").and_then(Value::as_str) == Some("string") {
            return Err(needs_files("writing a text-valued property, which an effect may read as a file path,"));
        }
        Ok(())
    }

    fn property_writes_allowed(&mut self, id: &str, params: &Value) -> Result<(), String> {
        if self.holds_file_permissions() {
            return Ok(());
        }
        if id != "engine.batch" {
            return self.property_write_allowed(id, params, false);
        }
        let owned: Vec<(String, Value)> = steps(params)?.into_iter().map(|(inner, inner_params)| (inner.to_string(), inner_params.clone())).collect();
        owned.iter().try_for_each(|(inner, inner_params)| self.property_write_allowed(inner, inner_params, true))
    }

    fn files_allowed(&mut self, id: &str, params: &Value) -> Result<(), String> {
        let confinement = self.confinement()?;
        let not_for_plugins = |e: effectcraft_automation::Error| format!("`{id}` is not available to this plugin: {e}");
        for lookup in confinement.check_command(id, params).map_err(not_for_plugins)? {
            let query = match lookup {
                Lookup::ProjectFile | Lookup::QueueDefault => "project.summary",
                Lookup::RenderOutputs => "renderQueue.list",
            };
            let found = self.session.execute(query, json!({})).map_err(|e| e.to_string())?;
            confinement.check_lookup(&lookup, &found).map_err(not_for_plugins)?;
        }
        Ok(())
    }
}

impl Host for SessionHost<'_> {
    fn access(&self, id: &str, params: &Value) -> Access {
        let mut needs = BTreeSet::new();
        match needed(id, params, &mut needs) {
            Ok(()) => Access::Needs(needs.into_iter().collect()),
            Err(why) => Access::Refused(why),
        }
    }

    fn notify(&mut self, message: &str) {
        let message: String = message.chars().take(MAX_NOTICE).collect();
        let message = format!("{}: {message}", self.plugin.bundle.manifest.name);
        self.session.events.push(Event::Toast { message, error: false });
    }

    fn command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.property_writes_allowed(id, &params)?;
        self.files_allowed(id, &params)?;
        self.session.execute_checked(id, params).map_err(|e| e.to_string())
    }
}
