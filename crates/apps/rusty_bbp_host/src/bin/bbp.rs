//! `bbp`: host commands for the Blackboard Protocol.
//!
//! ```text
//! bbp mcp                                   per-turn MCP server; env BBP_DIR, BBP_TASK, BBP_PRINCIPAL, BBP_TURN
//! bbp open    --dir D --task T --repo R --brief FILE [--human ID]
//! bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
//! bbp card    --dir D --task T
//! bbp tick    --dir D --task T
//! bbp human   --dir D --task T [--rev N] VERB ARGS...  approve-plan | approve-merge | reject | decision | ask | answer | rerun | receipt | resume | extend | cancel
//!                                          reject, rerun, resume and cancel need --rev, the card revision you saw
//! ```

use rusty_bbp::*;
use rusty_bbp_host::{admin, args::Args, human, mcp, response_json};
use std::path::PathBuf;
use std::process::ExitCode;

fn run() -> Result<String, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let sub = argv
        .first()
        .cloned()
        .ok_or("usage: bbp <mcp|open|assign|card|tick|human> ...")?;
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
            admin::open_task(&dir, &task, a.flag("repo")?, &brief, &human)?
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
            let rev = a
                .flags
                .get("rev")
                .map(|r| {
                    r.parse()
                        .map(Rev)
                        .map_err(|_| "--rev must be a number".to_owned())
                })
                .transpose()?;
            human::perform(&dir, &task, verb, &rest, rev)?
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
