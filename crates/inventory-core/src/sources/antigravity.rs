//! Antigravity — a VS Code fork; see [`super::vscdb`] for the shared reader.

use super::{vscdb, ScanContext, Source};
use crate::model::{ParsedConversation, SourceId};
use crate::Result;

pub struct Antigravity;

impl Source for Antigravity {
    fn id(&self) -> SourceId {
        SourceId::Antigravity
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        vscdb::scan_fork(SourceId::Antigravity, self.roots(), ctx)
    }
}
