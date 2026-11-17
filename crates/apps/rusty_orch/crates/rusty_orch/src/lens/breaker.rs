//! `rusty_orch lens breaker` (#598, Let Em Loose #758 item 2, D137): a
//! read-only report on one task's cost breaker.
//!
//! The breaker's windows live only in a running `lens run` (598 PLAN S8),
//! and would-trips go to that run's log, so this reports what is kept: the
//! config's breaker, the task's meter account, its taint table by class,
//! and whether BBP holds the task paused, with the latest pause's reason
//! and detail. There is no reset: the human resumes a paused task in BBP,
//! and the next run opens a new window. Nothing is posted or saved.

use std::path::{Path, PathBuf};
use std::process::Command;

use orch_lens::breaker::{Breaker, Mode};
use orch_lens::meter::Meter;
use orch_lens::taint::Taints;
use orch_lens::{DataClass, LensConfig};
use orch_store::Store;
use rusty_serde::{Serialize, Value};

use crate::args::ArgsError;

pub const USAGE: &str = "usage: rusty_orch lens breaker <config.json> --bbp <bbp binary> --dir <bbp dir> --task <task>\n           --state <orch state dir> --experiment <name>\n\nPrint one JSON line on the task's cost breaker: the config's breaker, the task's\ncalls and spend on the experiment's meter, its taint table by class, and whether\nBBP holds it paused (with the latest pause's reason and detail). Read-only: a\nrunning `lens run` keeps its window in memory and logs would-trips itself, and\nholds the store's lock, so this fails until that run ends.";

/// What to report on.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub config: PathBuf,
    pub bbp: PathBuf,
    pub dir: PathBuf,
    pub task: String,
    pub state: PathBuf,
    pub experiment: String,
}

/// Parse `breaker <config> --bbp … --dir … --task … --state … --experiment …`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    if args.next().as_deref() != Some("breaker") {
        return Err(ArgsError::Help);
    }
    let mut config = None;
    let (mut bbp, mut dir, mut task, mut state, mut experiment) = (None, None, None, None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--bbp" => &mut bbp,
            "--dir" => &mut dir,
            "--task" => &mut task,
            "--state" => &mut state,
            "--experiment" => &mut experiment,
            _ if !arg.starts_with('-') && config.is_none() => {
                config = Some(PathBuf::from(arg));
                continue;
            }
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        };
        if slot.is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
        *slot = Some(value);
    }
    let required =
        |v: Option<String>, f: &str| v.ok_or_else(|| ArgsError::Usage(format!("{f} is required")));
    Ok(Options {
        config: config.ok_or_else(|| ArgsError::Usage("a lens config is required".into()))?,
        bbp: required(bbp, "--bbp")?.into(),
        dir: required(dir, "--dir")?.into(),
        task: required(task, "--task")?,
        state: required(state, "--state")?.into(),
        experiment: required(experiment, "--experiment")?,
    })
}

/// Read the config, the store (without changing it) and BBP's inbox, then
/// [`render`] the report.
pub fn run(o: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&o.config)
        .map_err(|e| format!("cannot read {}: {e}", o.config.display()))?;
    let config = LensConfig::parse(&text).map_err(|e| format!("{}: {e}", o.config.display()))?;
    let board = super::run::board(&o.dir, &o.task)?;
    let snapshot = Store::read_meter(&o.state, &o.experiment).map_err(|e| e.to_string())?;
    // Restored as `lens run` restores it: a reservation still held counts
    // as a call and as spent.
    let meter = snapshot.map(|s| Meter::restore(config.ceilings, &s));
    let taints = Store::read_taints(&o.state, &board).map_err(|e| e.to_string())?;
    let pause = paused(&inbox(&o.bbp, &o.dir, &o.task)?)?;
    let report = Report::of(&config.breaker, &o.task, meter.as_ref(), &taints, pause);
    render(&report)
}

/// The task's latest pause, as BBP's inbox shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pause {
    pub reason: String,
    pub detail: String,
}

/// Taint table entries by class.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ByClass {
    pub public: u32,
    pub internal: u32,
    pub sensitive: u32,
}

/// The report's one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub task: String,
    pub mode: &'static str,
    pub window_calls: u32,
    pub window_micro_usd: u64,
    pub min_score_move: u64,
    /// The task's calls and spend on the experiment's meter, as `lens run`
    /// would restore it: held reservations count.
    pub calls: u32,
    pub spent_micro_usd: u64,
    pub taints: ByClass,
    pub paused: bool,
    pub pause: Option<Pause>,
}

impl Report {
    /// The report for `task` from what was read. Pure.
    pub fn of(
        b: &Breaker,
        task: &str,
        meter: Option<&Meter>,
        taints: &Taints,
        pause: Option<Pause>,
    ) -> Self {
        let (spent, calls) = meter.map_or((0, 0), |m| (m.task_spent(task), m.calls(task)));
        let mut by = ByClass::default();
        for (_, class) in taints.entries() {
            match class {
                DataClass::Public => by.public += 1,
                DataClass::Internal => by.internal += 1,
                DataClass::Sensitive => by.sensitive += 1,
            }
        }
        Self {
            task: task.to_owned(),
            mode: match b.mode {
                Mode::Observe => "observe",
                Mode::Enforce => "enforce",
            },
            window_calls: b.window_calls,
            window_micro_usd: b.window_spend,
            min_score_move: b.min_score_move,
            calls,
            spent_micro_usd: spent,
            taints: by,
            paused: pause.is_some(),
            pause,
        }
    }
}

