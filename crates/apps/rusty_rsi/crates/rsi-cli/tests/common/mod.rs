//! Helpers shared by the integration tests: the real `rsi` binary, the
//! toy task suite and per-test scratch directories.
// Each test file uses a different subset.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use rsi_runtime::{ProcessExecutor, TaskDir};

/// The `rsi` binary under test: sandbox helper and grader.
pub const RSI: &str = env!("CARGO_BIN_EXE_rsi");

/// A per-test directory under the system temp dir, removed on success.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rsi-it-{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("clearing an old scratch dir");
        }
        std::fs::create_dir_all(&dir).expect("creating scratch");
        Self(dir.canonicalize().expect("canonical scratch"))
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            std::fs::remove_dir_all(&self.0).expect("removing scratch");
        }
    }
}

pub fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../rsi-runtime/tasks")
}

pub fn suite() -> Vec<TaskDir> {
    TaskDir::load_suite(&suite_dir()).expect("toy suite loads")
}

pub fn task(tasks: &[TaskDir], id: &str) -> TaskDir {
    tasks
        .iter()
        .find(|t| t.manifest().id.as_str() == id)
        .cloned()
        .expect("task exists")
}

pub fn executor(scratch: &Scratch) -> ProcessExecutor {
    ProcessExecutor::new(RSI.into(), vec!["__sandbox".into()], scratch.path("state"))
}
