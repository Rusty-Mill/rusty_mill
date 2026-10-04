//! The command line. Hand-parsed over `std::env::args`: the surface is one
//! subcommand and a handful of flags, which does not justify a parser
//! dependency in an otherwise registry-free family (ADR-0009).

use std::path::PathBuf;

/// Printed for `--help` and on a usage error.
pub const USAGE: &str = "\
usage: rusty_orch run <goal.json> [options]

Runs the goal file through the dispatcher and prints the board.

options:
  --ollama-model <name>   model for Agent::Local       [env ORCH_OLLAMA_MODEL, default llama3.2]
  --repo <dir>            repository Codex and Claude may read
                                                       [env ORCH_REPO, default: current dir]
  --codex-repo <dir>      alias of --repo              [env ORCH_CODEX_REPO]
  --codex-model <name>    model for Agent::Codex       [default: Codex's own]
  --claude-model <name>   model for Agent::Claude      [default: Claude Code's own]
  --interactive           answer open questions from stdin and keep going
  --state <dir>           save the plan, board, and ledger there and resume
                          from it on the next run      [env RUSTY_ORCH_STATE]
  --json                  print the report as one JSON object
  -h, --help              this text

exit status: 0 finished, 3 blocked on questions, 4 budget or agent failure, 2 usage, 1 other error";

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub goal: PathBuf,
    pub ollama_model: String,
    /// The repository the CLI agents read, as their working directory.
    pub repo: PathBuf,
    pub codex_model: Option<String>,
    pub claude_model: Option<String>,
    pub interactive: bool,
    pub json: bool,
    /// Where to persist and resume from; `None` keeps the run in memory.
    pub state: Option<PathBuf>,
}

/// The two ways parsing can end without an [`Args`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    /// `--help` was asked for.
    Help,
    /// The command line was wrong; the message names the problem.
    Usage(String),
}

/// Parse `argv` without the program name. `env` resolves the fallback
/// environment variables, so tests need not touch the process environment.
pub fn parse(
    argv: impl IntoIterator<Item = String>,
    env: impl Fn(&str) -> Option<String>,
    cwd: PathBuf,
) -> Result<Args, ArgsError> {
    let mut argv = argv.into_iter();
    match argv.next().as_deref() {
        Some("run") => {}
        Some("-h" | "--help") => return Err(ArgsError::Help),
        Some(other) => return Err(usage(format!("unknown command {other:?}"))),
        None => return Err(usage("missing command".to_owned())),
    }
    let mut goal = None;
    let mut ollama_model = env("ORCH_OLLAMA_MODEL");
    let mut repo = env("ORCH_REPO")
        .or_else(|| env("ORCH_CODEX_REPO"))
        .map(PathBuf::from);
    let mut codex_model = None;
    let mut claude_model = None;
    let mut interactive = false;
    let mut json = false;
    let mut state = env("RUSTY_ORCH_STATE").map(PathBuf::from);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--ollama-model" => ollama_model = Some(value(&arg, argv.next())?),
            "--repo" | "--codex-repo" => repo = Some(PathBuf::from(value(&arg, argv.next())?)),
            "--codex-model" => codex_model = Some(value(&arg, argv.next())?),
            "--claude-model" => claude_model = Some(value(&arg, argv.next())?),
            "--interactive" => interactive = true,
            "--json" => json = true,
            "--state" => state = Some(PathBuf::from(value(&arg, argv.next())?)),
            "-h" | "--help" => return Err(ArgsError::Help),
            flag if flag.starts_with('-') => return Err(usage(format!("unknown option {flag:?}"))),
            _ if goal.is_none() => goal = Some(PathBuf::from(arg)),
            _ => return Err(usage(format!("unexpected argument {arg:?}"))),
        }
    }
    Ok(Args {
        goal: goal.ok_or_else(|| usage("missing <goal.json>".to_owned()))?,
        ollama_model: ollama_model.unwrap_or_else(|| "llama3.2".to_owned()),
        repo: repo.unwrap_or(cwd),
        codex_model,
        claude_model,
        interactive,
        json,
        state,
    })
}

fn value(flag: &str, next: Option<String>) -> Result<String, ArgsError> {
    next.filter(|v| !v.is_empty())
        .ok_or_else(|| usage(format!("{flag} needs a value")))
}

