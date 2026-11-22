use craft_plugin::{HostCall, Permission, Root};

use super::serve::Identity;
use crate::host::{Access, Host, Origin};

fn access(call: &HostCall, host: &dyn Host, plugin: &Identity, origin: Origin) -> Access {
    match call {
        HostCall::Start { .. } | HostCall::Task { .. } | HostCall::Cancel { .. } | HostCall::Progress { .. } => Access::Free,
        HostCall::Frame { .. } if origin == Origin::Render => {
            Access::Refused("an effect cannot render a frame while it renders; read another layer through a layer parameter (cx.layer) instead".into())
        }
        HostCall::Audio { .. } if origin == Origin::Render => {
            Access::Refused("an effect cannot read audio while it renders; read it in a task and pass what the effect needs as a parameter".into())
        }
        HostCall::Frame { .. } | HostCall::Audio { .. } => Access::Needs(vec![Permission::DocumentRead]),
        HostCall::PickFile { .. } if origin != Origin::Interactive => Access::Refused("a file can be picked only from a panel or a command".into()),
        HostCall::PickFile { .. } => Access::Free,
        HostCall::ReadFile { root: Root::User, path, .. } | HostCall::ListFiles { root: Root::User, path } if plugin.picked(path, false) => Access::Free,
        HostCall::WriteFile { root: Root::User, path } if plugin.picked(path, true) => Access::Free,
        HostCall::Log { .. } | HostCall::Notify { .. } | HostCall::Layer { .. } | HostCall::Source { .. } | HostCall::Masks | HostCall::Scene => Access::Free,
        HostCall::CacheGet { .. } | HostCall::CachePut { .. } | HostCall::CacheRemove { .. } | HostCall::CacheHas { .. } | HostCall::UploadImage { .. } => {
            Access::Free
        }
        HostCall::ParamsAt { .. } | HostCall::RemoveImage { .. } => Access::Free,
        HostCall::ClipboardRead | HostCall::ClipboardWrite { .. } => Access::Needs(vec![Permission::Clipboard]),
        HostCall::Command { id, params } => host.access(id, params),
        HostCall::ReadFile { root: Root::User, .. } | HostCall::ListFiles { root: Root::User, .. } => Access::Needs(vec![Permission::FilesRead]),
        HostCall::WriteFile { root: Root::User, .. } => Access::Needs(vec![Permission::FilesWrite]),
        HostCall::WriteFile { root: Root::Bundle, .. } => Access::Refused("the plugin folder is read-only".into()),
        HostCall::ReadFile { .. } | HostCall::ListFiles { .. } | HostCall::WriteFile { .. } => Access::Free,
    }
}

pub(crate) fn check(call: &HostCall, host: &dyn Host, plugin: &Identity, origin: Origin) -> Result<(), String> {
    let grants = &plugin.grants;
    match access(call, host, plugin, origin) {
        Access::Free => Ok(()),
        Access::Refused(why) => Err(format!("refused: {why}")),
        Access::Needs(needed) => match needed.iter().find(|p| !grants.contains(p)) {
            None => Ok(()),
            Some(missing) => Err(format!("permission denied: this plugin does not have `{missing}`; add \"{missing}\" to `permissions` in plugin.json")),
        },
    }
}
