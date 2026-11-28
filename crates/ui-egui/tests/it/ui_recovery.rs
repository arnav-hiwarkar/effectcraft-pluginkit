//! Crash recovery: Open Auto-Save in the Recover Project dialog brings back every comp tab and the
//! active comp, and File ▸ Save then writes the original project, not the auto-save.

use effectcraft_engine::Session;
use effectcraft_engine::autosave::Recovery;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, PointerButton, pos2};
use egui_kittest::Harness;
use serde_json::json;

fn crashed_run(dir: &std::path::Path) -> (String, String, Vec<u64>) {
    let mut s = Session::default();
    s.prefs.auto_save.location = "custom".into();
    s.prefs.auto_save.folder = dir.join("saves").to_string_lossy().into_owned();
    let mut ids = Vec::new();
    for name in ["A", "B", "C"] {
        s.execute("comp.new", json!({"name": name, "width": 320, "height": 180, "duration": 10})).unwrap();
        ids.push(s.state.active_comp.unwrap().0);
    }
    s.open_comp(effectcraft_engine::project::ItemId(ids[1]));
    let original = dir.join("work.ecproj").to_string_lossy().into_owned();
    s.execute("file.saveAs", json!({"path": original})).unwrap();
    s.execute("layer.newNull", json!({"name": "unsaved work"})).unwrap();
    let autosave = s.autosave_now().unwrap();
    (original, autosave, ids)
}

#[test]
fn open_auto_save_restores_the_tabs_and_saves_over_the_original() {
    let dir = std::env::temp_dir().join(format!("ec-ui-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (original, autosave, ids) = crashed_run(&dir);

    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_context_setup(effectcraft_ui_egui::framework::install_headless)
        .build_eframe(|_| EffectcraftApp::new(Session::default()));
    h.state_mut().offer_recovery(Recovery { project: Some(original.clone()), autosave: Some(autosave.clone()), dirty: true });
    h.run_steps(3);
    let r = h.state().auto.find("recovery.openAutoSave").expect("the Open Auto-Save button").rect;
    let p = pos2(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);

    let s = &h.state().session;
    let open: Vec<u64> = s.state.open_comps.iter().map(|c| c.0).collect();
    assert_eq!(open, ids);
    assert_eq!(s.state.active_comp.map(|c| c.0), Some(ids[1]));
    assert_eq!(s.path.as_deref(), Some(original.as_str()));
    assert!(s.is_dirty());
    assert!(s.prefs.recent_projects.iter().all(|p| p != &autosave));

    h.state_mut().session.execute("file.save", json!({})).unwrap();
    assert!(std::fs::read_to_string(&original).unwrap().contains("unsaved work"));
    let _ = std::fs::remove_dir_all(dir);
}
