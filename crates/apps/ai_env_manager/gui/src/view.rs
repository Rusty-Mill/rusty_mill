//! What crosses the IPC boundary: serializable views of core types. Pure
//! mapping, no I/O, so it is tested without a window.

use ai_env_app::{
    Delta, DiffLine, EditPreview, FileBase, FileInfo, Listing, Opened, PushFile, PushPreview,
    Pushed,
};
use ai_env_core::{Action, Entry, Observed, Ownership, Plan, Tool};
use serde::{Deserialize, Serialize};
use std::path::Path;

fn path(p: &Path) -> String {
    p.display().to_string()
}

/// Stable machine names; the UI owns the wording.
pub fn state(o: Observed) -> &'static str {
    match o {
        Observed::Absent => "absent",
        Observed::ManagedCurrent => "enabled",
        Observed::ManagedStale => "stale",
        Observed::ManagedModified => "edited",
        Observed::Unmanaged => "foreign",
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Cell {
    pub tool: String,
    pub state: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub source: String,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ListView {
    pub tools: Vec<String>,
    pub rows: Vec<Row>,
    pub skipped: Vec<Skipped>,
}

pub fn row(entry: &Entry, cells: Vec<(&Tool, Observed)>) -> Row {
    Row {
        id: entry.id.to_string(),
        source: path(&entry.source),
        cells: cells
            .into_iter()
            .map(|(t, o)| Cell {
                tool: t.id.to_string(),
                state: state(o),
            })
            .collect(),
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ActionView {
    /// `link`, `unlink` or `refuse`.
    pub kind: &'static str,
    pub path: String,
    /// Link only: what the link points at.
    pub source: Option<String>,
    /// Link only: `Symlink`, `Junction` or `Copy`.
    pub method: Option<String>,
    /// Refuse only: why.
    pub reason: Option<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ToolPlan {
    pub tool: String,
    pub noop: bool,
    pub refused: bool,
    pub actions: Vec<ActionView>,
}

pub fn tool_plan(tool: &Tool, plan: &Plan) -> ToolPlan {
    let actions = plan
        .actions()
        .iter()
        .map(|a| match a {
            Action::Link { from, to, method } => ActionView {
                kind: "link",
                path: path(to),
                source: Some(path(from)),
                method: Some(format!("{method:?}")),
                reason: None,
            },
            Action::Unlink { path: p } => ActionView {
                kind: "unlink",
                path: path(p),
                source: None,
                method: None,
                reason: None,
            },
            Action::Refuse { path: p, reason } => ActionView {
                kind: "refuse",
                path: path(p),
                source: None,
                method: None,
                reason: Some(reason),
            },
        })
        .collect();
    ToolPlan {
        tool: tool.id.to_string(),
        noop: plan.is_noop(),
        refused: plan.has_refusals(),
        actions,
    }
}

/// What the page asks for. Serialized as `"enable"` or `"disable"`.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    Enable,
    Disable,
}

impl From<Verb> for ai_env_app::Op {
    fn from(v: Verb) -> Self {
        match v {
            Verb::Enable => ai_env_app::Op::Enable,
            Verb::Disable => ai_env_app::Op::Disable,
        }
    }
}

/// How one tool's apply ended.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Applied,
    /// Already as asked.
    Nothing,
    /// Left untouched.
    Refused,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ToolResult {
    pub tool: String,
    pub outcome: Outcome,
}

/// Stable machine names for who owns a file; the UI owns the wording.
pub fn ownership(o: Ownership) -> &'static str {
    match o {
        Ownership::Source => "source",
        Ownership::MarkedCopy => "copy",
        Ownership::Unmanaged => "foreign",
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileItem {
    pub path: String,
    pub editable: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FilesView {
    pub ownership: &'static str,
    pub files: Vec<FileItem>,
}

pub fn files(l: Listing) -> FilesView {
    FilesView {
        ownership: ownership(l.ownership),
        files: l
            .files
            .into_iter()
            .map(|FileInfo { path, editable }| FileItem {
                path: path.as_str().to_owned(),
                editable,
            })
            .collect(),
    }
}

/// A file as read. The page sends `hash` back unchanged when it saves.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileView {
    pub ownership: &'static str,
    pub text: String,
    pub hash: String,
}

pub fn file(o: Opened) -> FileView {
    FileView {
        ownership: ownership(o.ownership),
        text: o.text,
        hash: o.hash,
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DiffItem {
    /// `same`, `added` or `removed`.
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PreviewView {
    pub ownership: &'static str,
    /// True when saving leaves a marked copy in the "edited" state.
    pub marks_edited: bool,
    pub diff: Vec<DiffItem>,
}

fn diff(lines: Vec<DiffLine>) -> Vec<DiffItem> {
    lines
        .into_iter()
        .map(|l| match l {
            DiffLine::Same(text) => DiffItem { kind: "same", text },
            DiffLine::Added(text) => DiffItem {
                kind: "added",
                text,
            },
            DiffLine::Removed(text) => DiffItem {
                kind: "removed",
                text,
            },
        })
        .collect()
}

pub fn preview(p: EditPreview) -> PreviewView {
    PreviewView {
        ownership: ownership(p.ownership),
        marks_edited: p.marks_edited,
        diff: diff(p.diff),
    }
}

/// What the page sends back to push a file: the hashes it was shown.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BaseIn {
    pub path: String,
    pub source_hash: String,
    pub copy_hash: String,
}

impl From<BaseIn> for FileBase {
    fn from(b: BaseIn) -> Self {
        FileBase {
            path: b.path,
            source_hash: b.source_hash,
            copy_hash: b.copy_hash,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PushFileView {
    pub path: String,
    pub source_hash: String,
    pub copy_hash: String,
    pub diff: Vec<DiffItem>,
}

/// A path a push leaves alone, and why.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct LeftView {
    pub path: String,
    pub why: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PushPreviewView {
    pub files: Vec<PushFileView>,
    pub left: Vec<LeftView>,
}

fn left(d: Vec<ai_env_app::Difference>) -> Vec<LeftView> {
    d.into_iter()
        .map(|d| LeftView {
            path: d.path,
            why: match d.delta {
                Delta::Differs => "differs, not selected",
                Delta::OnlyInCopy => "only in the copy; a push never creates files",
                Delta::OnlyInSource => "only in the source; a push never deletes files",
                Delta::KindDiffers => "a file on one side and a folder on the other",
            },
        })
        .collect()
}

pub fn push_preview(p: PushPreview) -> PushPreviewView {
    PushPreviewView {
        files: p
            .files
            .into_iter()
            .map(|PushFile { base, diff: d }| PushFileView {
                path: base.path,
                source_hash: base.source_hash,
                copy_hash: base.copy_hash,
                diff: diff(d),
            })
            .collect(),
        left: left(p.left),
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PushedView {
    pub written: Vec<String>,
    /// True when the copy now matches the source and reads enabled again.
    pub remarked: bool,
    pub left: Vec<LeftView>,
}

pub fn pushed(p: Pushed) -> PushedView {
    PushedView {
        written: p.written,
        remarked: p.remarked,
        left: left(p.left),
    }
}
