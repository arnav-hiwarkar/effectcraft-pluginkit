use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ROOT: &str = "effectcraft-test-scratch";
const LOCK_EXTENSION: &str = "lock";

pub(crate) struct ProcessDir(PathBuf);

impl ProcessDir {
    pub(crate) fn new() -> Self {
        Self(tempfile::Builder::new().prefix("dir-").tempdir_in(process_scratch()).expect("a scratch directory").keep())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

fn process_scratch() -> &'static Path {
    static SCRATCH: OnceLock<(PathBuf, File)> = OnceLock::new();
    &SCRATCH.get_or_init(open_scratch).0
}

fn open_scratch() -> (PathBuf, File) {
    let root = std::env::temp_dir().join(ROOT);
    std::fs::create_dir_all(&root).expect("the scratch root");
    sweep_finished_processes(&root);
    let scratch = tempfile::Builder::new().prefix("process-").tempdir_in(&root).expect("a scratch directory").keep();
    let lock = tempfile::NamedTempFile::new_in(&root).expect("the scratch lock");
    lock.as_file().lock().expect("the scratch lock is held while this process runs");
    let lock = lock.persist_noclobber(scratch.with_extension(LOCK_EXTENSION)).expect("publish the held scratch lock");
    (scratch, lock)
}

fn sweep_finished_processes(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for lock_path in entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == LOCK_EXTENSION)) {
        let Ok(lock) = File::options().write(true).open(&lock_path) else { continue };
        if lock.try_lock().is_ok() {
            let _ = std::fs::remove_dir_all(lock_path.with_extension(""));
            let _ = std::fs::remove_file(&lock_path);
        }
    }
}
