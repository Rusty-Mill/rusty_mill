//! `bbp`: host commands for the Blackboard Protocol.
//!
//! ```text
//! bbp mcp                                   per-turn MCP server; env BBP_DIR, BBP_TASK, BBP_PRINCIPAL, BBP_TURN
//! bbp open    --dir D --task T --repo R --brief FILE [--human ID]
//! bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
//! bbp card    --dir D --task T
//! bbp tick    --dir D --task T
//! bbp human   --dir D --task T VERB ARGS...  approve-plan | approve-merge | reject | decision | ask | answer | rerun | receipt | resume | extend | cancel
//! bbp runner  --dir D --task T --repo-path PATH --work DIR [--confine sandbox|none]
//! bbp mod     --dir D --task T --repo-path PATH --work DIR --agents FILE [--confine sandbox|none] [--poll-ms N] [--max-wall-secs N]
//! bbp __sandbox ...                         the sandbox helper rusty_sandbox re-invokes; not for hands
//! ```

use rusty_bbp::*;
use rusty_bbp_host::{admin, args::Args, human, mcp, moderator, profiles, response_json, runner};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

fn run() -> Result<String, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let sub = argv
        .first()
        .cloned()
        .ok_or("usage: bbp <mcp|open|assign|card|tick|human|runner|mod> ...")?;
    if sub == "__sandbox" {
        let rest: Vec<OsString> = std::env::args_os().skip(2).collect();
        let err = rusty_sandbox::run_helper(&rest);
        eprintln!("bbp __sandbox: {err}");
        std::process::exit(i32::from(rusty_sandbox::SETUP_FAILED));
    }
    let a = Args::parse(argv.into_iter().skip(1));
    if sub == "mcp" {
        let binding = mcp::Binding::from_env()?;
        return mcp::Server::new(binding)?
            .serve()
            .map(|_| String::new())
            .map_err(|e| e.to_string());
    }
    let dir = PathBuf::from(a.flag_or_env("dir", "BBP_DIR")?);
    let task = TaskId(a.flag_or_env("task", "BBP_TASK")?);
    let resp = match sub.as_str() {
        "open" => {
            let brief = std::fs::read(a.flag("brief")?).map_err(|e| e.to_string())?;
            let human = PrincipalId(
                a.flags
                    .get("human")
                    .cloned()
                    .unwrap_or_else(|| "human".into()),
            );
            let set = match a.flags.get("profiles") {
                Some(file) => profiles::read_file(&PathBuf::from(file))?,
                None => profiles::ProfileSet::shell("test", "cargo test"),
            };
            admin::open_task(&dir, &task, a.flag("repo")?, &brief, &human, &set)?
        }
        "mod" => {
            let confinement = match a.flags.get("confine").map(String::as_str) {
                Some("none") => runner::Confinement::Unconfined,
                None | Some("sandbox") => runner::Confinement::Sandboxed,
                Some(other) => return Err(format!("unknown confinement {other}")),
            };
            let num = |k: &str, d: u64| -> Result<u64, String> {
                match a.flags.get(k) {
                    Some(v) => v.parse().map_err(|_| format!("--{k} must be a number")),
                    None => Ok(d),
                }
            };
            let cfg = moderator::Config {
                dir: dir.clone(),
                task: task.clone(),
                bbp: std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?,
                repo: PathBuf::from(a.flag("repo-path")?),
                work: PathBuf::from(a.flag("work")?),
                launchers: moderator::Launchers::read_file(&PathBuf::from(a.flag("agents")?))?,
                confinement,
                poll: std::time::Duration::from_millis(num("poll-ms", 500)?),
                max_wall: std::time::Duration::from_secs(num("max-wall-secs", u64::MAX / 1000)?),
            };
            let out = moderator::run(&cfg)?;
            return Ok(format!(
                "{{\"state\":\"{:?}\",\"turns_launched\":{},\"runs\":{}}}",
                out.state, out.turns_launched, out.runs
            ));
        }
        "runner" => {
            let repo = PathBuf::from(a.flag("repo-path")?);
            let work = PathBuf::from(a.flag("work")?);
            match a.flags.get("confine").map(String::as_str) {
                Some("none") => runner::run_once(
                    &dir,
                    &task,
                    &repo,
                    &work,
                    &runner::Unconfined,
                    runner::Confinement::Unconfined,
                )?,
                None | Some("sandbox") => {
                    let exec = runner::sandboxed(&dir.join("sandbox-state"))?;
                    runner::run_once(
                        &dir,
                        &task,
                        &repo,
                        &work,
                        &exec,
                        runner::Confinement::Sandboxed,
                    )?
                }
                Some(other) => return Err(format!("unknown confinement {other}")),
            }
        }
        "assign" => admin::assign(
            &dir,
            &task,
            admin::parse_role(a.flag("role")?)?,
            &PrincipalId(a.flag("principal")?.to_owned()),
            a.flag("vendor")?,
        )?,
        "card" => {
            return rusty_serde::json::to_string(&admin::card(&dir, &task)?)
                .map_err(|e| e.to_string())
        }
        "tick" => admin::tick(&dir, &task)?,
        "human" => {
            let verb = a.pos(0)?;
            let rest: Vec<&str> = a.positional.iter().skip(1).map(String::as_str).collect();
            human::perform(&dir, &task, verb, &rest)?
        }
        other => return Err(format!("unknown subcommand {other}")),
    };
    Ok(response_json(&resp))
}

fn main() -> ExitCode {
    match run() {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bbp: {e}");
            ExitCode::FAILURE
        }
    }
}
