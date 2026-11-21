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
