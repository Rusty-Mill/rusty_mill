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
pub mod hooks;
pub mod paths;
pub mod supervisor;
pub mod transport;
pub mod worker;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sessionmgr_core::{Disposition, SessionId, SessionKind};

use crate::error::{Error, Result};

pub const USAGE: &str = "\
sessionmgr - manage AI coding-agent CLI sessions that survive this app closing

USAGE:
    sessionmgr <COMMAND>

COMMANDS:
    new [--kind KIND] [--agent AGENT] [--hooks] [--repo <path>] [--no-pty] [-- <command>...]
                                              create a session and start it
    new --parent <id> [--start-now] [--agent AGENT] [--no-pty] [-- <command>...]
                                              create a session dependent on another
                                              session's worktree (see DEPENDENT SESSIONS)
    list                                      list every session
    attach <id>                               stream a session's output
    close <id> [--merge|--discard]            tear a session down
    rename <id> <name> | rename <id> --clear  set or clear a display label
    start-now <id>                            stop a waiting dependent session from
                                              waiting any longer and start it now
    fork <id> [--no-pty]                      clone a session's own agent-CLI
                                              conversation into a new, independent
                                              session (see FORK)
    switch-agent <id> <agent> [--no-pty]      hand <id>'s live agent conversation
                                              off to a new session running a
                                              different agent (see SWITCH AGENT)
    tui                                       grid of session panes (starts a daemon if needed)
    daemon run                                run the supervisor in the foreground
    daemon start                              start the supervisor detached
    daemon status                             is a supervisor running?
    daemon shutdown                           stop the supervisor (sessions keep running)

SESSION KINDS:
    worktree     isolated in its own git worktree and branch (needs a repo)
    same-dir     runs in the repository's own working copy -- NOT isolated;
                 concurrent same-dir sessions can collide with each other
    terminal     a plain shell, no repository (the default)
    dependent    shares a --parent session's worktree; set implicitly by
                 --parent, never chosen directly with --kind

DEPENDENT SESSIONS:
    --parent <id> creates a session in <id>'s own worktree instead of
    creating one of its own -- CAPABILITIES.md's dependent sessions (an
    --agent given: a chained task, waiting for the parent to finish by
    default) and dependent terminal sessions (no --agent: a plain shell
    alongside a still-running agent, for manual work in the same
    workspace). Requires the parent to be --kind worktree (or itself
    dependent on one) and does not accept --kind/--repo/--hooks, which a
    dependent session inherits from its parent instead.

    By default a dependent session is created --status Waiting and does
    not start until the parent session finishes (its process exits, or it
    is closed with no disposition, which leaves the worktree in place --
    --merge/--discard remove it, which fails the dependent session
    instead of starting it into a workspace that no longer exists).
    --start-now skips the wait and starts immediately. `start-now <id>`
    does the same thing later, to a session already Waiting.

