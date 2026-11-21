use std::collections::BTreeSet;

use craft_plugin::{HostCall, Permission, Root};

use crate::host::{Access, Host};

fn access(call: &HostCall, host: &mut dyn Host) -> Access {
    match call {
        HostCall::Log { .. } | HostCall::Layer { .. } | HostCall::Source { .. } | HostCall::Masks | HostCall::Scene => Access::Free,
        HostCall::Command { id, params } => host.access(id, params),
        HostCall::ReadFile { root: Root::User, .. } | HostCall::ListFiles { root: Root::User, .. } => Access::Needs(vec![Permission::FilesRead]),
        HostCall::WriteFile { root: Root::User, .. } => Access::Needs(vec![Permission::FilesWrite]),
        HostCall::WriteFile { root: Root::Bundle, .. } => Access::Refused("the plugin folder is read-only".into()),
        HostCall::ReadFile { .. } | HostCall::ListFiles { .. } | HostCall::WriteFile { .. } => Access::Free,
    }
}

pub(crate) fn check(call: &HostCall, host: &mut dyn Host, grants: &BTreeSet<Permission>) -> Result<(), String> {
    match access(call, host) {
        Access::Free => Ok(()),
        Access::Refused(why) => Err(format!("refused: {why}")),
        Access::Needs(needed) => match needed.iter().find(|p| !grants.contains(p)) {
            None => Ok(()),
            Some(missing) => Err(format!("permission denied: this plugin does not have `{missing}`")),
        },
    }
}
