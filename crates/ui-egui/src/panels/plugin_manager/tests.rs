use std::sync::OnceLock;

use egui_kittest::Harness;

use super::{Listed, State};
use crate::EffectcraftApp;

fn quarantined(id: &str) -> Listed {
    Listed {
        id: id.into(),
        name: "Crashy".into(),
        version: "1.0.0".into(),
        author: String::new(),
        description: String::new(),
        sandboxed: false,
        can_unsandbox: false,
        permissions: Vec::new(),
        state: State::Quarantined,
        error: Some("it was turned off because the app closed unexpectedly while it ran unsandboxed".into()),
        session_only: false,
    }
}

#[test]
fn a_quarantined_plugin_says_why_and_turn_back_on_asks_to_approve_it() {
    static CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("temp dir"));
    let mut session = effectcraft_host::session();
    effectcraft_plugin::install(&mut session, Some(config.path()));
    let mut app = EffectcraftApp::new(session);
    let id = "test.manager-quarantined";
    app.plugin_ui.manager_open = true;
    app.plugin_ui.listed = Some((std::time::Instant::now(), Ok(vec![quarantined(id)])));
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(|_| app);
    harness.step();

    assert_eq!(super::state_text(&quarantined(id)), "Turned off: it was turned off because the app closed unexpectedly while it ran unsandboxed");
    let button = harness.state().auto.find(&format!("plugins.{id}.approve")).expect("Turn Back On is shown");
    assert_eq!(button.label, "Turn Back On");
    let at = egui::pos2(button.rect[0] + button.rect[2] / 2.0, button.rect[1] + button.rect[3] / 2.0);
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    harness.input_mut().events.extend([egui::Event::PointerMoved(at), press(true), press(false)]);
    harness.step();
    harness.step();
    assert!(harness.state().ui.status.contains(&format!("plugin `{id}` is not installed")), "{}", harness.state().ui.status);
}
