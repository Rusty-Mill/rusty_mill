//! Push a marked copy's edits back to the entry's source (pack D42). Two
//! steps like every change: [`push_preview`] reads and writes nothing,
//! [`push_apply`] re-checks both sides and then writes the source files.
//!
//! The marker keeps only the copy's tree hash, so there is no common base: a
//! source file changed since the copy was taken is overwritten by the push,
//! and the preview's diff (source to copy) is what shows that.

use crate::{diff_lines, resolve, DiffLine, Error, Place, Tool};
use ai_env_core::{plan_edit, Entry, Observed, Ownership, RelPath, ToolId};
use ai_env_fs::{Delta, Difference};
use std::path::{Path, PathBuf};

/// What a file looked like when previewed; [`push_apply`] refuses if either
/// side has changed since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBase {
    pub path: String,
    pub source_hash: String,
    pub copy_hash: String,
}

/// One file that would change in the source.
#[derive(Debug, PartialEq, Eq)]
pub struct PushFile {
    pub base: FileBase,
    /// Source text to copy text.
    pub diff: Vec<DiffLine>,
}

/// What a push would do.
#[derive(Debug, PartialEq, Eq)]
pub struct PushPreview {
    pub files: Vec<PushFile>,
    /// Differences a push cannot fix (files or folders on one side only) or
    /// was not asked to. While any remain, the copy stays "edited".
    pub left: Vec<Difference>,
}

/// What a push did.
#[derive(Debug, PartialEq, Eq)]
pub struct Pushed {
    pub written: Vec<String>,
    /// True when the copy now equals the source and reads enabled again.
    pub remarked: bool,
    /// Paths that still differ (see [`PushPreview::left`]).
    pub left: Vec<Difference>,
}

/// The copy's folder, only if it holds our marker and was edited.
fn edited_copy(entry: &Entry, tool: &ToolId, tools: &[Tool]) -> Result<PathBuf, Error> {
    let (copy, ownership) = resolve(entry, &Place::Tool(tool.clone()), tools)?;
    let t = tools.iter().find(|t| &t.id == tool);
    let edited =
        t.is_some_and(|t| matches!(ai_env_fs::observe(entry, t), Ok(Observed::ManagedModified)));
    if ownership != Ownership::MarkedCopy || !edited {
        return Err(Error::NotEdited(tool.to_string()));
    }
    Ok(copy)
}

/// Both sides of `path`, read: (source text, source hash, copy text, copy hash).
fn both(
    entry: &Entry,
    copy: &Path,
    path: &RelPath,
) -> Result<(ai_env_fs::FileRead, ai_env_fs::FileRead), Error> {
    Ok((
        ai_env_fs::read_file(&entry.source, path)?,
        ai_env_fs::read_file(copy, path)?,
    ))
}

/// What pushing `only` (one file, else every file that differs) from `tool`'s
/// edited copy to the source would change. Writes nothing.
pub fn push_preview(
    entry: &Entry,
    tool: &ToolId,
    tools: &[Tool],
    only: Option<&str>,
) -> Result<PushPreview, Error> {
    let copy = edited_copy(entry, tool, tools)?;
    let only = only.map(RelPath::parse).transpose()?;
    let (mut files, mut left) = (Vec::new(), Vec::new());
    for d in ai_env_fs::differences(&entry.source, &copy)? {
        let wanted = only.as_ref().is_none_or(|o| o.as_str() == d.path);
        if d.delta != Delta::Differs || !wanted {
            left.push(d);
            continue;
        }
        let path = RelPath::parse(&d.path)?;
        let (src, dst) = both(entry, &copy, &path)?;
        files.push(PushFile {
            diff: diff_lines(&src.text, &dst.text),
            base: FileBase {
                path: d.path,
                source_hash: src.hash,
                copy_hash: dst.hash,
            },
        });
    }
    if let (Some(o), true) = (&only, files.is_empty()) {
        return Err(Error::NothingToPush(o.as_str().to_owned()));
    }
    Ok(PushPreview { files, left })
}

/// Writes the previewed files to the source. Every file is checked first
/// (both sides unchanged since `bases`, rules as for any edit), so a refusal
/// changes nothing; a write error part-way leaves the files before it pushed,
/// and a new preview shows what is left. The caller gets here only after the
/// user confirmed the preview (D6, D8).
pub fn push_apply(
    entry: &Entry,
    tool: &ToolId,
    tools: &[Tool],
    bases: &[FileBase],
) -> Result<Pushed, Error> {
    let copy = edited_copy(entry, tool, tools)?;
    let mut plans = Vec::new();
    for b in bases {
        let path = RelPath::parse(&b.path)?;
        let (src, dst) = both(entry, &copy, &path)?;
        if src.hash != b.source_hash || dst.hash != b.copy_hash {
            return Err(ai_env_fs::Error::Changed(path.under(&copy)).into());
        }
        plans.push(plan_edit(
            entry.id.clone(),
            path,
            Ownership::Source,
            src.hash,
            &src.text,
            dst.text,
        )?);
    }
    for plan in &plans {
        ai_env_fs::write_file(&entry.source, plan)?;
    }
    let remarked = ai_env_fs::remark_if_equal(entry, &copy)?;
    let left = ai_env_fs::differences(&entry.source, &copy)?;
    Ok(Pushed {
        written: bases.iter().map(|b| b.path.clone()).collect(),
        remarked,
        left,
    })
}
