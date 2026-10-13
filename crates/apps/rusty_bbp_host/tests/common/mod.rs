#![allow(dead_code)]
//! Test support: the four-role proving roster, read from the shipped
//! `docs/proving/roster.json` so the example config is exercised, and handles
//! for its four role ids.

use rusty_bbp::*;
use rusty_bbp_host::admin;
use std::path::Path;

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
