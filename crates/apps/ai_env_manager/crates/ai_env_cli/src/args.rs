//! Command line parsing: pure, no I/O. Illegal states are unrepresentable: only
//! `enable`, `disable` and `edit` carry an entry and `--yes`.

pub use ai_env_app::Scope;
use ai_env_core::{EntryId, ToolId};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: aienv list --skills DIR... [--tool ID]...
       aienv enable ID --skills DIR... [--tool ID]... [--yes]
       aienv disable ID --skills DIR... [--tool ID]... [--yes]
       aienv edit ID FILE --skills DIR... [--tool ID] [--from PATH|-] [--yes]
       aienv push ID [FILE] --tool ID --skills DIR... [--yes]

  --skills DIR  a folder whose subfolders (each with a SKILL.md) are skills; repeatable
  --tool ID     limit to one tool from the catalog; repeatable (default: every tool)
  --yes         apply the plan; without it enable and disable only show the plan
  edit          change one text file of a skill (FILE is relative to the skill folder). The
                new text comes from --from (a file, or - for stdin), else from $VISUAL or
                $EDITOR. It shows the diff and writes only on --yes (or a typed y after the
                editor). Without --tool the skill's source is edited, else that tool's copy.
  push          copy the edits made in a tool's copy back to the skill's source (one FILE, or
                every changed file). It shows the diff and writes only on --yes. The source
                is overwritten, including any change made there since the copy was taken.

exit status: 0 done, 1 a link plan held a refusal (nothing changed there), 2 usage (or the
editor with no terminal to confirm on), 3 error (including an edit or push that was refused)";

/// One entry to change, and whether to apply the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub entry: EntryId,
    pub scope: Scope,
    pub yes: bool,
}

/// Where the new text of an `edit` comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewText {
    /// `$VISUAL` or `$EDITOR`, run on a temporary copy.
    Editor,
    Stdin,
    File(PathBuf),
}

/// One file to edit. `scope.tools` holds at most one tool: without it the
/// skill's source is edited, with it that tool's copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditFile {
    pub entry: EntryId,
    pub file: String,
    pub scope: Scope,
    pub text: NewText,
    pub yes: bool,
}

/// Copy a tool's edited copy back to the source: `file` alone, else all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    pub entry: EntryId,
    pub file: Option<String>,
    pub tool: ToolId,
    pub scope: Scope,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    List(Scope),
    Enable(Change),
    Disable(Change),
    Edit(EditFile),
    Push(Push),
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

/// Turns the process arguments into text. A non-UTF-8 argument is a usage
/// error, not a panic (Linux paths need not be UTF-8).
pub fn from_os(args: impl IntoIterator<Item = OsString>) -> Result<Vec<String>, UsageError> {
    args.into_iter()
        .map(|a| {
            a.into_string()
                .map_err(|a| UsageError(format!("argument {a:?} is not valid UTF-8")))
        })
        .collect()
}

/// Parses everything after the program name.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let mut it = args.iter();
    let verb = match it.next().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => return Ok(Command::Help),
        Some(v @ ("list" | "enable" | "disable" | "edit" | "push")) => v,
        Some(other) => return err(format!("unknown command {other:?}")),
    };
    let (mut skills, mut tools, mut yes, mut from) = (Vec::new(), Vec::new(), false, None);
    let mut positional = Vec::new();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--skills" => skills.push(PathBuf::from(value(&mut it, "--skills")?)),
            "--tool" => {
                let id: ToolId = value(&mut it, "--tool")?
                    .parse()
                    .map_err(|e| UsageError(format!("{e}")))?;
                if tools.contains(&id) {
                    return err(format!("--tool {id} given twice"));
                }
                tools.push(id);
            }
            "--yes" => yes = true,
            "--from" => {
                if from.is_some() {
                    return err("--from given twice");
                }
                from = Some(value(&mut it, "--from")?);
            }
            "-" => positional.push(arg),
            flag if flag.starts_with('-') => return err(format!("unknown option {flag:?}")),
            _ => positional.push(arg),
        }
    }
    if skills.is_empty() {
        return err("give at least one --skills DIR");
    }
    let scope = Scope { skills, tools };
    let mut words = positional.into_iter();
    let entry = words
        .next()
        .map(|id| id.parse().map_err(|e| UsageError(format!("{e}"))))
        .transpose()?;
    let file = words.next();
    if let Some(extra) = words.next() {
        return err(format!("unexpected argument {extra:?}"));
    }
    if !matches!(verb, "edit" | "push") && file.is_some() {
        return err(format!("{verb} takes no file"));
    }
    if verb != "edit" && from.is_some() {
        return err(format!("{verb} takes no --from"));
    }
    match (verb, entry) {
        ("list", None) if !yes => Ok(Command::List(scope)),
        ("list", None) => err("list changes nothing, so --yes does not apply"),
        ("list", Some(_)) => err("list takes no entry id"),
        (_, None) => err(format!("{verb} needs an entry id")),
        ("enable", Some(entry)) => Ok(Command::Enable(Change { entry, scope, yes })),
        ("edit", Some(entry)) => edit(entry, file, from, scope, yes),
        ("push", Some(entry)) => push(entry, file, scope, yes),
        (_, Some(entry)) => Ok(Command::Disable(Change { entry, scope, yes })),
    }
}

