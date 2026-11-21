use std::path::PathBuf;

use craft_plugin_host::{Access, Host, Permission};
use effectcraft_automation::confine::{Confinement, Lookup};
use effectcraft_engine::{Session, commands};
use serde_json::{Value, json};

use crate::manage::Loaded;

pub(crate) struct SessionHost<'a> {
    session: &'a mut Session,
    confinement: Option<Confinement>,
}

impl<'a> SessionHost<'a> {
    pub fn new(session: &'a mut Session, plugin: &Loaded) -> Self {
        let granted = &plugin.bundle.manifest.permissions;
        let everywhere = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
        let mut read = vec![plugin.bundle.dir.clone()];
        let mut write = vec![plugin.data_dir.clone()];
        if granted.contains(&Permission::FilesRead) {
            read.push(everywhere.clone());
        }
        if granted.contains(&Permission::FilesWrite) {
            write.push(everywhere);
        }
        let confinement = std::fs::create_dir_all(&plugin.data_dir).ok().and_then(|()| Confinement::new(&read, &write).ok());
        SessionHost { session, confinement }
    }

    fn files_allowed(&mut self, id: &str, params: &Value) -> Result<(), String> {
        let confinement = self.confinement.as_ref().ok_or("the plugin's folders are unavailable")?;
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

fn edits(id: &str, params: &Value) -> bool {
    if id == "engine.batch" {
        let steps = params.get("steps").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        return steps.iter().any(|step| {
            let inner = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str).unwrap_or_default();
            edits(inner, step.get("params").unwrap_or(&Value::Null))
        });
    }
    commands::find(id).is_none_or(|spec| spec.journal)
}

impl Host for SessionHost<'_> {
    fn access(&mut self, id: &str, params: &Value) -> Access {
        if let Err(why) = self.files_allowed(id, params) {
            return Access::Refused(why);
        }
        Access::Needs(vec![if edits(id, params) { Permission::DocumentWrite } else { Permission::DocumentRead }])
    }

    fn command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.session.execute_checked(id, params).map_err(|e| e.to_string())
    }
}
