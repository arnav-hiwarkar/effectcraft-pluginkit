use craft_plugin_host::{HookKind, HookReply, Host};
use effectcraft_engine::plugin_hooks::{PluginHook, PluginVeto};

use crate::manage::running_plugins;

struct Asked;

impl Host for Asked {}

pub(crate) fn ask(hook: PluginHook, path: &str) -> Vec<PluginVeto> {
    let kind = match hook {
        PluginHook::BeforeSave => HookKind::BeforeSave,
        PluginHook::BeforeExport => HookKind::BeforeExport,
    };
    running_plugins()
        .iter()
        .filter_map(|loaded| match loaded.plugin.hook(&mut Asked, kind, path) {
            Ok(HookReply::Continue) => None,
            Ok(HookReply::Veto { reason }) => Some(PluginVeto { plugin: loaded.bundle.manifest.name.clone(), reason }),
            Err(error) => {
                log::warn!("plugin `{}` failed its {kind:?} hook for {path}: {error}", loaded.bundle.manifest.id);
                None
            }
        })
        .collect()
}