fn push(
    entry: EntryId,
    file: Option<&String>,
    scope: Scope,
    yes: bool,
) -> Result<Command, UsageError> {
    let [tool] = scope.tools.as_slice() else {
        return err("push needs exactly one --tool: the copy to push back");
    };
    Ok(Command::Push(Push {
        entry,
        file: file.cloned(),
        tool: tool.clone(),
        scope: scope.clone(),
        yes,
    }))
}

fn edit(
    entry: EntryId,
    file: Option<&String>,
    from: Option<&str>,
    scope: Scope,
    yes: bool,
) -> Result<Command, UsageError> {
    let Some(file) = file else {
        return err("edit needs a file, relative to the skill folder (try SKILL.md)");
    };
    if scope.tools.len() > 1 {
        return err("edit works on one copy: give --tool at most once");
    }
    let text = match from {
        None => NewText::Editor,
        Some("-") => NewText::Stdin,
        Some(path) => NewText::File(PathBuf::from(path)),
    };
    Ok(Command::Edit(EditFile {
        entry,
        file: file.clone(),
        scope,
        text,
        yes,
    }))
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
    fn parses_edit() {
        let Command::Edit(e) = p("edit x SKILL.md --skills a --from - --yes").unwrap() else {
            panic!("not edit")
        };
        assert!(e.yes && e.text == NewText::Stdin && e.file == "SKILL.md");
        let Command::Edit(e) = p("edit x a/b.md --skills a --tool codex --from n.md").unwrap()
        else {
            panic!("not edit")
        };
        assert_eq!(e.text, NewText::File(PathBuf::from("n.md")));
        assert_eq!(e.scope.tools.len(), 1);
        let Command::Edit(e) = p("edit x SKILL.md --skills a").unwrap() else {
            panic!("not edit")
        };
        assert_eq!(e.text, NewText::Editor);
    }

    #[test]
    fn parses_push() {
        let Command::Push(c) = p("push x --skills a --tool codex").unwrap() else {
            panic!("not push")
        };
        assert!(c.file.is_none() && !c.yes && c.tool.to_string() == "codex");
        let Command::Push(c) = p("push x SKILL.md --skills a --tool codex --yes").unwrap() else {
            panic!("not push")
        };
        assert_eq!(c.file.as_deref(), Some("SKILL.md"));
        for bad in [
            "push x --skills a",                                 // no tool
            "push x --skills a --tool codex --tool claude-code", // two tools
            "push x f g --skills a --tool codex",                // extra
            "push x --skills a --tool codex --from f",           // --from
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_bad_edit_lines() {
        for bad in [
            "edit x --skills a",                                   // no file
            "edit x f --skills a --tool codex --tool claude-code", // two copies
            "edit x f --skills a --from",                          // missing value
            "edit x f --skills a --from a --from b",               // two sources
            "enable x f --skills a",                               // file on enable
            "enable x --skills a --from b",                        // --from on enable
            "edit x f g --skills a",                               // extra
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_argument_is_a_usage_error() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'a', 0xff]);
        assert!(from_os([OsString::from("list"), bad]).is_err());
        assert_eq!(from_os([OsString::from("list")]).unwrap(), ["list"]);
    }

    #[test]
    fn rejects_bad_command_lines() {
        for bad in [
            "frob",
            "list",                                          // no --skills
            "list --skills",                                 // missing value
            "list --skills a --yes",                         // --yes on list
            "list x --skills a",                             // entry on list
            "enable --skills a",                             // no entry
            "enable x y --skills a",                         // two entries
            "enable X --skills a",                           // invalid id
            "enable x --skills a --tool Bad",                // invalid tool id
            "enable x --skills a --nope",                    // unknown flag
            "enable x --skills a --tool codex --tool codex", // same tool twice
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
}
