//! `bbp`: host commands for the Blackboard Protocol.
//!
//! ```text
//! bbp mcp                                   per-turn MCP server; env BBP_DIR, BBP_TASK, BBP_PRINCIPAL, BBP_TURN
//! bbp open    --dir D --task T --repo R --brief FILE --roster FILE [--human ID] [--profiles FILE] [--live-turns N] [--policy turn-based|event-driven]
//! bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
//! bbp card    --dir D --task T
//! bbp tick    --dir D --task T
//! bbp human   --dir D --task T [--rev N] [--as WHO] VERB ARGS...  approve-plan | approve-merge | reject | decision | ask | answer | rerun | receipt | resume | extend | cancel
//!                                          reject, rerun, resume and cancel need --rev, the card revision you saw
//! bbp runner  --dir D --task T --repo-path PATH --work DIR [--confine sandbox|none]
//! bbp metrics --dir D --task T                             concurrency metrics of the task's log, as JSON
//! bbp pairs   --manifest FILE                              the 2b gate criteria over a run's pairs, as JSON
//! bbp why     --dir D (--task T | T) [--turn N]            why each turn was granted (wake reason, or Explicit), replayed from the log, as JSON
//! bbp grade   pack --manifest M --rubrics D --seed S --out O | join --key K --scores S --rubrics D   blind rubric grading
//! bbp tokens  --dir D --task T [--criteria FILE] [--api-usage FILE]   model tokens per turn, from the harness output, as JSON; weighted by the criteria file; a mismatch with the API usage is an error
//! bbp shadow  --dir D --task T                             how often an event-driven rule would have picked another role, as JSON
//! bbp mod     --dir D --task T --repo-path PATH --work DIR --agents FILE [--confine sandbox|none] [--poll-ms N] [--max-wall-secs N]
//! bbp __sandbox ...                         the sandbox helper rusty_sandbox re-invokes; not for hands
//! ```

use rusty_bbp::*;
use rusty_bbp_host::{
    admin, args::Args, grade, human, mcp, moderator, pairs, profiles, report, response_json,
    runner, shadow, tokens, why,
};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

fn run() -> Result<String, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let sub = argv.first().cloned().ok_or(
        "usage: bbp <mcp|open|assign|card|tick|human|runner|metrics|tokens|pairs|grade|why|shadow|mod> ...",
    )?;
    if sub == "__sandbox" {
        let rest: Vec<OsString> = std::env::args_os().skip(2).collect();
        let err = rusty_sandbox::run_helper(&rest);
        eprintln!("bbp __sandbox: {err}");
        std::process::exit(i32::from(rusty_sandbox::SETUP_FAILED));
    }
    let a = Args::parse(argv.into_iter().skip(1));
    if sub == "grade" {
        let p = |k: &str| a.flag(k).map(PathBuf::from);
        return match a.pos(0)? {
            "pack" => {
                grade::write_pack(&p("manifest")?, &p("rubrics")?, a.flag("seed")?, &p("out")?)
            }
            "join" => grade::join_files(&p("key")?, &p("scores")?, &p("rubrics")?),
            other => Err(format!("unknown grade action {other} (pack|join)")),
        };
    }
    if sub == "pairs" {
        return pairs::json(&PathBuf::from(a.flag("manifest")?));
    }
    if sub == "mcp" {
        let binding = mcp::Binding::from_env()?;
        return mcp::Server::new(binding)?
            .serve()
            .map(|_| String::new())
            .map_err(|e| e.to_string());
    }
    let dir = PathBuf::from(a.flag_or_env("dir", "BBP_DIR")?);
    // `bbp why TASK` names the task positionally too.
    let task = match (sub.as_str(), a.positional.first()) {
        ("why", Some(t)) => TaskId(t.clone()),
        _ => TaskId(a.flag_or_env("task", "BBP_TASK")?),
    };
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
            let roster = admin::read_roster(&PathBuf::from(a.flag("roster")?))?;
            let live = match a.flags.get("live-turns") {
                Some(v) => v.parse().map_err(|_| "--live-turns must be a number")?,
                None => 1,
            };
            let policy = match a.flags.get("policy") {
                Some(v) => admin::parse_policy(v)?,
                None => SchedulingPolicy::default(),
            };
            admin::open_task_live(
                &dir,
                &task,
                a.flag("repo")?,
                &brief,
                &human,
                &set,
                roster,
                live,
                policy,
            )?
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
            let visits: Vec<String> = out
                .visits
                .iter()
                .map(|(s, d)| format!("[\"{s:?}\",{}]", d.as_millis()))
                .collect();
            return Ok(format!(
                "{{\"state\":\"{:?}\",\"turns_launched\":{},\"runs\":{},\"elapsed_ms\":{},\"state_ms\":[{}]}}",
                out.state,
                out.turns_launched,
                out.runs,
                out.elapsed.as_millis(),
                visits.join(",")
            ));
        }
        "metrics" => return report::json(&dir, &task, a.flags.get("owner").map(String::as_str)),
        "tokens" => {
            let path = |k: &str| a.flags.get(k).map(PathBuf::from);
            return tokens::json(
                &dir,
                &task,
                path("criteria").as_deref(),
                path("api-usage").as_deref(),
            );
        }
        "why" => {
            let turn = match a.flags.get("turn") {
                Some(v) => Some(v.parse().map_err(|_| "--turn must be a number")?),
                None => None,
            };
            return why::json(&dir, &task, turn);
        }
        "shadow" => return shadow::json(&dir, &task),
        "runner" => {
            let repo = PathBuf::from(a.flag("repo-path")?);
            let work = PathBuf::from(a.flag("work")?);
            if a.flags.get("supervised").is_some_and(|v| v == "yes") {
                let confinement = match a.flags.get("confine").map(String::as_str) {
                    Some("none") => runner::Confinement::Unconfined,
                    None | Some("sandbox") => runner::Confinement::Sandboxed,
                    Some(other) => return Err(format!("unknown confinement {other}")),
                };
                return rusty_bbp_host::run_control::supervised(
                    &dir,
                    &task,
                    &repo,
                    &work,
                    confinement,
                )
                .map(|response| response_json(&response));
            }
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
            &RoleId::from(a.flag("role")?),
            &PrincipalId(a.flag("principal")?.to_owned()),
            a.flag("vendor")?,
        )?,
        "show" => {
            return rusty_serde::json::to_string(&admin::show(&dir, &task)?)
                .map_err(|e| e.to_string())
        }
        "card" => {
            return rusty_serde::json::to_string(&admin::card_view(&dir, &task)?)
                .map_err(|e| e.to_string())
        }
        "tick" => admin::tick(&dir, &task)?,
        "human" => {
            let verb = a.pos(0)?;
            let rest: Vec<&str> = a.positional.iter().skip(1).map(String::as_str).collect();
            let rev = a
                .flags
                .get("rev")
                .map(|r| {
                    r.parse()
                        .map(Rev)
                        .map_err(|_| "--rev must be a number".to_owned())
                })
                .transpose()?;
            human::perform_fenced(
                &dir,
                &task,
                verb,
                &rest,
                rev,
                a.flags.get("as").map_or(human::UNLABELLED, String::as_str),
                a.flags
                    .get("fence")
                    .map(|r| {
                        r.parse()
                            .map(Rev)
                            .map_err(|_| "--fence must be a revision".to_owned())
                    })
                    .transpose()?,
                a.flags.get("op").map(|s| OpId(s.clone())),
            )?
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