/// The report as one JSON line.
pub fn render(r: &Report) -> Result<String, String> {
    let json = rusty_serde::json::to_string(r).map_err(|e| e.to_string())?;
    Ok(format!("{json}\n"))
}

/// `bbp inbox --dir D --task T`'s JSON.
fn inbox(bbp: &Path, dir: &Path, task: &str) -> Result<String, String> {
    let out = Command::new(bbp)
        .arg("inbox")
        .arg("--dir")
        .arg(dir)
        .args(["--task", task])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bbp.display()))?;
    if !out.status.success() {
        return Err(format!(
            "bbp inbox: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("bbp inbox: {e}"))
}

/// The open `TaskPaused` escalation in an inbox listing, if any. BBP lists
/// at most one task-level escalation, the latest.
///
/// # Errors
///
/// A message when the listing isn't an array of escalations, or a
/// `TaskPaused` one has no pause.
pub fn paused(json: &str) -> Result<Option<Pause>, String> {
    let v: Value = rusty_serde::json::from_str(json).map_err(|e| format!("bbp inbox: {e}"))?;
    let items = v.as_seq().ok_or("bbp inbox: not a list")?;
    let Some(item) = items
        .iter()
        .find(|e| e.get("reason").and_then(Value::as_str) == Some("TaskPaused"))
    else {
        return Ok(None);
    };
    let pause = item
        .get("pause")
        .ok_or("bbp inbox: a TaskPaused item has no pause")?;
    let field = |k: &str| {
        pause
            .get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("bbp inbox: pause has no {k}"))
    };
    Ok(Some(Pause {
        reason: field("reason")?,
        detail: field("detail")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_lens::meter::Snapshot;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_need_every_flag_and_the_config() {
        let o = options(argv(
            "breaker c.json --bbp b --dir d --task t --state s --experiment e",
        ))
        .expect("parses");
        assert_eq!(o.config, PathBuf::from("c.json"));
        assert_eq!(o.experiment, "e");
        let err = |s: &str| match options(argv(s)) {
            Err(ArgsError::Usage(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(err("breaker c.json --bbp b --dir d --task t --state s").contains("--experiment"));
        assert!(
            err("breaker --bbp b --dir d --task t --state s --experiment e").contains("config")
        );
        assert!(err("breaker c.json --task t --task u").contains("duplicate"));
        assert!(err("breaker c.json d.json").contains("unexpected"));
    }

    #[test]
    fn the_report_reads_the_tasks_account_and_counts_taints() {
        let snap = Snapshot {
            experiment_spent: 9,
            tasks: vec![("other".into(), 5, 2), ("t".into(), 1_500_000, 12)],
            // Held at a crash: `lens run` restores it as a settled call.
            reserved: vec![("t".into(), 250_000)],
        };
        let ceilings = orch_lens::meter::Ceilings {
            experiment: 0,
            task: 0,
            task_calls: 0,
        };
        let meter = Meter::restore(ceilings, &snap);
        let taints = Taints::from_entries([
            (1, DataClass::Internal),
            (2, DataClass::Sensitive),
            (3, DataClass::Internal),
        ]);
        let r = Report::of(&Breaker::default(), "t", Some(&meter), &taints, None);
        assert_eq!((r.calls, r.spent_micro_usd), (13, 1_750_000));
        assert_eq!(
            r.taints,
            ByClass {
                public: 0,
                internal: 2,
                sensitive: 1
            }
        );
        assert_eq!(r.mode, "observe");
        assert!(!r.paused);
        let line = render(&r).expect("json");
        assert!(
            line.ends_with("}\n") && line.contains("\"window_calls\":40"),
            "{line}"
        );
    }

    #[test]
    fn a_task_with_no_meter_reads_zero() {
        let r = Report::of(&Breaker::default(), "t", None, &Taints::default(), None);
        assert_eq!((r.calls, r.spent_micro_usd), (0, 0));
    }

    /// The reason reads as BBP serializes it.
    #[test]
    fn a_pause_reason_reads_as_bbp_writes_it() {
        #[derive(Serialize)]
        struct P {
            reason: rusty_bbp::claims::PauseReason,
            detail: String,
        }
        #[derive(Serialize)]
        struct E {
            reason: &'static str,
            pause: P,
        }
        let json = rusty_serde::json::to_string(&vec![E {
            reason: "TaskPaused",
            pause: P {
                reason: rusty_bbp::claims::PauseReason::Breaker,
                detail: "d".into(),
            },
        }])
        .expect("json");
        let p = paused(&json).expect("reads").expect("paused");
        assert!(
            p.reason.contains("Breaker") || p.reason.contains("breaker"),
            "{json}"
        );
    }

    #[test]
    fn a_paused_task_is_read_from_the_inbox() {
        let json = r#"[{"reason":"Budget","pause":null},
            {"reason":"TaskPaused","pause":{"reason":"Breaker","detail":"cost breaker: 40 calls"}}]"#;
        assert_eq!(
            paused(json).expect("reads"),
            Some(Pause {
                reason: "Breaker".into(),
                detail: "cost breaker: 40 calls".into()
            })
        );
        assert_eq!(paused("[]").expect("reads"), None);
        assert!(paused(r#"[{"reason":"TaskPaused"}]"#).is_err());
        assert!(paused("{}").is_err());
    }
}
