use serde_json::json;

use crate::app;

#[test]
fn a_plugin_with_read_permission_reads_but_cannot_edit() {
    let mut app = app("test.gate-reader", &["document.read"]);
    app.run("read").expect("reading is allowed");

    let denied = app.run("edit").unwrap_err();
    assert!(denied.contains("document.write"), "{denied}");
    assert!(app.layer_names().is_empty(), "the edit ran without permission: {:?}", app.layer_names());
}

#[test]
fn a_batch_is_judged_by_its_steps_so_it_cannot_smuggle_an_edit() {
    let mut app = app("test.gate-batch", &["document.read"]);
    let denied = app.run("batch-edit").unwrap_err();
    assert!(denied.contains("document.write"), "{denied}");
    assert!(app.layer_names().is_empty(), "the batch ran without permission: {:?}", app.layer_names());
}

#[test]
fn a_plugin_with_write_permission_edits_and_a_batch_is_one_undo_step() {
    let mut app = app("test.gate-editor", &["document.read", "document.write"]);
    app.run("batch-edit").expect("editing is allowed");
    assert_eq!(app.layer_names().len(), 2, "{:?}", app.layer_names());

    app.session.execute("edit.undo", serde_json::json!({})).unwrap();
    assert!(app.layer_names().is_empty(), "one undo removes the whole batch: {:?}", app.layer_names());
}

#[test]
fn running_scripts_is_refused_whatever_the_plugin_was_granted() {
    let mut app = app("test.gate-scripts", &["document.read", "document.write", "files.read", "files.write"]);
    let refused = app.run("run-script").unwrap_err();
    assert!(refused.contains("refused"), "{refused}");
}

#[test]
fn a_command_that_writes_a_file_needs_the_files_permission_for_that_path() {
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("copy.ecproj");

    let mut confined = app("test.gate-files-no", &["document.read", "document.write"]);
    std::fs::write(confined.bundle.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
    let denied = confined.run("save-copy").unwrap_err();
    assert!(denied.contains("files."), "{denied}");
    assert!(!target.exists(), "the project was written outside the plugin's folders");

    let mut trusted = app("test.gate-files-yes", &["document.read", "document.write", "files.read", "files.write"]);
    std::fs::write(trusted.bundle.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
    trusted.run("save-copy").expect("both file permissions allow the path");
    assert!(target.exists());
}

#[test]
fn a_plugin_cannot_approve_or_manage_plugins() {
    let mut app = app("test.gate-self", &["document.read", "document.write", "files.read", "files.write"]);
    let refused = app.run("approve-itself").unwrap_err();
    assert!(refused.contains("refused"), "{refused}");
}

#[test]
fn a_read_inside_an_edit_batch_still_needs_the_read_permission() {
    let mut app = app("test.gate-batch-read", &["document.write"]);
    let denied = app.run("read-inside-edit").unwrap_err();
    assert!(denied.contains("document.read"), "{denied}");
    assert!(app.layer_names().is_empty(), "{:?}", app.layer_names());
}

#[test]
fn commands_outside_the_document_families_are_refused() {
    let mut app = app("test.gate-families", &["document.read", "document.write", "files.read", "files.write"]);
    let refused = app.run("open-website").unwrap_err();
    assert!(refused.contains("not available to plugins"), "{refused}");
}

#[test]
fn a_file_command_needs_both_file_permissions() {
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("copy.ecproj");
    let mut half = app("test.gate-files-half", &["document.read", "document.write", "files.write"]);
    std::fs::write(half.bundle.path().join("user-target.txt"), target.to_str().unwrap()).unwrap();
    let denied = half.run("save-copy").unwrap_err();
    assert!(denied.contains("files.read"), "{denied}");
    assert!(!target.exists());
}

#[test]
fn a_plugin_message_reaches_the_user_with_the_plugin_name() {
    let mut app = app("test.gate-notify", &[]);
    app.run("greet").unwrap();
    let shown: Vec<String> = app
        .session
        .events
        .iter()
        .filter_map(|event| match event {
            effectcraft_engine::Event::Toast { message, error: false } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["Probe: 3 layers renamed"]);
}

const EDITOR: &[&str] = &["document.read", "document.write"];
const EVERYTHING: &[&str] = &["document.read", "document.write", "files.read", "files.write"];

#[test]
fn a_file_path_cannot_be_planted_in_an_effect_parameter_without_file_permissions() {
    let mut app = app("test.gate-lut", EDITOR);
    app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "ec.utility.applylut"})).unwrap();
    let lut = |app: &mut crate::App| app.session.execute_checked("prop.get", json!({"path": "effects/#1/lut"})).unwrap()["value"].clone();

    for (command, params) in [
        ("prop.set", json!({"path": "effects/#1/lut", "value": "Users/someone/look.cube"})),
        ("prop.setExpression", json!({"path": "effects/#1/lut", "expression": "'/Users' + '/someone/look.cube'"})),
        ("prop.setExpression", json!({"expression": "'/Users' + '/someone/look.cube'"})),
    ] {
        let denied = app.relay(command, params).unwrap_err();
        assert!(denied.contains("files.read"), "{command}: {denied}");
    }
    let batch = json!({"steps": [
        {"command": "layer.newSolid", "params": {"name": "Another"}},
        {"command": "prop.set", "params": {"layer": "$1.layer", "path": "effects/#1/lut", "value": "Users/someone/look.cube"}},
    ]});
    let denied = app.relay("engine.batch", batch).unwrap_err();
    assert!(denied.contains("files.read"), "batch: {denied}");
    assert_eq!(lut(&mut app), json!(""));

    app.relay("prop.set", json!({"path": "transform/opacity", "value": 40.0})).expect("a number property is writable");
}

