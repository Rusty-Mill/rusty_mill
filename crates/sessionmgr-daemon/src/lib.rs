//! `sessionmgr`'s composition root.
//!
//! One binary, three roles:
//!
//! | Role | Invocation | Lifetime |
//! |---|---|---|
//! | Supervisor daemon | `sessionmgr daemon run` | long-running; outlives the UI |
//! | Session worker | `sessionmgr __worker-main …` | detached; outlives the daemon |
//! | Client | `new` / `list` / `attach` / `close` | one command |
//!
//! Argument parsing is hand-rolled rather than pulling in a parser crate.
//! The surface is a dozen flags with no interdependencies, and the
//! sibling projects this one reuses patterns from hold the same minimal-
//! dependency line.

pub mod catalog;
pub mod client;
pub mod error;
pub mod paths;
pub mod supervisor;
pub mod transport;
pub mod worker;

use std::path::{Path, PathBuf};

use sessionmgr_core::{SessionId, SessionKind};

use crate::error::{Error, Result};

pub const USAGE: &str = "\
sessionmgr - manage AI coding-agent CLI sessions that survive this app closing

USAGE:
    sessionmgr <COMMAND>

COMMANDS:
    new [--kind terminal] [-- <command>...]   create a session and start it
    list                                      list every session
    attach <id>                               stream a session's output
    close <id>                                tear a session down
    daemon run                                run the supervisor in the foreground
    daemon start                              start the supervisor detached
    daemon status                             is a supervisor running?
    daemon shutdown                           stop the supervisor (sessions keep running)

GLOBAL OPTIONS:
    --state-root <path>   where to keep state (default: $SESSIONMGR_HOME, else
                          the platform's per-user state directory)
";

/// Parses and runs one invocation.
pub async fn run(args: &[String]) -> Result<()> {
    let mut args = args.to_vec();
    let root = client::resolve_root(take_option(&mut args, "--state-root")?.map(PathBuf::from))?;

    let Some(command) = args.first().cloned() else {
        println!("{USAGE}");
        return Ok(());
    };
    let rest = &args[1..];

    match command.as_str() {
        "new" => cmd_new(&root, rest).await,
        "list" => {
            let sessions = client::session_list(&root).await?;
            println!("{}", client::render_sessions(&sessions));
            Ok(())
        }
        "attach" => {
            let id = parse_id(rest.first())?;
            client::session_attach(&root, id).await
        }
        "close" => {
            let id = parse_id(rest.first())?;
            client::session_close(&root, id).await?;
            println!("closed");
            Ok(())
        }
        "daemon" => cmd_daemon(&root, rest).await,
        "__worker-main" => {
            let mut rest = rest.to_vec();
            let id = take_option(&mut rest, "--session-id")?
                .ok_or_else(|| Error::usage("__worker-main requires --session-id"))?;
            // The worker's state root is passed explicitly by the daemon
            // that spawned it, and is not re-derived here: a worker must
            // serve the same root its supervisor recorded it under, even
            // if the environment has since changed underneath it.
            let state_root = take_option(&mut rest, "--state-root")?
                .map(PathBuf::from)
                .unwrap_or(root);
            worker::run(worker::WorkerArgs {
                session_id: parse_id(Some(&id))?,
                state_root,
            })
            .await
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(Error::usage(format!(
            "unknown command `{other}`\n\n{USAGE}"
        ))),
    }
}

async fn cmd_new(root: &Path, args: &[String]) -> Result<()> {
    let mut args = args.to_vec();
    let kind = match take_option(&mut args, "--kind")?.as_deref() {
        // Phase 1 knows exactly one kind. `--kind` exists now so the
        // interface does not change shape when Phase 2 adds the other
        // two, and so an unsupported value fails with a clear message
        // rather than being silently ignored.
        None | Some("terminal") => SessionKind::PlainTerminal,
        Some(other) => {
            return Err(Error::usage(format!(
                "unknown session kind `{other}` (this phase supports `terminal` only; \
                 worktree and same-directory sessions arrive with Phase 2's worktree lifecycle)"
            )))
        }
    };
    // Everything after `--` is the command to run.
    let command = match args.iter().position(|arg| arg == "--") {
        Some(index) => args[index + 1..].to_vec(),
        None => Vec::new(),
    };
    let id = client::session_new(root, kind, command).await?;
    println!("{id}");
    Ok(())
}

async fn cmd_daemon(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("run") => supervisor::run(root.to_path_buf()).await,
        Some("start") => {
            client::ensure_daemon(root).await?;
            println!("{}", client::daemon_status(root).await?);
            Ok(())
        }
        Some("status") => {
            println!("{}", client::daemon_status(root).await?);
            Ok(())
        }
        Some("shutdown") => {
            client::daemon_shutdown(root).await?;
            println!("stopped");
            Ok(())
        }
        Some(other) => Err(Error::usage(format!("unknown daemon command `{other}`"))),
        None => Err(Error::usage("daemon requires a subcommand")),
    }
}

fn parse_id(raw: Option<&String>) -> Result<SessionId> {
    let raw = raw.ok_or_else(|| Error::usage("this command requires a session id"))?;
    raw.parse()
        .map_err(|e| Error::usage(format!("invalid session id `{raw}`: {e}")))
}

/// Removes `--name <value>` from `args` and returns the value.
///
/// Scans the whole argument list rather than only the front, so global
/// options like `--state-root` work wherever they are written.
fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    if index + 1 >= args.len() {
        return Err(Error::usage(format!("{name} requires a value")));
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn take_option_removes_both_the_flag_and_its_value() {
        let mut args = argv(&["new", "--state-root", "/tmp/x", "--kind", "terminal"]);
        assert_eq!(
            take_option(&mut args, "--state-root").expect("parse"),
            Some("/tmp/x".to_owned())
        );
        assert_eq!(args, argv(&["new", "--kind", "terminal"]));
    }

    #[test]
    fn take_option_finds_a_flag_written_anywhere() {
        let mut args = argv(&["list", "--state-root", "/tmp/x"]);
        assert_eq!(
            take_option(&mut args, "--state-root").expect("parse"),
            Some("/tmp/x".to_owned())
        );
    }

    #[test]
    fn a_flag_with_no_value_is_a_usage_error_not_a_silent_none() {
        let mut args = argv(&["list", "--state-root"]);
        assert!(take_option(&mut args, "--state-root").is_err());
    }

    #[test]
    fn a_missing_flag_is_none() {
        let mut args = argv(&["list"]);
        assert_eq!(take_option(&mut args, "--state-root").expect("parse"), None);
        assert_eq!(args, argv(&["list"]));
    }

    #[test]
    fn an_invalid_session_id_is_rejected_before_it_reaches_the_filesystem() {
        // Ids become path components; `../../etc` must fail at the
        // command line, not somewhere deeper.
        assert!(parse_id(Some(&"../../etc".to_owned())).is_err());
        assert!(parse_id(None).is_err());
    }
}
