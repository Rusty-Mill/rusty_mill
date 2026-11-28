//! `orch-fs-shell --root <dir> --mode builder|critic`: serve one session's
//! root over MCP on stdin/stdout.
//!
//! It confines nothing: see [`USAGE`] for what the critic mode does and does
//! not withhold.

use std::process::ExitCode;
use std::sync::Arc;

use orch_fs_shell::{server, FsWorkspace, Role};

/// The usage text, printed by `--help` and after every usage error. It says
/// where the boundary is, because the mode name alone suggests more than the
/// server does.
const USAGE: &str = "\
usage: orch-fs-shell --root <dir> --mode builder|critic

Serves read, list, stat, write and exec over MCP on stdin/stdout. Paths in
tool calls are relative to --root.

This server confines nothing. `--mode critic` withholds the `write` tool and
nothing else: `exec` still runs any program with this process's own rights,
so a critic's commands can write files, reach paths outside --root, and leave
descendants running. Read-only execution, filesystem confinement and cleanup
of an exec's descendants must come from the jail that launches this process;
without one, neither mode is confined.";

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Command {
    /// Print [`USAGE`] and exit.
    Help,
    /// Serve `root` in `role`.
    Serve(Args),
}

/// Where and how to serve.
#[derive(Debug, PartialEq)]
struct Args {
    root: String,
    role: Role,
}

fn parse(mut argv: impl Iterator<Item = String>) -> Result<Command, String> {
    let (mut root, mut role) = (None, None);
    while let Some(flag) = argv.next() {
        if matches!(flag.as_str(), "--help" | "-h") {
            return Ok(Command::Help);
        }
        let value = argv
            .next()
            .ok_or_else(|| format!("`{flag}` needs a value"))?;
        match flag.as_str() {
            "--root" => root = Some(value),
            "--mode" => role = Some(mode(&value)?),
            _ => return Err(format!("unknown argument `{flag}`")),
        }
    }
    Ok(Command::Serve(Args {
        root: root.ok_or("`--root` is required")?,
        role: role.ok_or("`--mode` is required")?,
    }))
}

fn mode(value: &str) -> Result<Role, String> {
    match value {
        "builder" => Ok(Role::Builder),
        "critic" => Ok(Role::Critic),
        _ => Err(format!("`--mode` is builder or critic, not `{value}`")),
    }
}

fn serve(args: &Args) -> Result<(), String> {
    let ws = FsWorkspace::new(&args.root).map_err(|e| format!("--root {}: {e}", args.root))?;
    let server = server::build(ws, args.role).map_err(|e| e.to_string())?;
    rusty_mcp_server::serve_stdio(Arc::new(server)).map_err(|e| format!("stdio: {e}"))
}

fn main() -> ExitCode {
    let run = match parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Serve(args)) => serve(&args),
        Err(usage) => Err(format!("{usage}\n\n{USAGE}")),
    };
    match run {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("orch-fs-shell: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Command, String> {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn flags_parse_and_are_checked() {
        let ok = args(&["--mode", "critic", "--root", "/w"]).expect("parse");
        let want = Args {
            root: "/w".to_owned(),
            role: Role::Critic,
        };
        assert_eq!(ok, Command::Serve(want));
        for (bad, want) in [
            (&["--root", "/w"][..], "--mode"),
            (&["--mode", "builder"][..], "--root"),
            (
                &["--mode", "admin", "--root", "/w"][..],
                "builder or critic",
            ),
            (&["--root"][..], "needs a value"),
            (&["--jail", "x"][..], "unknown"),
        ] {
            let err = args(bad).expect_err("refused");
            assert!(err.contains(want), "{bad:?}: {err}");
        }
    }

    #[test]
    fn help_wins_wherever_it_appears() {
        for argv in [&["--help"][..], &["-h"], &["--mode", "critic", "--help"]] {
            assert_eq!(args(argv), Ok(Command::Help), "{argv:?}");
        }
    }

    #[test]
    fn usage_says_the_critic_is_not_confined_here() {
        for claim in [
            "confines nothing",
            "withholds the `write` tool and\nnothing else",
            "`exec` still runs any program",
            "must come from the jail",
        ] {
            assert!(USAGE.contains(claim), "usage lost: {claim}");
        }
    }
}
