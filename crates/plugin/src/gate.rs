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
    "edit",
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

fn needed(id: &str, params: &Value, needs: &mut BTreeSet<Permission>) -> Result<(), String> {
    if id == "engine.batch" {
        return steps(params)?.into_iter().try_for_each(|(inner, inner_params)| needed(inner, inner_params, needs));
    }
    let spec = commands::find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    let family = id.split('.').next().unwrap_or_default();
    if confine::names_files(id) {
        needs.extend(FILE_PERMISSIONS);
    } else if !DOCUMENT_FAMILIES.contains(&family) {
        return Err(format!("`{id}` is not available to plugins"));
    }
    needs.insert(if spec.journal { Permission::DocumentWrite } else { Permission::DocumentRead });
    Ok(())
}

impl<'a> SessionHost<'a> {
    pub fn new(session: &'a mut Session, plugin: &'a Loaded) -> Self {
        SessionHost { session, plugin }
    }

    fn confinement(&self) -> Result<Confinement, String> {
        let granted = &self.plugin.bundle.manifest.permissions;
        let (mut read, mut write) = (vec![self.plugin.bundle.dir.clone()], vec![self.plugin.data_dir.clone()]);
        if FILE_PERMISSIONS.iter().all(|p| granted.contains(p)) {
            let everywhere = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
            read.push(everywhere.clone());
            write.push(everywhere);
        }
        std::fs::create_dir_all(&self.plugin.data_dir).map_err(|e| e.to_string())?;
        Confinement::new(&read, &write).map_err(|e| e.to_string())
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
        self.files_allowed(id, &params)?;
        self.session.execute_checked(id, params).map_err(|e| e.to_string())
    }
}
