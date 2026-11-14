//! Runs a parsed command against the disk through `ai_env_app` and prints the
//! result. Output goes to a writer so tests can read it.

use crate::args::{Change, Command, Scope, USAGE};
use ai_env_app::{apply, find_entry, load_entries, observe, plan, select_tools, Op};
use ai_env_core::{Action, Observed, Plan};
use std::io::{self, Write};
use std::path::Path;

pub use ai_env_app::Error;

/// How a run ended, for the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// A plan held a refusal: nothing was changed for that tool.
    Refused,
}

/// Notes on skipped folders go to `err`.
fn entries(scope: &Scope, err: &mut impl Write) -> Result<Vec<ai_env_core::Entry>, Error> {
    let loaded = load_entries(scope)?;
    for s in loaded.skipped {
        writeln!(err, "skipped {}: {}", s.path.display(), s.reason)?;
    }
    Ok(loaded.entries)
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

fn list(
    scope: &Scope,
    home: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Outcome, Error> {
    let tools = select_tools(scope, home)?;
    for entry in entries(scope, err)? {
        write!(out, "{}", entry.id)?;
        for tool in &tools {
            write!(out, "  {}={}", tool.id, state(observe(&entry, tool)?))?;
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

fn change(
    c: &Change,
    op: Op,
    home: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Outcome, Error> {
    let tools = select_tools(&c.scope, home)?;
    let entries = entries(&c.scope, err)?;
    let entry = find_entry(&entries, &c.entry)?;
    let (mut outcome, mut pending) = (Outcome::Done, false);
    for tool in &tools {
        let plan = plan(entry, tool, op)?;
        writeln!(out, "{}:", tool.id)?;
        if plan.is_noop() {
            writeln!(out, "  nothing to do")?;
            continue;
        }
        show(&plan, out)?;
        if plan.has_refusals() {
            outcome = Outcome::Refused;
        } else if c.yes {
            apply(&plan)?;
            writeln!(out, "  applied")?;
        } else {
            pending = true;
        }
    }
    if pending {
        writeln!(out, "dry run: nothing changed. Add --yes to apply.")?;
    }
    Ok(outcome)
}

/// Runs `cmd`. `home` expands `~/` in the tool catalog. Results go to `out`;
/// notes about skipped folders go to `err`. Tools are applied one at a time and
/// each says "applied" when done, so after a failure the output shows which
/// tools had already changed.
pub fn run(
    cmd: &Command,
    home: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Outcome, Error> {
    match cmd {
        Command::Help => {
            writeln!(out, "{USAGE}")?;
            Ok(Outcome::Done)
        }
        Command::List(scope) => list(scope, home, out, err),
        Command::Enable(c) => change(c, Op::Enable, home, out, err),
        Command::Disable(c) => change(c, Op::Disable, home, out, err),
    }
}
