use crate::Session;

/// When the app asks plugins before writing a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginHook {
    BeforeSave,
    BeforeExport,
}

/// A plugin's reason not to go ahead with a save or an export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginVeto {
    pub plugin: String,
    pub reason: String,
}

impl std::fmt::Display for PluginVeto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.plugin, self.reason)
    }
}

/// Asks every running plugin that declared `hook` about writing `path`, and returns their vetoes.
pub type PluginHooks = fn(PluginHook, &str) -> Vec<PluginVeto>;

impl Session {
    /// Asks the plugins before a save to `path`. A save always goes ahead; each veto shows as a notice.
    pub(crate) fn plugins_before_save(&mut self, path: &str) {
        for veto in self.plugin_vetoes(PluginHook::BeforeSave, [path]) {
            self.toast(veto.to_string());
        }
    }

    /// Asks the plugins before an export writes `paths`. The first veto cancels the export.
    pub(crate) fn plugins_before_export<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
        match self.plugin_vetoes(PluginHook::BeforeExport, paths).into_iter().next() {
            Some(veto) => Err(format!("Not rendered: {veto}")),
            None => Ok(()),
        }
    }

    fn plugin_vetoes<'a>(&self, hook: PluginHook, paths: impl IntoIterator<Item = &'a str>) -> Vec<PluginVeto> {
        let Some(ask) = self.plugin_hooks else { return Vec::new() };
        let mut asked = std::collections::BTreeSet::new();
        paths.into_iter().filter(|path| asked.insert(*path)).flat_map(|path| ask(hook, path)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refuse_gif(hook: PluginHook, path: &str) -> Vec<PluginVeto> {
        let veto = |reason: &str| vec![PluginVeto { plugin: "Alpha Guard".into(), reason: reason.into() }];
        match hook {
            PluginHook::BeforeSave => veto("the project keeps a temporary matte"),
            PluginHook::BeforeExport if path.ends_with(".gif") => veto("GIF drops the alpha this look needs"),
            PluginHook::BeforeExport => Vec::new(),
        }
    }

    #[test]
    fn a_save_veto_is_shown_and_the_save_goes_ahead() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ecproj").to_string_lossy().into_owned();
        let mut s = Session { plugin_hooks: Some(refuse_gif), ..Session::default() };
        s.execute_checked("file.saveAs", json!({"path": path})).unwrap();
        assert!(std::path::Path::new(&path).is_file());
        let toasts: Vec<String> = s
            .drain_events()
            .into_iter()
            .filter_map(|event| match event {
                crate::Event::Toast { message, .. } => Some(message),
                _ => None,
            })
            .collect();
        assert!(toasts.iter().any(|toast| toast == "Alpha Guard: the project keeps a temporary matte"), "{toasts:?}");
    }

    #[test]
    fn an_export_veto_names_the_plugin_and_its_reason() {
        let s = Session { plugin_hooks: Some(refuse_gif), ..Session::default() };
        assert_eq!(s.plugins_before_export(["/out/a.mov", "/out/b.gif"]), Err("Not rendered: Alpha Guard: GIF drops the alpha this look needs".into()));
        assert_eq!(s.plugins_before_export(["/out/a.mov"]), Ok(()));
        assert_eq!(Session::default().plugins_before_export(["/out/b.gif"]), Ok(()));
    }
}