fn usage(message: String) -> ArgsError {
    ArgsError::Usage(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn minimal_run_uses_defaults() {
        let a = parse(argv("run goal.json"), no_env, PathBuf::from("/work")).expect("ok");
        assert_eq!(a.goal, PathBuf::from("goal.json"));
        assert_eq!(a.ollama_model, "llama3.2");
        assert_eq!(a.repo, PathBuf::from("/work"));
        assert_eq!(a.codex_model, None);
        assert_eq!(a.claude_model, None);
        assert!(!a.interactive && !a.json);
        assert_eq!(a.state, None);
    }

    #[test]
    fn state_comes_from_the_flag_or_the_environment() {
        let env = |k: &str| (k == "RUSTY_ORCH_STATE").then(|| "/env/state".to_owned());
        let a = parse(argv("run g.json"), env, PathBuf::from("/work")).expect("ok");
        assert_eq!(a.state, Some(PathBuf::from("/env/state")));
        let a = parse(
            argv("run --state .orch g.json"),
            env,
            PathBuf::from("/work"),
        )
        .expect("ok");
        assert_eq!(a.state, Some(PathBuf::from(".orch")));
        let e = parse(argv("run g.json --state"), no_env, PathBuf::from("/work"));
        assert_eq!(e, Err(ArgsError::Usage("--state needs a value".to_owned())));
    }

    #[test]
    fn repo_comes_from_orch_repo_before_the_codex_alias() {
        let both = |k: &str| match k {
            "ORCH_REPO" => Some("/env/repo".to_owned()),
            "ORCH_CODEX_REPO" => Some("/env/old".to_owned()),
            _ => None,
        };
        let a = parse(argv("run g.json"), both, PathBuf::from("/work")).expect("ok");
        assert_eq!(a.repo, PathBuf::from("/env/repo"));
        let old_only = |k: &str| (k == "ORCH_CODEX_REPO").then(|| "/env/old".to_owned());
        let a = parse(argv("run g.json"), old_only, PathBuf::from("/work")).expect("ok");
        assert_eq!(a.repo, PathBuf::from("/env/old"));
        let a = parse(
            argv("run --codex-repo /flag g.json"),
            both,
            PathBuf::from("/work"),
        )
        .expect("ok");
        assert_eq!(a.repo, PathBuf::from("/flag"));
    }

    #[test]
    fn flags_override_environment_which_overrides_defaults() {
        let env = |k: &str| match k {
            "ORCH_OLLAMA_MODEL" => Some("env-model".to_owned()),
            "ORCH_CODEX_REPO" => Some("/env/repo".to_owned()),
            _ => None,
        };
        let a = parse(argv("run g.json"), env, PathBuf::from("/work")).expect("ok");
        assert_eq!(a.ollama_model, "env-model");
        assert_eq!(a.repo, PathBuf::from("/env/repo"));
        let a = parse(
            argv(
                "run --ollama-model m --repo /r --codex-model c --claude-model k --interactive --json g.json",
            ),
            env,
            PathBuf::from("/work"),
        )
        .expect("ok");
        assert_eq!(a.ollama_model, "m");
        assert_eq!(a.repo, PathBuf::from("/r"));
        assert_eq!(a.claude_model.as_deref(), Some("k"));
        assert_eq!(a.codex_model.as_deref(), Some("c"));
        assert!(a.interactive && a.json);
    }

    #[test]
    fn errors_name_the_problem() {
        let cases = [
            ("", "missing command"),
            ("build", "unknown command"),
            ("run", "missing <goal.json>"),
            ("run --bogus g.json", "unknown option"),
            ("run g.json extra", "unexpected argument"),
            ("run g.json --ollama-model", "needs a value"),
        ];
        for (line, expected) in cases {
            match parse(argv(line), no_env, PathBuf::from("/")) {
                Err(ArgsError::Usage(m)) => assert!(m.contains(expected), "{line}: {m}"),
                other => panic!("{line}: {other:?}"),
            }
        }
        assert_eq!(
            parse(argv("--help"), no_env, PathBuf::from("/")),
            Err(ArgsError::Help)
        );
        assert_eq!(
            parse(argv("run -h"), no_env, PathBuf::from("/")),
            Err(ArgsError::Help)
        );
    }
}
