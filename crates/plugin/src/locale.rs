use std::sync::Mutex;

use effectcraft_engine::Session;
use serde_json::json;

static APPLIED: Mutex<String> = Mutex::new(String::new());

const DEFAULT: &str = "en";

/// Shows plugin labels written as `@key` in the app's interface language, such as `de` or
/// `zh-hans`. Running plugins restart when the language changes, so their labels follow it.
pub fn set_locale(session: &mut Session, language: &str) {
    let locale = tag(language);
    {
        let mut applied = APPLIED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = if applied.is_empty() { DEFAULT } else { applied.as_str() };
        if current == locale {
            return;
        }
        *applied = locale.clone();
    }
    craft_plugin_host::set_locale(&locale);
    if crate::plugins().is_some()
        && let Err(error) = session.execute("plugin.reload", json!({}))
    {
        log::warn!("plug-ins were not restarted for the language {locale}: {error}");
    }
}

fn tag(language: &str) -> String {
    let mut parts = language.split(['-', '_']);
    let mut tag = parts.next().unwrap_or_default().to_ascii_lowercase();
    for part in parts {
        tag.push('-');
        match part.len() {
            2 => tag.push_str(&part.to_ascii_uppercase()),
            4 => tag.extend(part.chars().enumerate().map(|(at, c)| if at == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })),
            _ => tag.push_str(part),
        }
    }
    tag
}

#[cfg(test)]
mod tests {
    #[test]
    fn interface_languages_become_locale_tags() {
        assert_eq!(super::tag("de"), "de");
        assert_eq!(super::tag("pt-br"), "pt-BR");
        assert_eq!(super::tag("zh-hans"), "zh-Hans");
        assert_eq!(super::tag("zh_HANT"), "zh-Hant");
    }
}
