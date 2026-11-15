//! Ai Env Manager application layer (pack D32): the steps every front end
//! shares, as plain data and `Result`s with no output formatting. The CLI and
//! the GUI both call this; neither holds logic of its own. Planning stays in
//! `ai_env_core` (pure) and disk access in `ai_env_fs`.
//!
//! A change is always two steps: [`plan`] builds a [`Plan`] (nothing is
//! written), then [`apply`] writes it, only after the front end has shown it
//! and got consent (D6).

use ai_env_core::{
    parse_catalog, plan_disable, plan_edit, plan_enable, CatalogError, EditRefusal, Entry, EntryId,
    LinkMethod, Observed, Ownership, Plan, RelPath, Restricted, Tool, ToolId, BUILTIN_TOOLS,
};
use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

mod diff;

pub use ai_env_fs::Skipped;
pub use diff::{diff_lines, DiffLine};

/// Where to look: skill folders to scan and the tools to act on (empty means
/// every tool in the catalog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub skills: Vec<PathBuf>,
    pub tools: Vec<ToolId>,
}

/// Entries found, sorted by id, and the folders passed over (with reasons).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub entries: Vec<Entry>,
    pub skipped: Vec<Skipped>,
}

/// Why a step failed. `Display` gives the text front ends show the user.
#[derive(Debug)]
pub enum Error {
    Catalog(CatalogError),
    UnknownTool(String),
    UnknownEntry(String),
    DuplicateEntry(String),
    /// A skills folder could not be read.
    Folder(PathBuf, io::Error),
    Fs(ai_env_fs::Error),
    /// The tool has no copy or link of the entry to edit.
    NotEnabled(String),
    Edit(EditRefusal),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Catalog(e) => write!(f, "tool catalog: {e}"),
            Error::UnknownTool(t) => {
                write!(f, "unknown tool {t:?}; see `aienv list` for the catalog")
            }
            Error::UnknownEntry(e) => write!(f, "no skill {e:?} in the --skills folders"),
            Error::DuplicateEntry(e) => {
                write!(f, "skill {e:?} appears in more than one --skills folder")
            }
            Error::Folder(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Fs(e) => e.fmt(f),
            Error::NotEnabled(t) => write!(f, "the skill is not enabled in {t}"),
            Error::Edit(e) => e.fmt(f),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<CatalogError> for Error {
    fn from(e: CatalogError) -> Self {
        Error::Catalog(e)
    }
}

impl From<ai_env_fs::Error> for Error {
    fn from(e: ai_env_fs::Error) -> Self {
        Error::Fs(e)
    }
}

impl From<EditRefusal> for Error {
    fn from(e: EditRefusal) -> Self {
        Error::Edit(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// The user's home folder (`HOME`, else `USERPROFILE`), if set and not empty.
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Tools to act on: those named in `scope`, or all of the catalog. `home`
/// expands `~/` in the catalog.
pub fn select_tools(scope: &Scope, home: &Path) -> Result<Vec<Tool>, Error> {
    let all = parse_catalog(&Restricted, BUILTIN_TOOLS, home)?;
    if scope.tools.is_empty() {
        return Ok(all);
    }
    scope
        .tools
        .iter()
        .map(|id| {
            all.iter()
                .find(|t| &t.id == id)
                .cloned()
                .ok_or_else(|| Error::UnknownTool(id.to_string()))
        })
        .collect()
}

/// Entries from every skills folder, sorted by id; an id found twice is an
/// error because it would mean two sources for one link.
pub fn load_entries(scope: &Scope) -> Result<Loaded, Error> {
    let mut loaded = Loaded::default();
    for dir in &scope.skills {
        let found = ai_env_fs::discover_skills(dir).map_err(|e| Error::Folder(dir.clone(), e))?;
        loaded.skipped.extend(found.skipped);
        loaded.entries.extend(found.entries);
    }
    loaded.entries.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(w) = loaded.entries.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(Error::DuplicateEntry(w[0].id.to_string()));
    }
    Ok(loaded)
}

/// The entry named `id`, or [`Error::UnknownEntry`].
pub fn find_entry<'a>(entries: &'a [Entry], id: &EntryId) -> Result<&'a Entry, Error> {
    entries
        .iter()
        .find(|e| &e.id == id)
        .ok_or_else(|| Error::UnknownEntry(id.to_string()))
}

/// What `tool`'s folder holds for `entry` right now.
pub fn observe(entry: &Entry, tool: &Tool) -> Result<Observed, Error> {
    Ok(ai_env_fs::observe(entry, tool)?)
}

/// What to do with an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Enable,
    Disable,
}

/// The plan to `op` `entry` for `tool`. Reads the disk, writes nothing.
pub fn plan(entry: &Entry, tool: &Tool, op: Op) -> Result<Plan, Error> {
    let observed = observe(entry, tool)?;
    Ok(match op {
        Op::Enable => plan_enable(
            entry,
            tool,
            observed,
            LinkMethod::preferred(ai_env_fs::HOST),
        ),
        Op::Disable => plan_disable(entry, tool, observed),
    })
}

/// Writes `plan`. A plan holding a refusal is rejected whole and nothing is
/// changed (see `ai_env_fs::apply`); front ends should show it, not apply it.
pub fn apply(plan: &Plan) -> Result<(), Error> {
    Ok(ai_env_fs::apply(plan)?)
}

/// Which copy of an entry's files to work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The entry's source folder, where links come from.
    Source,
    /// What this tool has: the source if it links to it, else its marked copy.
    Tool(ToolId),
}

