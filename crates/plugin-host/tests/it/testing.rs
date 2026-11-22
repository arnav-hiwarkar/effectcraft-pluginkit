use craft_plugin_host::testing::{Pattern, Picture, TestPlugin, check_frame};

use super::{Kind, bundle};

fn probe() -> (tempfile::TempDir, TestPlugin) {
    let (dir, bundle) = bundle(Kind::Wasm, &["document.write"]);
    let plugin = TestPlugin::load(&bundle).unwrap();
    (dir, plugin)
}

#[test]
fn a_stored_frame_passes_and_a_changed_render_writes_the_actual_and_a_difference_image_beside_it() {
    let (_bundle, plugin) = probe();
    let work = tempfile::tempdir().unwrap();
    let full = plugin.render("invert").on(Pattern::Checker).size(64, 32);
    full.run().unwrap().save(&work.path().join("tests/frames/invert.png")).unwrap();
    assert_eq!(check_frame(work.path(), "invert", &full), Ok(()));

    let half = plugin.render("invert").on(Pattern::Checker).size(64, 32).set("amount", 0.5);
    let error = check_frame(work.path(), "invert", &half).unwrap_err();
    assert!(error.contains("2048 of 2048 pixels are off by more than 1/255 (largest 128/255, first at (0, 0))"), "{error}");
    assert!(error.contains("UPDATE_FRAMES=1"), "{error}");
    let actual = Picture::open(&work.path().join("tests/frames/invert.actual.png")).unwrap();
    assert_eq!(actual.at(0, 0), [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
    let difference = Picture::open(&work.path().join("tests/frames/invert.diff.png")).unwrap();
    assert_eq!(difference.at(5, 5), [1.0, 0.0, 0.0, 1.0]);

    assert_eq!(check_frame(work.path(), "invert", &full), Ok(()));
    assert!(!work.path().join("tests/frames/invert.diff.png").exists());
}

#[test]
fn a_misspelt_parameter_or_effect_names_what_the_plugin_has() {
    let (_bundle, plugin) = probe();
    let error = plugin.render("invert").set("amout", 1.0).run().unwrap_err();
    assert_eq!(error, r#"effect `invert` has no parameter `amout`; its parameters are ["amount"]"#);
    let error = plugin.render("invrt").run().unwrap_err();
    assert!(error.starts_with("this plugin has no effect `invrt`; its effects are [\"invert\""), "{error}");
}

#[test]
fn a_panel_is_clicked_by_its_label_and_a_command_is_recorded_with_its_parameters() {
    let (_bundle, plugin) = probe();
    let mut panel = plugin.panel("counter").unwrap();
    panel.click("Count").unwrap();
    panel.click("Count").unwrap();
    assert!(panel.texts().contains(&"2 clicks at speed 0".to_string()), "{:?}", panel.texts());
    let error = panel.click("Cont").unwrap_err();
    assert!(error.starts_with("panel `counter` shows no widget labelled `Cont`; it shows [\"Count\""), "{error}");
    plugin.command("edit").unwrap();
    assert_eq!(plugin.commands(), [("layer.newNull".to_string(), serde_json::json!({"name": "From Plugin"}))]);
}

#[test]
fn a_failed_call_is_journalled_with_the_call_and_the_effect() {
    let (_bundle, plugin) = probe();
    let error = plugin.render("read-inputs").run().unwrap_err();
    assert_eq!(error, "no matte layer");
    assert_eq!(plugin.logs(), ["render of effect `read-inputs` failed: no matte layer"]);
}
