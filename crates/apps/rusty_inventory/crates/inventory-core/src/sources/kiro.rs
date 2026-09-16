//! Kiro — a VS Code fork; see [`super::vscdb`] for the shared reader.

use super::{vscdb, ScanContext, Source};
use crate::model::{ParsedConversation, SourceId};
use crate::Result;

pub struct Kiro;

impl Source for Kiro {
    fn id(&self) -> SourceId {
        SourceId::Kiro
    }

    fn files(&self) -> Vec<std::path::PathBuf> {
        vscdb::store_files(&self.roots())
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        vscdb::scan_fork(SourceId::Kiro, self.roots(), ctx)
    }
}
