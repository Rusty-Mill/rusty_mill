#![allow(dead_code)]
//! Test support: the four-role proving roster, read from the shipped
//! `docs/proving/roster.json` so the example config is exercised, and handles
//! for its four role ids.

use rusty_bbp::*;
use rusty_bbp_host::admin;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh, empty directory under the system temp dir, unique to this call.
///
/// Tests in one binary run in parallel and share a process id, and helpers
/// reuse tags (fixture names, `confine`/`mode` pairs), so a tag must never be
/// the only thing that names the directory: one test's setup would wipe
/// another's live run.
pub fn fresh_dir(prefix: &str, tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{n}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Planner,
    Coder,
    Tester,
    Reviewer,
}

impl Who {
    pub const ALL: [Who; 4] = [Who::Planner, Who::Coder, Who::Tester, Who::Reviewer];

    pub fn name(self) -> &'static str {
        match self {
            Who::Planner => "planner",
            Who::Coder => "coder",
            Who::Tester => "tester",
            Who::Reviewer => "reviewer",
        }
    }

    pub fn id(self) -> RoleId {
        RoleId::from(self.name())
    }
}

impl From<Who> for RoleId {
    fn from(w: Who) -> RoleId {
        w.id()
    }
}

/// The proving roster (spec Appendix C.1).
pub fn roster() -> Vec<Role> {
    admin::read_roster(&Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/proving/roster.json"))
        .expect("docs/proving/roster.json parses")
}
