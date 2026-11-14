//! Ai Env Manager application layer (pack D32): the steps every front end
//! shares, as plain data and `Result`s with no output formatting. The CLI and
//! the GUI both call this; neither holds logic of its own. Planning stays in
//! `ai_env_core` (pure) and disk access in `ai_env_fs`.
//!
//! A change is always two steps: [`plan`] builds a [`Plan`] (nothing is
//! written), then [`apply`] writes it, only after the front end has shown it
//! and got consent (D6).

use ai_env_core::{
    parse_catalog, plan_disable, plan_enable, CatalogError, Entry, EntryId, LinkMethod, Observed,
    Plan, Restricted, Tool, ToolId, BUILTIN_TOOLS,
};
use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub use ai_env_fs::Skipped;

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