#[test]
fn a_batch_can_still_rig_layers_it_creates() {
    let mut app = app("test.gate-rig", EDITOR);
    let batch = json!({"label": "Rig", "steps": [
        {"command": "layer.newNull", "params": {"name": "Control"}},
        {"command": "prop.setExpression", "params": {"layer": "$1.layer", "path": "transform/rotation", "expression": "time * 90"}},
    ]});
    app.relay("engine.batch", batch).expect("an expression on a new layer's transform is allowed");
    assert_eq!(app.layer_names(), ["Control"]);
}

#[test]
fn render_jobs_settings_and_waiting_are_refused_even_with_every_permission() {
    let mut app = app("test.gate-jobs", EVERYTHING);
    for (command, params) in [
        ("renderQueue.render", json!({})),
        ("shortcuts.export", json!({"path": "/tmp/shortcuts.json"})),
        ("comp.savePreset", json!({"name": "Mine"})),
        ("layer.revealInFinder", json!({})),
        ("edit.purgeUndo", json!({})),
        ("layer.sceneEditDetection", json!({"wait": true})),
    ] {
        let refused = app.relay(command, params).unwrap_err();
        assert!(refused.contains("refused"), "{command}: {refused}");
    }
}

#[test]
fn an_effect_write_after_a_step_that_reshapes_the_document_needs_file_permissions() {
    let mut app = app("test.gate-reorder", EDITOR);
    app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "ec.blur.gaussian"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "ec.utility.applylut"})).unwrap();
    let blurriness = |app: &mut crate::App| app.session.execute_checked("prop.get", json!({"path": "effects/#1/blurriness"})).unwrap()["value"].clone();
    let before = blurriness(&mut app);
    for reshape in [
        json!({"command": "effect.duplicate", "params": {"effect": 1}}),
        json!({"command": "effect.reorder", "params": {"effect": 1, "index": 1}}),
        json!({"command": "layer.select", "params": {"layers": ["Graded"]}}),
    ] {
        let batch = json!({"steps": [reshape, {"command": "prop.set", "params": {"path": "effects/#1/blurriness", "value": 50.0}}]});
        let denied = app.relay("engine.batch", batch).unwrap_err();
        assert!(denied.contains("files.read"), "{denied}");
    }
    assert_eq!(blurriness(&mut app), before);
    let batch = json!({"steps": [
        {"command": "prop.set", "params": {"path": "transform/opacity", "value": 50.0}},
        {"command": "layer.select", "params": {"layers": ["Graded"]}},
        {"command": "prop.set", "params": {"path": "transform/rotation", "value": 10.0}},
    ]});
    app.relay("engine.batch", batch).expect("transform writes stay allowed after any step");
}