FORK:
    fork <id> clones <id>'s own agent-CLI conversation into a brand-new,
    independent session -- CAPABILITIES.md's Fork session capability. The
    new session gets its own worktree, branched from <id>'s own branch tip
    (not the repository's default branch), so its code state matches the
    conversation it starts with; forking never touches <id> itself.

    Requires <id> to be a --kind worktree session with an --agent whose
    adapter supports it -- as of this release, only claude. Codex and
    Gemini CLI have real fork mechanisms of their own but need machinery
    this release does not yet build (see docs/phase-6-report.md for
    exactly what and why); forking a Codex or Gemini session fails with a
    clear error naming the gap, not a silent no-op or a guess.

SWITCH AGENT:
    switch-agent <id> <agent> stops <id>'s live agent conversation and
    starts a brand-new session running <agent> in its place, in <id>'s own
    workspace -- CAPABILITIES.md's Switch agent mid-session capability.
    Unlike fork, this is one line of work continuing, not two: no new
    worktree is created, and <id> itself stops running once switched.

    The new session's initial prompt is a rendered copy of <id>'s own
    transcript, so <agent> can read what already happened -- not a native,
    per-CLI resume. Every agent this tool supports can receive a handoff
    this way; see docs/phase-7-report.md for why a native translation
    between three different on-disk transcript formats was not built.

    Requires <id> to have a live agent conversation (status running or
    needs-input) and a different --agent than it already runs.

AGENTS:
    claude       Claude Code -- tier-3 pattern matching, plus a verified
                 hook mechanism (--hooks wires it to this tool's status)
    codex        Codex -- tier-3 pattern matching, plus a verified hook
                 mechanism (--hooks wires it to this tool's status)
    gemini       Gemini CLI -- tier-3 pattern matching and a hook config
                 (--hooks), both built from gemini-cli's own source and
                 docs rather than a live-verified session (no credentials
                 on this machine); lower confidence than claude/codex
    Without --agent, a session gets none of the above: `command` runs
    literally and only process-exit status is ever reported.

HOOKS:
    --hooks installs the chosen --agent's own hook config into the
    session's worktree, calling back into this tool for a higher-
    confidence status signal than pattern matching alone. Needs both
    --agent and --kind worktree. Set SESSIONMGR_WEBHOOK_URL to also get
    an outbound POST on needs-input/finished/errored/subagent-finished
    -- minimal payload, no transcript, no absolute paths.

CLOSING:
    close <id>              stop the processes; leave any worktree in place
    close <id> --merge      also merge the session's branch back
                            (fast-forward only; fails loudly if diverged)
    close <id> --discard    also delete the session's worktree and branch

TERMINALS:
    Sessions run on a real terminal by default. Interactive agent CLIs
    refuse to start without one. --no-pty runs the process on plain pipes
    instead, which suits non-interactive commands.

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
        "close" => cmd_close(&root, rest).await,
        "rename" => cmd_rename(&root, rest).await,
        "start-now" => cmd_start_now(&root, rest).await,
        "fork" => cmd_fork(&root, rest).await,
        "switch-agent" => cmd_switch_agent(&root, rest).await,
        "tui" => {
            client::ensure_daemon(&root).await?;
            sessionmgr_tui::run(paths::daemon_socket(&root))
                .await
                .map_err(|e| Error::conflict(e.to_string()))
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
        "__hook-fire" => {
            let mut rest = rest.to_vec();
            let session_id = take_option(&mut rest, "--session-id")?
                .ok_or_else(|| Error::usage("__hook-fire requires --session-id"))?;
            let event = take_option(&mut rest, "--event")?
                .ok_or_else(|| Error::usage("__hook-fire requires --event"))?;
            cmd_hook_fire(&root, session_id, event).await
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

/// `__hook-fire`'s own handler.
///
/// **Fast, silent no-op on anything that does not look like one of this
/// tool's own sessions** (PLAN.md's own explicit requirement): a hook
/// this tool installs only ever fires for a session it created, but a
/// copied hook config, a stray file, or a future id-format change must
/// never block or auto-start a daemon with nothing to report to. That
/// check happens *before* touching the daemon at all -- reading the
/// catalog directly off disk, exactly the same way a recognized id
/// then still requires no daemon interaction to establish.
///
/// A real, recognized session **does** get the daemon's auto-start
/// sugar: the whole point is reporting to something. Bounded to a few
/// seconds regardless, because the CLI that invoked this hook is
/// blocked on this process exiting -- most hook events are synchronous
/// by default -- and a hang here would hang the user's own agent CLI.
async fn cmd_hook_fire(root: &Path, session_id: String, event: String) -> Result<()> {
    let Ok(id) = session_id.parse::<SessionId>() else {
        return Ok(());
    };
    if catalog::read_session(root, &id).is_err() {
        return Ok(());
    }
    let outcome = rusty_tokio::time::timeout(Duration::from_secs(5), async {
        let mut conn = client::connect(root).await?;
        conn.request::<_, sessionmgr_protocol::Response>(&sessionmgr_protocol::Request::HookFire {
            session_id: id.to_string(),
            event,
        })
        .await
    })
    .await;
    if let Ok(Err(e)) = outcome {
        eprintln!("sessionmgr __hook-fire: {e}");
    }
    Ok(())
}

async fn cmd_new(root: &Path, args: &[String]) -> Result<()> {
    let mut args = args.to_vec();
    // `--parent` implies a dependent session and is mutually exclusive
    // with `--kind`/`--repo`: both would otherwise describe *how to
    // create a workspace*, which a dependent session does not do -- it
    // reuses its parent's, resolved daemon-side (`Request::SessionNew`'s
    // own docs).
    let parent = take_option(&mut args, "--parent")?
        .map(|raw| raw.parse::<SessionId>())
        .transpose()
        .map_err(|e| Error::usage(format!("invalid --parent session id: {e}")))?;
    let explicit_kind = take_option(&mut args, "--kind")?;
    // `--start-now` is `Request::SessionNew`'s `wait_for_parent: false`
    // -- meaningless without `--parent`, so it is rejected on its own
    // rather than silently ignored.
    let start_now = args.iter().any(|a| a == "--start-now");
    args.retain(|a| a != "--start-now");

    let kind = if parent.is_some() {
        if explicit_kind.is_some() {
            return Err(Error::usage(
                "--kind is implied by --parent (always `dependent`); do not pass both",
            ));
        }
        SessionKind::Dependent
    } else {
        if start_now {
            return Err(Error::usage("--start-now needs --parent"));
        }
        match explicit_kind.as_deref() {
            None | Some("terminal") => SessionKind::PlainTerminal,
            Some("worktree") => SessionKind::Worktree,
            Some("same-dir") | Some("same-directory") => SessionKind::SameDirectory,
            Some(other) => {
                return Err(Error::usage(format!(
                "unknown session kind `{other}` (expected `worktree`, `same-dir`, or `terminal`)"
            )))
            }
        }
    };
    // Defaults to the client's own working directory, since that is the
    // one process standing where the user is. The daemon resolves it to a
    // repository root; it is not resolved here, so that `--repo` and the
    // implicit case go through exactly the same code path. A dependent
    // session's `needs_repo()` is `false`, so it never hits the implicit
    // branch; an *explicit* `--repo` alongside `--parent` is caught
    // below instead of being silently ignored.
    let explicit_repo = take_option(&mut args, "--repo")?;
    if parent.is_some() && explicit_repo.is_some() {
        return Err(Error::usage(
            "--repo is meaningless for a dependent session; its workspace comes from \
             --parent's own worktree",
        ));
    }
    let repo = match explicit_repo {
        Some(path) => Some(PathBuf::from(path)),
        None if kind.needs_repo() => Some(
            std::env::current_dir()
                .map_err(|e| Error::io("locating the current directory", None, e))?,
        ),
        None => None,
    };
    // Everything after `--` is the command to run.
    let command = match args.iter().position(|arg| arg == "--") {
        Some(index) => args[index + 1..].to_vec(),
        None => Vec::new(),
    };
    // A terminal unless explicitly refused. Interactive agent CLIs will
    // not start without one; `--no-pty` selects the piped backend, whose
    // survives-a-kill behaviour is the one proven on Windows.
    let pty = !args.iter().any(|a| a == "--no-pty");
    args.retain(|a| a != "--no-pty");
    // `--agent` resolves `command` through that agent's own
    // `launch_args` instead of treating it as the literal program to
    // run, and turns on tier-3 `needs_input` detection for the session.
    let agent = take_option(&mut args, "--agent")?
        .as_deref()
        .map(parse_agent_name)
        .transpose()?;
    // `--hooks` installs `agent`'s own hook config into the session's
    // worktree -- opt-in (see Request::SessionNew's own docs for why),
    // and meaningless without an agent to install hooks for. A dependent
    // session inherits whatever hook config its parent already installed
    // into the shared worktree, so `--hooks` is rejected for one rather
    // than silently installing a second, conflicting copy.
    let hooks = args.iter().any(|a| a == "--hooks");
    args.retain(|a| a != "--hooks");
    if hooks && agent.is_none() {
        return Err(Error::usage("--hooks needs --agent <claude|codex>"));
    }
    if hooks && kind == SessionKind::Dependent {
        return Err(Error::usage(
            "--hooks is inherited from the parent's own worktree for a dependent session; \
             pass --hooks on the parent session instead",
        ));
    }
    let wait_for_parent = parent.is_some() && !start_now;
    let id = client::session_new(
        root,
        kind,
        command,
        repo,
        pty,
        agent,
        hooks,
        parent,
        wait_for_parent,
    )
    .await?;
    println!("{id}");
    Ok(())
}

async fn cmd_start_now(root: &Path, args: &[String]) -> Result<()> {
    let id = parse_id(args.first())?;
    client::session_start_now(root, id).await?;
    println!("started");
    Ok(())
}

async fn cmd_fork(root: &Path, args: &[String]) -> Result<()> {
    let mut args = args.to_vec();
    let pty = !args.iter().any(|a| a == "--no-pty");
    args.retain(|a| a != "--no-pty");
    let id = parse_id(args.first())?;
    let forked = client::session_fork(root, id, pty).await?;
    println!("{forked}");
    Ok(())
}

async fn cmd_switch_agent(root: &Path, args: &[String]) -> Result<()> {
    let mut args = args.to_vec();
    let pty = !args.iter().any(|a| a == "--no-pty");
    args.retain(|a| a != "--no-pty");
    let id = parse_id(args.first())?;
    let agent = match args.get(1) {
        Some(name) => parse_agent_name(name)?,
        None => return Err(Error::usage("switch-agent requires <id> <agent>")),
    };
    let switched = client::session_switch_agent(root, id, agent, pty).await?;
    println!("{switched}");
    Ok(())
}

fn parse_agent_name(name: &str) -> Result<sessionmgr_core::AgentKind> {
    match name {
        "claude" | "claude-code" => Ok(sessionmgr_core::AgentKind::ClaudeCode),
        "codex" => Ok(sessionmgr_core::AgentKind::Codex),
        "gemini" => Ok(sessionmgr_core::AgentKind::Gemini),
        other => Err(Error::usage(format!(
            "unknown agent `{other}` (expected `claude`, `codex`, or `gemini`)"
        ))),
    }
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

async fn cmd_close(root: &Path, args: &[String]) -> Result<()> {
    let merge = args.iter().any(|a| a == "--merge");
    let discard = args.iter().any(|a| a == "--discard");
    // Refused rather than resolved by precedence: the two mean opposite
    // things about the user's work, and guessing which one was meant is
    // exactly the wrong thing to do when one of them is irreversible.
    let disposition = match (merge, discard) {
        (true, true) => {
            return Err(Error::usage(
                "--merge and --discard are opposites; pass at most one",
            ))
        }
        (true, false) => Some(Disposition::Merge),
        (false, true) => Some(Disposition::Discard),
        (false, false) => None,
    };
    let id = parse_id(args.iter().find(|a| !a.starts_with("--")))?;
    client::session_close(root, id, disposition).await?;
    println!(
        "{}",
        match disposition {
            Some(Disposition::Merge) => "closed and merged",
            Some(Disposition::Discard) => "closed and discarded",
            None => "closed",
        }
    );
    Ok(())
}

async fn cmd_rename(root: &Path, args: &[String]) -> Result<()> {
    let id = parse_id(args.first())?;
    let name = match args.get(1).map(String::as_str) {
        None => {
            return Err(Error::usage(
                "usage: rename <id> <name> | rename <id> --clear",
            ))
        }
        Some("--clear") => None,
        Some(name) => Some(name.to_owned()),
    };
    client::session_rename(root, id, name.clone()).await?;
    match name {
        Some(name) => println!("renamed to \"{name}\""),
        None => println!("name cleared"),
    }
    Ok(())
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
