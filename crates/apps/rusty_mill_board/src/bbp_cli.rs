//! The [`Escalations`] adapter over the orch host's `bbp` command (#595).
//!
//! A process boundary, not a crate dependency (ADR-0003): the host stays the
//! only writer of the BBP log, behind its lock and revision fence. Mill Board
//! never reads BBP files.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::anchor::Verified;
use crate::inbox::{
    Action, DryRun, Escalation, Escalations, InboxError, Outcome, Response, Step, plan,
    step_decision,
};

/// Runs `<bin> ... --dir <root>` for every read and write.
#[derive(Debug, Clone)]
pub struct BbpCli {
    bin: PathBuf,
    root: PathBuf,
    verify_deadline: Duration,
}

impl BbpCli {
    /// The host's `bbp` binary and the BBP task directory it serves.
    #[must_use]
    pub const fn new(bin: PathBuf, root: PathBuf) -> Self {
        Self {
            bin,
            root,
            verify_deadline: VERIFY_DEADLINE,
        }
    }

    /// How long one `bbp verify` may run before it is killed and logged as a host
    /// error, so a stuck verifier never stalls the anchor pass. Default: [`VERIFY_DEADLINE`].
    #[must_use]
    pub const fn with_verify_deadline(mut self, deadline: Duration) -> Self {
        self.verify_deadline = deadline;
        self
    }

    /// Run `bbp <args>` with a deadline and a stdout ceiling of `max` bytes: its exit
    /// status, stdout and (bounded) stderr. Past the deadline the child is killed and
    /// this is [`InboxError::Host`]. The pipe readers are not joined then: a grandchild
    /// (the guard's `sudo` target) may still hold the pipes, and must not block the caller.
    fn run_deadline(
        &self,
        args: &[String],
        max: u64,
        deadline: Duration,
    ) -> Result<(ExitStatus, Vec<u8>, String), InboxError> {
        let host = |e: std::io::Error| InboxError::Host(format!("{}: {e}", self.bin.display()));
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(host)?;
        let out = child.stdout.take();
        let err = child.stderr.take();
        let out_reader = std::thread::spawn(move || {
            let mut body = Vec::new();
            if let Some(o) = out {
                let _ = o.take(max + 1).read_to_end(&mut body);
            }
            body
        });
        let err_reader = std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(e) = err {
                let _ = e.take(MAX_ERR).read_to_string(&mut text);
            }
            text
        });
        let until = Instant::now() + deadline;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(host)? {
                break status;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                return Err(InboxError::Host(format!(
                    "`bbp {}` timed out after {}s; killed",
                    args.first().map_or("", String::as_str),
                    deadline.as_secs_f32()
                )));
            }
            std::thread::sleep(POLL);
        };
        let body = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();
        if body.len() as u64 > max {
            return Err(InboxError::Host(format!(
                "the host's answer is over {max} bytes; refused"
            )));
        }
        Ok((status, body, stderr))
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

    /// As [`BbpCli::run`], but stdout is read through a byte ceiling: a host that
    /// prints more than `max` bytes is killed and refused, never buffered whole.
    fn run_bounded(&self, args: &[String], max: u64) -> Result<Value, InboxError> {
        let host = |e: std::io::Error| InboxError::Host(format!("{}: {e}", self.bin.display()));
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(host)?;
        let mut err = child.stderr.take();
        // stderr drains on its own thread so a chatty host cannot block on a full pipe.
        let err_reader = std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.take(MAX_ERR).read_to_string(&mut text);
            }
            text
        });
        let mut body = Vec::new();
        if let Some(out) = child.stdout.take() {
            out.take(max + 1).read_to_end(&mut body).map_err(host)?;
        }
        if body.len() as u64 > max {
            let _ = child.kill();
            let _ = child.wait();
            return Err(InboxError::Host(format!(
                "the host's answer is over {max} bytes; refused rather than buffered"
            )));
        }
        let status = child.wait().map_err(host)?;
        let stderr = err_reader.join().unwrap_or_default();
        if !status.success() {
            return Err(InboxError::Host(stderr.trim().to_owned()));
        }
        serde_json::from_slice(&body)
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

/// The most `bbp claims` may print: 8 MiB. A task's claims and their evidence
/// are bounded per message but not in total, so the board bounds the read.
const MAX_CLAIMS: u64 = 8 << 20;

/// The most stderr kept from a bounded run.
const MAX_ERR: u64 = 16 << 10;

/// The most `bbp verify` may print: its answer is one short JSON object.
const MAX_VERIFY: u64 = 64 << 10;

/// The default deadline for one `bbp verify`. Verify reads one task's log
/// once; minutes means the host is stuck, not busy.
pub const VERIFY_DEADLINE: Duration = Duration::from_secs(120);

/// How often a deadline-bound run checks whether its child has exited.
const POLL: Duration = Duration::from_millis(20);

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

    fn claims(&self, task: &str) -> Result<Value, InboxError> {
        if !valid_task(task) {
            return Err(InboxError::Invalid(format!("bad BBP task id {task:?}")));
        }
        let argv = [
            "claims".to_owned(),
            "--dir".to_owned(),
            self.root_str(),
            "--task".to_owned(),
            task.to_owned(),
            "--json".to_owned(),
            "yes".to_owned(),
        ];
        let v = self.run_bounded(&argv, MAX_CLAIMS)?;
        if v.is_array() {
            Ok(v)
        } else {
            Err(InboxError::Host("`bbp claims` did not print a list".into()))
        }
    }

    /// `bbp` prints `bbp: <reason>` and exits 1 when it refuses; that is the answer, recorded
    /// by the caller. Anything else (no binary, the guard's exit 64, a signal, a run past
    /// the deadline) is [`InboxError::Host`], so a broken or stuck deploy never reads as a
    /// rewritten log.
    fn verify(
        &self,
        task: &str,
        expect: Option<&str>,
    ) -> Result<Result<Verified, String>, InboxError> {
        if !valid_task(task) || !crate::anchor::anchorable(task) {
            return Err(InboxError::Invalid(format!("bad BBP task id {task:?}")));
        }
        let mut argv = vec![
            "verify".to_owned(),
            "--dir".to_owned(),
            self.root_str(),
            "--task".to_owned(),
            task.to_owned(),
        ];
        if let Some(h) = expect {
            argv.extend(["--expect".to_owned(), h.to_owned()]);
        }
        let (status, stdout, stderr) =
            self.run_deadline(&argv, MAX_VERIFY, self.verify_deadline)?;
        if status.code() == Some(1)
            && let Some(reason) = stderr.trim().strip_prefix("bbp: ")
        {
            return Ok(Err(reason.to_owned()));
        }
        if !status.success() {
            return Err(InboxError::Host(stderr.trim().to_owned()));
        }
        let v: VerifyOut = serde_json::from_slice(&stdout)
            .map_err(|e| InboxError::Host(format!("unreadable verify: {e}")))?;
        Ok(Ok(Verified {
            lines: v.lines,
            unchained: v.unchained,
            head: v.head,
        }))
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

/// What `bbp verify` prints on success.
#[derive(serde::Deserialize)]
struct VerifyOut {
    lines: u64,
    unchained: u64,
    head: String,
}

/// One row of `bbp decision`.
#[derive(serde::Deserialize)]
struct Applied {
    decision: String,
    applied: bool,
    rev: Option<u64>,
}
