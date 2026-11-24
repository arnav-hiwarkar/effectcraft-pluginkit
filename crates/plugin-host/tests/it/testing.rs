use craft_plugin_host::testing::{Pattern, Picture, TestPlugin, check_frame, updating_frames};

use super::{Kind, bundle};

fn probe() -> (tempfile::TempDir, TestPlugin) {
    let (dir, bundle) = bundle(Kind::Wasm, &["document.write"]);
    let plugin = TestPlugin::load(&bundle).unwrap();
    (dir, plugin)
}

#[test]
fn a_stored_frame_passes_and_a_changed_render_writes_the_actual_and_a_difference_image_under_target() {
    let (_bundle, plugin) = probe();
    let work = tempfile::tempdir().unwrap();
    let full = plugin.render("invert").on(Pattern::Checker).size(64, 32);
    full.run().unwrap().save(&work.path().join("tests/frames/invert.png")).unwrap();
    assert_eq!(check_frame(work.path(), "invert", &full), Ok(()));

    let half = plugin.render("invert").on(Pattern::Checker).size(64, 32).set("amount", 0.5);
    let error = check_frame(work.path(), "invert", &half).unwrap_err();
    assert!(error.contains("2048 of 2048 pixels are off by more than 1/255 (largest 128/255, first at (0, 0))"), "{error}");
    assert!(error.contains("UPDATE_FRAMES=1"), "{error}");
    let actual = Picture::open(&work.path().join("target/frames/invert.actual.png")).unwrap();
    assert_eq!(actual.at(0, 0), [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
    let difference = Picture::open(&work.path().join("target/frames/invert.diff.png")).unwrap();
    assert_eq!(difference.at(5, 5), [1.0, 0.0, 0.0, 1.0]);

    assert_eq!(check_frame(work.path(), "invert", &full), Ok(()));
    assert!(!work.path().join("target/frames/invert.diff.png").exists());
    assert_eq!(std::fs::read_dir(work.path().join("tests/frames")).unwrap().count(), 1);
}

#[test]
fn a_pointwise_effect_that_changes_its_rows_in_place_renders_the_same_in_bands_as_whole() {
    let (_bundle, plugin) = probe();
    let picture = plugin.render("halve").on(Pattern::Checker).size(40, 257).run().unwrap();
    let (half, dark) = ([0.5, 0.5, 0.5, 1.0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!([(0, 200), (39, 200), (0, 230), (39, 230)].map(|(x, y)| picture.at(x, y)), [half, dark, dark, half]);
}

#[test]
fn only_update_frames_set_to_1_updates_stored_frames() {
    let asked = [None, Some("0"), Some("false"), Some("yes"), Some(""), Some("1")];
    assert_eq!(asked.map(|value| updating_frames(value.map(std::ffi::OsStr::new))), [false, false, false, false, false, true]);
}

#[test]
fn a_frame_name_that_is_not_a_plain_stem_is_refused_and_nothing_is_written() {
    let (_bundle, plugin) = probe();
    let work = tempfile::tempdir().unwrap();
    let render = plugin.render("invert").on(Pattern::Checker).size(8, 8);
    for name in ["../preview", "full.actual", "a/b", ""] {
        let error = check_frame(work.path(), name, &render).unwrap_err();
        assert_eq!(error, format!("frame name `{name}` must be a plain name of letters, digits, `-` and `_`, such as `full`"));
    }
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_stored_frame_that_is_a_link_is_refused_and_its_target_is_not_written() {
    let (_bundle, plugin) = probe();
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(work.path().join("tests/frames")).unwrap();
    let link = work.path().join("tests/frames/full.png");
    std::os::unix::fs::symlink(outside.path().join("victim.png"), &link).unwrap();
    let error = check_frame(work.path(), "full", &plugin.render("invert").on(Pattern::Checker).size(8, 8)).unwrap_err();
    assert_eq!(error, format!("{} is a link; stored frames must be plain files", link.display()));
    assert!(!outside.path().join("victim.png").exists());
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

#[test]
fn a_test_render_clamps_to_the_hard_range_and_refuses_text_for_a_number_like_the_app() {
    let (_bundle, plugin) = probe();
    let render = |amount: f64| plugin.render("invert").on(Pattern::Checker).size(2, 1).set("amount", amount).run().unwrap();
    assert_eq!(render(2.0).pixels, [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]]);
    assert_eq!(render(2.0), render(1.0));
    let error = plugin.render("invert").set("amount", "x").run().unwrap_err();
    assert_eq!(error, r#"parameter `amount` takes a number, not the text "x""#);
}

#[test]
fn an_effect_that_changes_with_time_must_say_so() {
    let (_bundle, plugin) = probe();
    assert_eq!(plugin.render("drift").time_check(), Ok(()));
    assert_eq!(plugin.render("invert").time_check(), Ok(()));
    let error = plugin.render("silent-drift").time_check().unwrap_err();
    assert!(error.contains("effect `silent-drift` renders differently at 0 s and at 0.5 s") && error.contains("`.time_dependent()`"), "{error}");
}

#[test]
fn an_effect_that_reads_the_scene_at_another_time_without_declaring_other_frames_fails_and_says_what_to_declare() {
    let (_bundle, plugin) = probe();
    let error = plugin.render("scene-later").size(8, 8).run().unwrap_err();
    assert!(error.contains("effect `scene-later` must declare `.reads_other_frames()`"), "{error}");
}