/// A file the editor can name, and whether it may be changed.
#[derive(Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub path: RelPath,
    pub editable: bool,
}

/// The files in a [`Place`] and who owns them.
#[derive(Debug, PartialEq, Eq)]
pub struct Listing {
    pub ownership: Ownership,
    pub files: Vec<FileInfo>,
}

/// A file as read: text and the hash to pass back when saving.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    pub ownership: Ownership,
    pub text: String,
    pub hash: String,
}

/// What saving would change; nothing is written to get one.
#[derive(Debug, PartialEq, Eq)]
pub struct EditPreview {
    pub ownership: Ownership,
    pub diff: Vec<DiffLine>,
    /// True when saving leaves a marked copy in the "edited" state.
    pub marks_edited: bool,
}

/// The folder to work in and who owns it. Looked up afresh on every call, so
/// a change since the last one is seen.
fn resolve(entry: &Entry, place: &Place, tools: &[Tool]) -> Result<(PathBuf, Ownership), Error> {
    let Place::Tool(id) = place else {
        return Ok((entry.source.clone(), Ownership::Source));
    };
    let tool = tools
        .iter()
        .find(|t| &t.id == id)
        .ok_or_else(|| Error::UnknownTool(id.to_string()))?;
    ai_env_fs::locate(entry, tool)?.ok_or_else(|| Error::NotEnabled(id.to_string()))
}

/// The files in `place`, each flagged editable or read-only.
pub fn list_files(entry: &Entry, place: &Place, tools: &[Tool]) -> Result<Listing, Error> {
    let (root, ownership) = resolve(entry, place, tools)?;
    let files = ai_env_fs::list_files(&root)?
        .into_iter()
        .map(|path| FileInfo {
            editable: ai_env_core::check_editable(&path, ownership).is_ok(),
            path,
        })
        .collect();
    Ok(Listing { ownership, files })
}

/// The text of `path` in `place`.
pub fn read_file(
    entry: &Entry,
    place: &Place,
    tools: &[Tool],
    path: &str,
) -> Result<Opened, Error> {
    let (root, ownership) = resolve(entry, place, tools)?;
    let read = ai_env_fs::read_file(&root, &RelPath::parse(path)?)?;
    Ok(Opened {
        ownership,
        text: read.text,
        hash: read.hash,
    })
}

/// Checks an edit against every rule and the file as it is now. `hash` is
/// the one [`read_file`] gave. Reads the disk, writes nothing.
fn check_edit(
    entry: &Entry,
    place: &Place,
    tools: &[Tool],
    path: &str,
    new_text: String,
    hash: &str,
) -> Result<(PathBuf, ai_env_core::EditPlan, String), Error> {
    let (root, ownership) = resolve(entry, place, tools)?;
    let path = RelPath::parse(path)?;
    let now = ai_env_fs::read_file(&root, &path)?;
    if now.hash != hash {
        return Err(ai_env_fs::Error::Changed(path.under(&root)).into());
    }
    let plan = plan_edit(
        entry.id.clone(),
        path,
        ownership,
        hash.to_owned(),
        &now.text,
        new_text,
    )?;
    Ok((root, plan, now.text))
}

/// The diff `new_text` would make to `path`. Writes nothing.
pub fn edit_preview(
    entry: &Entry,
    place: &Place,
    tools: &[Tool],
    path: &str,
    new_text: String,
    hash: &str,
) -> Result<EditPreview, Error> {
    let (_, plan, old) = check_edit(entry, place, tools, path, new_text, hash)?;
    Ok(EditPreview {
        ownership: plan.ownership(),
        diff: diff_lines(&old, plan.new_text()),
        marks_edited: plan.marks_edited(),
    })
}

/// Saves `new_text` over `path`. The front end calls this only after it
/// showed [`edit_preview`] and got consent (D6, D8). Every rule is checked
/// again here, so a file that changed or lost its owner since the preview is
/// refused; nothing from the preview is trusted.
pub fn edit_apply(
    entry: &Entry,
    place: &Place,
    tools: &[Tool],
    path: &str,
    new_text: String,
    hash: &str,
) -> Result<(), Error> {
    let (root, plan, _) = check_edit(entry, place, tools, path, new_text, hash)?;
    Ok(ai_env_fs::write_file(&root, &plan)?)
}

#[cfg(test)]
mod tests;
