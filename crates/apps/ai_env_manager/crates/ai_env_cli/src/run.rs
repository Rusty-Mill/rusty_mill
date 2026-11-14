//! Runs a parsed command against the disk, through the core planners and the
//! `ai_env_fs` adapter. Output goes to a writer so tests can read it.

use crate::args::{Change, Command, Scope, USAGE};
use ai_env_core::{
    parse_catalog, plan_disable, plan_enable, Action, CatalogError, Entry, LinkMethod, Observed,
    Plan, Restricted, Tool, BUILTIN_TOOLS,
};
use std::fmt;
use std::io::{self, Write};
use std::path::Path;

/// How a run ended, for the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// A plan held a refusal: nothing was changed for that tool.
    Refused,
}

#[derive(Debug)]
pub enum Error {
    Catalog(CatalogError),
    UnknownTool(String),
    UnknownEntry(String),
    DuplicateEntry(String),
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
            Error::Fs(e) => e.fmt(f),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

macro_rules! from {
    ($($t:ty => $v:ident),*) => {$(impl From<$t> for Error { fn from(e: $t) -> Self { Error::$v(e) } })*};
}
from!(CatalogError => Catalog, ai_env_fs::Error => Fs, io::Error => Io);

/// Tools to act on: those named in `scope`, or all of the catalog.
fn select_tools(scope: &Scope, home: &Path) -> Result<Vec<Tool>, Error> {
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

/// Entries from every `--skills` folder, sorted by id; an id found twice is an
/// error because it would mean two sources for one link.
fn load_entries(scope: &Scope, out: &mut impl Write) -> Result<Vec<Entry>, Error> {
    let mut entries: Vec<Entry> = Vec::new();
    for dir in &scope.skills {
        let found = ai_env_fs::discover_skills(dir)?;
        for s in found.skipped {
            writeln!(out, "skipped {}: {}", s.path.display(), s.reason)?;
        }
        entries.extend(found.entries);
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(w) = entries.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(Error::DuplicateEntry(w[0].id.to_string()));
    }
    Ok(entries)
}

fn state(o: Observed) -> &'static str {
    match o {
        Observed::Absent => "-",
        Observed::ManagedCurrent => "enabled",
        Observed::ManagedStale => "stale",
        Observed::ManagedModified => "edited",
        Observed::Unmanaged => "foreign",
    }
}

fn list(scope: &Scope, home: &Path, out: &mut impl Write) -> Result<Outcome, Error> {
    let tools = select_tools(scope, home)?;
    for entry in load_entries(scope, out)? {
        write!(out, "{}", entry.id)?;
        for tool in &tools {
            write!(
                out,
                "  {}={}",
                tool.id,
                state(ai_env_fs::observe(&entry, tool)?)
            )?;
        }
        writeln!(out)?;
    }
    Ok(Outcome::Done)
}

fn show(plan: &Plan, out: &mut impl Write) -> io::Result<()> {
    for action in plan.actions() {
        match action {
            Action::Link { from, to, method } => writeln!(
                out,
                "  link    {} -> {} ({method:?})",
                to.display(),
                from.display()
            )?,
            Action::Unlink { path } => writeln!(out, "  unlink  {}", path.display())?,
            Action::Refuse { path, reason } => {
                writeln!(out, "  refuse  {} {reason}", path.display())?
            }
        }
    }
    Ok(())
}

fn change(c: &Change, enable: bool, home: &Path, out: &mut impl Write) -> Result<Outcome, Error> {
    let tools = select_tools(&c.scope, home)?;
    let entries = load_entries(&c.scope, out)?;
    let entry = entries
        .iter()
        .find(|e| e.id == c.entry)
        .ok_or_else(|| Error::UnknownEntry(c.entry.to_string()))?;
    let method = LinkMethod::fallback_chain(ai_env_fs::HOST)[0];
    let (mut outcome, mut pending) = (Outcome::Done, false);
    for tool in &tools {
        let observed = ai_env_fs::observe(entry, tool)?;
        let plan = if enable {
            plan_enable(entry, tool, observed, method)
        } else {
            plan_disable(entry, tool, observed)
        };
        writeln!(out, "{}:", tool.id)?;
        if plan.is_noop() {
            writeln!(out, "  nothing to do")?;
            continue;
        }
        show(&plan, out)?;
        if plan.has_refusals() {
            outcome = Outcome::Refused;
        } else if c.yes {
            ai_env_fs::apply(&plan)?;
        } else {
            pending = true;
        }
    }
    if pending {
        writeln!(out, "dry run: nothing changed. Add --yes to apply.")?;
    }
    Ok(outcome)
}

/// Runs `cmd`. `home` expands `~/` in the tool catalog.
pub fn run(cmd: &Command, home: &Path, out: &mut impl Write) -> Result<Outcome, Error> {
    match cmd {
        Command::Help => {
            writeln!(out, "{USAGE}")?;
            Ok(Outcome::Done)
        }
        Command::List(scope) => list(scope, home, out),
        Command::Enable(c) => change(c, true, home, out),
        Command::Disable(c) => change(c, false, home, out),
    }
}
