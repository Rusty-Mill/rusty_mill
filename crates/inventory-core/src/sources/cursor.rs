//! Cursor — a VS Code fork; see [`super::vscdb`] for the shared reader.

use super::{vscdb, ScanContext, Source};
use crate::model::{ParsedConversation, SourceId};
use crate::Result;

pub struct Cursor;

impl Source for Cursor {
    fn id(&self) -> SourceId {
        SourceId::Cursor
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        vscdb::scan_fork(SourceId::Cursor, self.roots(), ctx)
    }
}