#[test]
fn commands_that_reach_outside_the_document_need_file_permissions() {
    let mut app = app("test.gate-outside", EDITOR);
    app.session.execute("layer.newText", json!({"text": "Title"})).unwrap();
    let fonts = app.relay("text.fonts", json!({"rescan": true})).unwrap_err();
    assert!(fonts.contains("files.read"), "{fonts}");
    let font = app.relay("layer.setText", json!({"font": "Inter"})).unwrap_err();
    assert!(font.contains("files.write"), "{font}");
    app.relay("layer.setText", json!({"text": "Subtitle"})).expect("changing the text alone stays a document edit");
}

#[test]
fn a_plugin_may_write_text_into_its_own_custom_parameter_but_not_into_another_plugins() {
    let mut app = app("test.gate-custom", EDITOR);
    app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.gate-custom.scale"})).unwrap();
    app.relay("prop.set", json!({"path": "effects/#1/level", "value": "0.5"})).expect("its own custom parameter is writable");
    let level = app.session.execute_checked("prop.get", json!({"path": "effects/#1/level"})).unwrap()["value"].clone();
    assert_eq!(level, json!("0.5"));

    let other = tempfile::tempdir().unwrap();
    std::fs::copy(app.bundle.path().join("probe.wasm"), other.path().join("probe.wasm")).unwrap();
    let manifest = json!({"id": "test.gate-other", "name": "Other", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": EDITOR});
    std::fs::write(other.path().join("plugin.json"), manifest.to_string()).unwrap();
    app.plugins.load_for_session(other.path()).unwrap();
    let request = json!({"command": "prop.set", "params": {"path": "effects/#1/level", "value": "/Users/someone/secret"}});
    std::fs::write(other.path().join("relay.json"), request.to_string()).unwrap();
    let denied = effectcraft_plugin::run_command(&mut app.session, "test.gate-other", "relay").unwrap_err();
    assert!(denied.contains("files.read"), "{denied}");
}

#[test]
fn changing_how_an_updated_plugin_runs_does_not_grant_permissions_the_user_has_not_approved() {
    let app = app("test.gate-mode-host", EDITOR);
    let id = "test.gate-mode";
    let source = tempfile::tempdir().unwrap();
    std::fs::copy(app.bundle.path().join("probe.wasm"), source.path().join("probe.wasm")).unwrap();
    let write = |version: &str, permissions: &[&str]| {
        let manifest = json!({"id": id, "name": "Mode", "version": version, "entry": {"wasm": "probe.wasm"}, "permissions": permissions});
        std::fs::write(source.path().join("plugin.json"), manifest.to_string()).unwrap();
    };
    write("1.0.0", &[]);
    app.plugins.install(source.path()).unwrap();
    app.plugins.approve(id).unwrap();
    assert!(app.plugins.get(id).is_some());

    write("1.1.0", &["document.read"]);
    app.plugins.install(source.path()).unwrap();
    assert!(app.plugins.get(id).is_none(), "the update waits for approval");
    app.plugins.set_sandboxed(id, true).unwrap();
    assert!(app.plugins.get(id).is_none(), "choosing the sandbox started a plugin whose new permission was never approved");

    app.plugins.approve(id).unwrap();
    let granted: Vec<&str> = app.plugins.get(id).unwrap().bundle.manifest.permissions.iter().map(|p| p.name()).collect();
    assert_eq!(granted, ["document.read"]);
    app.plugins.uninstall(id).unwrap();
}

#[test]
fn a_batch_cannot_plant_text_in_an_effect_parameter_by_its_uid_or_another_spelling() {
    let mut app = app("test.gate-lut-uid", EDITOR);
    app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "ec.utility.applylut"})).unwrap();
    let uid = app.session.execute_checked("prop.get", json!({"path": "effects/#1/lut"})).unwrap()["uid"].as_u64().unwrap();
    for path in [format!("@{uid}"), "Effects/#1/lut".to_string(), "#3/#1/lut".to_string()] {
        let batch = json!({"steps": [
            {"command": "layer.newNull", "params": {"name": "Other"}},
            {"command": "prop.set", "params": {"layer": "Graded", "path": path, "value": "/Users/someone/look.cube"}},
        ]});
        let denied = app.relay("engine.batch", batch).unwrap_err();
        assert!(denied.contains("files.read"), "{path}: {denied}");
    }
    let alone = app.relay("prop.set", json!({"layer": "Graded", "path": format!("@{uid}"), "value": "/Users/someone/look.cube"})).unwrap_err();
    assert!(alone.contains("files.read"), "{alone}");
    let lut = app.session.execute_checked("prop.get", json!({"layer": "Graded", "path": "effects/#1/lut"})).unwrap()["value"].clone();
    assert_eq!(lut, json!(""));
}

