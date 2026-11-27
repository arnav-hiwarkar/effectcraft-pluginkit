use std::sync::Mutex;

struct SystemClipboard(Mutex<Option<arboard::Clipboard>>);

impl SystemClipboard {
    fn with<T>(&self, run: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Result<T, String> {
        let mut held = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if held.is_none() {
            *held = Some(arboard::Clipboard::new().map_err(|e| format!("the system clipboard is not available: {e}"))?);
        }
        let clipboard = held.as_mut().ok_or("the system clipboard is not available")?;
        run(clipboard).map_err(|e| format!("the system clipboard refused: {e}"))
    }
}

impl effectcraft_plugin::Clipboard for SystemClipboard {
    fn text(&self) -> Result<String, String> {
        self.with(|clipboard| clipboard.get_text())
    }

    fn set_text(&self, text: &str) -> Result<(), String> {
        self.with(|clipboard| clipboard.set_text(text))
    }
}

pub fn install() {
    effectcraft_plugin::set_clipboard(Box::new(SystemClipboard(Mutex::new(None))));
}
