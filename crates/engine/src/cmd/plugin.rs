use serde_json::Value;

use super::{CommandSpec, always, frontend};
use crate::{EngineError, Result, Session, cmd, query};

const USER_ONLY: [&str; 7] = ["plugin.install", "plugin.approve", "plugin.setSandbox", "plugin.disable", "plugin.uninstall", "plugin.load", "plugin.reload"];

fn forward(s: &mut Session, id: &str, params: &Value) -> Result<Value> {
    if s.running_for_script() && USER_ONLY.contains(&id) {
        return Err(EngineError::Other(format!(
            "Scripts can't run `{id}`: installing, approving, loading and removing plug-ins is the user's act. Do it yourself in Window ▸ Plug-ins ▸ Manage Plug-ins."
        )));
    }
    let run = s.plugin_commands.ok_or_else(|| EngineError::Other("plug-ins are not available in this session".into()))?;
    run(s, id, params)
}

macro_rules! plugin {
    ($id:literal, $label:literal, $params:literal) => {
        query!($id, $label, $params, |s, p| forward(s, $id, p))
    };
}

macro_rules! window {
    ($id:literal, $label:literal, $params:literal) => {
        cmd!($id, $label, [], None, $params, always, |s, p| frontend(s, $id, p))
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        plugin!("plugin.list", "List Plug-ins", "{} — every plug-in with its state, permissions, effects, panels and commands"),
        plugin!("plugin.install", "Install Plug-in...", "{path: .craftplugin file or plug-in folder} — installs it; it runs after plugin.approve"),
        plugin!("plugin.approve", "Allow and Enable Plug-in", "{id} — grants the permissions the plug-in asks for and starts it"),
        plugin!(
            "plugin.setSandbox",
            "Set Plug-in Sandbox",
            "{id, sandboxed: bool} — a plug-in with a WebAssembly and a native build runs sandboxed until sandboxed is false; the choice is for this version"
        ),
        plugin!("plugin.disable", "Disable Plug-in", "{id}"),
        plugin!("plugin.uninstall", "Remove Plug-in", "{id}"),
        plugin!("plugin.load", "Load Plug-in for This Session", "{path: plug-in folder} — development: runs it with the permissions it asks for until quit"),
        plugin!("plugin.reload", "Reload Plug-ins", "{}"),
        plugin!("plugin.run", "Run Plug-in Command", "{plugin, command}"),
        plugin!("plugin.startTask", "Start Plug-in Task", "{plugin, task, args?} — runs the task in the background; returns its id"),
        plugin!(
            "plugin.tasks",
            "List Plug-in Tasks",
            "{wait?: bool, timeout?: seconds (600)} — every plug-in task with status, progress, message, result and error; `wait` answers the tasks' app calls until none runs"
        ),
        plugin!("plugin.cancelTask", "Cancel Plug-in Task", "{plugin, id}"),
        plugin!(
            "plugin.log",
            "Read Plug-in Log",
            "{id, after?: number, load?: number} — {load, missed, lines}: the plug-in's log lines, panics and failed calls with a sequence number each; pass the last `seq` and `load` seen to read only newer lines"
        ),
        window!("window.plugins", "Manage Plug-ins...", "{}"),
        window!("window.pluginPanel", "Show Plug-in Panel", "{plugin, panel}"),
    ]
}