#[test]
fn a_plugin_whose_id_prefixes_another_cannot_write_text_into_the_others_custom_parameter() {
    let mut app = app("test.gate-pre", EDITOR);
    let longer = tempfile::tempdir().unwrap();
    std::fs::copy(app.bundle.path().join("probe.wasm"), longer.path().join("probe.wasm")).unwrap();
    let manifest = json!({"id": "test.gate-pre.pro", "name": "Pro", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": EDITOR});
    std::fs::write(longer.path().join("plugin.json"), manifest.to_string()).unwrap();
    app.plugins.load_for_session(longer.path()).unwrap();
    app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "test.gate-pre.pro.scale"})).unwrap();
    let denied = app.relay("prop.set", json!({"path": "effects/#1/level", "value": "/Users/someone/secret"})).unwrap_err();
    assert!(denied.contains("files.read"), "{denied}");
}

#[test]
fn keyframes_holding_text_cannot_be_pasted_without_file_permissions() {
    let mut app = app("test.gate-paste", EDITOR);
    let graded = app.session.execute("layer.newSolid", json!({"name": "Graded"})).unwrap()["layer"].clone();
    app.session.execute("effect.apply", json!({"effect": "test.gate-paste.scale"})).unwrap();
    app.session.execute("effect.apply", json!({"effect": "ec.utility.applylut"})).unwrap();
    app.relay("prop.addKey", json!({"path": "effects/#1/level", "value": "Users/someone/look.cube", "time": 0.0}))
        .expect("its own custom parameter is writable");
    let lut = app.session.execute_checked("prop.get", json!({"path": "effects/#2/lut"})).unwrap()["uid"].clone();
    app.session.execute("keys.selectAll", json!({"layers": [graded]})).unwrap();
    app.session.execute("keys.copy", json!({})).unwrap();
    let denied = app.relay("keys.paste", json!({"layers": [graded], "prop": lut})).unwrap_err();
    assert!(denied.contains("files.read"), "{denied}");
    let batch = json!({"steps": [{"command": "keys.copy", "params": {}}, {"command": "keys.paste", "params": {"prop": lut}}]});
    let denied = app.relay("engine.batch", batch).unwrap_err();
    assert!(denied.contains("files.read"), "batch: {denied}");
    let value = app.session.execute_checked("prop.get", json!({"path": "effects/#2/lut"})).unwrap()["value"].clone();
    assert_eq!(value, json!(""));
}

#[test]
fn text_cannot_be_written_through_an_essential_property_without_file_permissions() {
    let mut app = app("test.gate-essential", EDITOR);
    let denied = app.relay("essential.set", json!({"layer": 1, "control": "c1", "value": "/Users/someone/look.cube"})).unwrap_err();
    assert!(denied.contains("files.read"), "{denied}");
    let batch = json!({"steps": [{"command": "essential.set", "params": {"layer": 1, "control": "c1", "value": "/Users/someone/look.cube"}}]});
    let denied = app.relay("engine.batch", batch).unwrap_err();
    assert!(denied.contains("files.read"), "batch: {denied}");
    let number = app.relay("essential.set", json!({"layer": 1, "control": "c1", "value": 3.0})).unwrap_err();
    assert!(!number.contains("files.read"), "a number is not held back by the text rule: {number}");
}
