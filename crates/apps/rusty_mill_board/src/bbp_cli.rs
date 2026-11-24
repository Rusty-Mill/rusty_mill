//! The [`Escalations`] adapter over the orch host's `bbp` command (#595).
//!
//! A process boundary, not a crate dependency (ADR-0003): the host stays the
//! only writer of the BBP log, behind its lock and revision fence. Mill Board
//! never reads BBP files.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::inbox::{
    Action, DryRun, Escalation, Escalations, InboxError, Outcome, Response, Step, plan,
    step_decision,
};

/// Runs `<bin> ... --dir <root>` for every read and write.
#[derive(Debug, Clone)]
pub struct BbpCli {
    bin: PathBuf,
    root: PathBuf,
}

impl BbpCli {
    /// The host's `bbp` binary and the BBP task directory it serves.
    #[must_use]
    pub const fn new(bin: PathBuf, root: PathBuf) -> Self {
        Self { bin, root }
    }

    /// Run `bbp <args>` and parse what it prints on success.
    fn run(&self, args: &[String]) -> Result<Value, InboxError> {
        let out = Command::new(&self.bin)
            .args(args)
            .output()
            .map_err(|e| InboxError::Host(format!("{}: {e}", self.bin.display())))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(InboxError::Host(err.trim().to_owned()));
        }
        serde_json::from_slice(&out.stdout)
            .map_err(|e| InboxError::Host(format!("unreadable output: {e}")))
    }

    /// `bbp human` for `step`, fenced on `fence`; a dry run if `dry`; sent as `decision`
    /// if the caller named one.
    fn human(
        &self,
        e: &Escalation,
        step: &Step,
        fence: u64,
        by: &str,
        decision: Option<&str>,
        dry: bool,
    ) -> Vec<String> {
        let fence = fence.to_string();
        let mut argv: Vec<String> = [
            "human",
            "--dir",
            &self.root_str(),
            "--task",
            &e.task,
            "--rev",
            &fence,
            "--fence",
            &fence,
            "--as",
            by,
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        if let Some(id) = decision {
            argv.extend(["--decision".to_owned(), id.to_owned()]);
        }
        if dry {
            argv.extend(["--dry-run".to_owned(), "yes".to_owned()]);
        }
        argv.push(step.verb.to_owned());
        argv.extend(step.args.iter().cloned());
        argv
    }

    fn root_str(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    /// The host's preview of `step` at `fence`: its response and events.
    fn dry(
        &self,
        e: &Escalation,
        step: &Step,
        fence: u64,
        by: &str,
        decision: Option<&str>,
    ) -> Result<(Response, Vec<Value>), InboxError> {
        let v = self.run(&self.human(e, step, fence, by, decision, true))?;
        let events = v
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| InboxError::Host("dry run printed no events".into()))?;
        let response = v
            .get("response")
            .cloned()
            .ok_or_else(|| InboxError::Host("dry run printed no response".into()))?;
        Ok((Response::from_json(response), events))
    }
}

/// The answerer label a preview carries; it never reaches the log.
const PREVIEW_AS: &str = "inbox";

fn valid_task(t: &str) -> bool {
    !t.is_empty() && !t.contains(',') && !t.starts_with("--")
}

impl Escalations for BbpCli {
    fn open(&self, tasks: &[String]) -> Result<Vec<Escalation>, InboxError> {
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(bad) = tasks.iter().find(|t| !valid_task(t)) {
            return Err(InboxError::Invalid(format!("bad BBP task id {bad:?}")));
        }
        let argv = [
            "inbox".to_owned(),
            "--dir".to_owned(),
            self.root_str(),
            "--task".to_owned(),
            tasks.join(","),
            "--json".to_owned(),
            "yes".to_owned(),
        ];
        serde_json::from_value(self.run(&argv)?)
            .map_err(|e| InboxError::Host(format!("unreadable inbox: {e}")))
    }

    fn preview(&self, e: &Escalation, a: &Action) -> Result<DryRun, InboxError> {
        let mut steps = plan(e, a)?.into_iter();
        let step = steps
            .next()
            .ok_or_else(|| InboxError::Invalid("the action has no command".into()))?;
        let (response, events) = self.dry(e, &step, e.rev, PREVIEW_AS, None)?;
        Ok(DryRun {
            step,
            response,
            events,
            then: steps.collect(),
        })
    }

    /// Each command is previewed, then committed at the same fence. A
    /// commit appends exactly the previewed events (BBP's pure engine), so
    /// the next command is fenced on `fence + events`: anyone else writing in
    /// between makes it `StaleRev`, and the outcome says how far it got.
    fn act(
        &self,
        e: &Escalation,
        a: &Action,
        by: &str,
        decision: Option<&str>,
    ) -> Result<Outcome, InboxError> {
        let steps = plan(e, a)?;
        // A repeat of a send replays its commands without events, which would leave a
        // later command fenced on the wrong rev: ids ride single-command actions only.
        if decision.is_some() && steps.len() != 1 {
            return Err(InboxError::Invalid(
                "a decision id needs a single-command action".into(),
            ));
        }
        let mut fence = e.rev;
        for (n, step) in steps.iter().enumerate() {
            let id = decision.map(|d| step_decision(d, n));
            let id = id.as_deref();
            let (response, events) = self.dry(e, step, fence, by, id)?;
            if let Response::Rejected { code, detail } = response {
                return Ok(Outcome::Refused {
                    step: n,
                    code,
                    detail,
                });
            }
            let committed =
                Response::from_json(self.run(&self.human(e, step, fence, by, id, false))?);
            if let Response::Rejected { code, detail } = committed {
                return Ok(Outcome::Refused {
                    step: n,
                    code,
                    detail,
                });
            }
            fence += events.len() as u64;
        }
        Ok(Outcome::Done)
    }

    fn applied(&self, task: &str, decision: &str) -> Result<Option<u64>, InboxError> {
        if !valid_task(task) {
            return Err(InboxError::Invalid(format!("bad BBP task id {task:?}")));
        }
        let argv = [
            "decision".to_owned(),
            "--dir".to_owned(),
            self.root_str(),
            "--task".to_owned(),
            task.to_owned(),
            "--decision".to_owned(),
            step_decision(decision, 0),
        ];
        let rows: Vec<Applied> = serde_json::from_value(self.run(&argv)?)
            .map_err(|e| InboxError::Host(format!("unreadable decision: {e}")))?;
        // Absence is only evidence if the host answered for exactly this id and its row
        // is consistent: anything else is an error, so nothing is settled from it.
        match rows.as_slice() {
            [row] if row.decision == argv[6] && row.applied == row.rev.is_some() => Ok(row.rev),
            _ => Err(InboxError::Host(format!(
                "decision lookup did not answer for {}",
                argv[6]
            ))),
        }
    }
}

/// One row of `bbp decision`.
#[derive(serde::Deserialize)]
struct Applied {
    decision: String,
    applied: bool,
    rev: Option<u64>,
}
