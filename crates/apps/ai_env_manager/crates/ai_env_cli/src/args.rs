//! Command line parsing: pure, no I/O. Illegal states are unrepresentable: only
//! `enable` and `disable` carry an entry and `--yes`.

use ai_env_core::{EntryId, ToolId};
use std::fmt;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: aienv list --skills DIR... [--tool ID]...
       aienv enable ID --skills DIR... [--tool ID]... [--yes]
       aienv disable ID --skills DIR... [--tool ID]... [--yes]

  --skills DIR  a folder whose subfolders (each with a SKILL.md) are skills; repeatable
  --tool ID     limit to one tool from the catalog; repeatable (default: every tool)
  --yes         apply the plan; without it enable and disable only show the plan";

/// Where to look and which tools to consider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub skills: Vec<PathBuf>,
    pub tools: Vec<ToolId>,
}

/// One entry to change, and whether to apply the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub entry: EntryId,
    pub scope: Scope,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    List(Scope),
    Enable(Change),
    Disable(Change),
}

/// A command line that could not be understood, with what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

fn err<T>(msg: impl Into<String>) -> Result<T, UsageError> {
    Err(UsageError(msg.into()))
}

/// Parses everything after the program name.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let mut it = args.iter();
    let verb = match it.next().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => return Ok(Command::Help),
        Some(v @ ("list" | "enable" | "disable")) => v,
        Some(other) => return err(format!("unknown command {other:?}")),
    };
    let (mut skills, mut tools, mut yes, mut entry) = (Vec::new(), Vec::new(), false, None);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--skills" => skills.push(PathBuf::from(value(&mut it, "--skills")?)),
            "--tool" => tools.push(
                value(&mut it, "--tool")?
                    .parse()
                    .map_err(|e| UsageError(format!("{e}")))?,
            ),
            "--yes" => yes = true,
            flag if flag.starts_with('-') => return err(format!("unknown option {flag:?}")),
            id if entry.is_none() => {
                entry = Some(id.parse().map_err(|e| UsageError(format!("{e}")))?);
            }
            extra => return err(format!("unexpected argument {extra:?}")),
        }
    }
    if skills.is_empty() {
        return err("give at least one --skills DIR");
    }
    let scope = Scope { skills, tools };
    match (verb, entry) {
        ("list", None) if !yes => Ok(Command::List(scope)),
        ("list", None) => err("list changes nothing, so --yes does not apply"),
        ("list", Some(_)) => err("list takes no entry id"),
        (_, None) => err(format!("{verb} needs an entry id")),
        ("enable", Some(entry)) => Ok(Command::Enable(Change { entry, scope, yes })),
        (_, Some(entry)) => Ok(Command::Disable(Change { entry, scope, yes })),
    }
}

fn value<'a>(it: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<&'a str, UsageError> {
    it.next()
        .map(String::as_str)
        .ok_or_else(|| UsageError(format!("{flag} needs a value")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Command, UsageError> {
        parse(&s.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
    }

    #[test]
    fn parses_each_command() {
        assert_eq!(p("").unwrap(), Command::Help);
        assert_eq!(p("--help").unwrap(), Command::Help);
        let Command::List(s) = p("list --skills a --skills b --tool codex").unwrap() else {
            panic!("not list")
        };
        assert_eq!(s.skills, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(s.tools.len(), 1);
        let Command::Enable(c) = p("enable x --skills a --yes").unwrap() else {
            panic!("not enable")
        };
        assert!(c.yes && c.entry.as_str() == "x" && c.scope.tools.is_empty());
        let Command::Disable(c) = p("disable --skills a x").unwrap() else {
            panic!("not disable")
        };
        assert!(!c.yes && c.entry.as_str() == "x");
    }

    #[test]
    fn rejects_bad_command_lines() {
        for bad in [
            "frob",
            "list",                           // no --skills
            "list --skills",                  // missing value
            "list --skills a --yes",          // --yes on list
            "list x --skills a",              // entry on list
            "enable --skills a",              // no entry
            "enable x y --skills a",          // two entries
            "enable X --skills a",            // invalid id
            "enable x --skills a --tool Bad", // invalid tool id
            "enable x --skills a --nope",     // unknown flag
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
}
