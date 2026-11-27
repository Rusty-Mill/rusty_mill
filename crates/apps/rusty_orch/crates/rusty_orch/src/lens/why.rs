//! `rusty_orch lens why` (#599 §5, D209): one claim's `bbp why --claim`
//! history with the orch lens job behind each evidence post.
//!
//! A step whose evidence message is one a saved blind review posted (the
//! orch-store `lens_record` rows, D92) gains `"job": {lens, epoch,
//! look_harder, unsure}`; any other step is printed as `bbp why` wrote it.
//! Model and cost are not reported: `lens_record` stores neither (Q67).
//! Read-only: posts and saves nothing.

use std::path::PathBuf;

use orch_lens::record::{MsgId, Review};
use orch_store::Store;
use rusty_serde::Value;

use crate::args::ArgsError;

pub const USAGE: &str = "usage: rusty_orch lens why --bbp <bbp binary> --dir <bbp dir> --task <task>\n           --claim <id> --state <orch state dir> [--at <rev>]\n\nPrint one claim's `bbp why --claim` history as one JSON line, with each evidence step a\nlens posted marked with its job: {lens, epoch, look_harder, unsure}. --at stops the history\nat that rev. Model and cost are not reported yet. Posts and saves nothing.";

/// What to explain.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub bbp: PathBuf,
    pub dir: PathBuf,
    pub task: String,
    pub claim: MsgId,
    pub state: PathBuf,
    pub at: Option<u64>,
}

/// Parse `why --bbp … --dir … --task … --claim … --state … [--at …]`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    if args.next().as_deref() != Some("why") {
        return Err(ArgsError::Help);
    }
    let (mut bbp, mut dir, mut task, mut claim, mut state, mut at) =
        (None, None, None, None, None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--bbp" => &mut bbp,
            "--dir" => &mut dir,
            "--task" => &mut task,
            "--claim" => &mut claim,
            "--state" => &mut state,
            "--at" => &mut at,
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
    let number = |v: String, f: &str| {
        let id = v.strip_prefix("msg:").unwrap_or(&v);
        id.parse::<u64>()
            .map_err(|_| ArgsError::Usage(format!("{f} must be a number, got {v:?}")))
    };
    Ok(Options {
        bbp: required(bbp, "--bbp")?.into(),
        dir: required(dir, "--dir")?.into(),
        task: required(task, "--task")?,
        claim: MsgId(number(required(claim, "--claim")?, "--claim")?),
        state: required(state, "--state")?.into(),
        at: at.map(|v| number(v, "--at")).transpose()?,
    })
}

/// Read the claim's history and the task's saved reviews, then [`annotate`].
pub fn run(o: &Options) -> Result<String, String> {
    let board = super::run::board(&o.dir, &o.task)?;
    // Read and released before `bbp why` runs, as `lens record` does.
    let rows = Store::read_lens_reviews(&o.state, &board).map_err(|e| e.to_string())?;
    let history = super::record::why(&o.bbp, &o.dir, &o.task, o.claim, o.at)?;
    let mut line = annotate(&history, o.claim, &rows)?;
    line.push('\n');
    Ok(line)
}

/// `history` (`bbp why --claim` JSON) with each evidence step that one of
/// `rows` posted on `claim` marked with that job.
///
/// # Errors
///
/// A message when `history` isn't a history: not JSON, or no `steps`.
pub fn annotate(history: &str, claim: MsgId, rows: &[Review]) -> Result<String, String> {
    let mut v: Value = rusty_serde::json::from_str(history).map_err(|e| e.to_string())?;
    let Some(Value::Seq(steps)) = steps_mut(&mut v) else {
        return Err("bbp why: no steps".to_owned());
    };
    let jobs: Vec<&Review> = rows.iter().filter(|r| r.claim == claim).collect();
    for step in steps {
        let Some(msg) = evidence_msg(step) else {
            continue;
        };
        if let Some(job) = jobs.iter().find(|r| r.items.contains(&msg)) {
            step.insert("job", job_value(job));
        }
    }
    rusty_serde::json::to_string(&v).map_err(|e| e.to_string())
}

fn steps_mut(v: &mut Value) -> Option<&mut Value> {
    let Value::Map(entries) = v else {
        return None;
    };
    entries
        .iter_mut()
        .find(|(k, _)| k == "steps")
        .map(|(_, s)| s)
}

/// The message an evidence step added, as a number or `msg:N`.
fn evidence_msg(step: &Value) -> Option<MsgId> {
    let m = step.get("change")?.get("Evidence")?.get("msg")?;
    m.as_u64()
        .or_else(|| m.as_str()?.strip_prefix("msg:")?.parse().ok())
        .map(MsgId)
}

/// The job object. A pending row is reported unsure: it may have posted
/// under ids nobody knows (D92), as the record scores it.
fn job_value(r: &Review) -> Value {
    Value::Map(vec![
        ("lens".to_owned(), Value::from(r.lens.as_str())),
        ("epoch".to_owned(), Value::UInt(u64::from(r.epoch))),
        ("look_harder".to_owned(), Value::Bool(r.look_harder)),
        ("unsure".to_owned(), Value::Bool(r.unsure || r.pending)),
    ])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    fn review(lens: &str, claim: u64, items: &[u64]) -> Review {
        Review {
            lens: lens.to_owned(),
            claim: MsgId(claim),
            epoch: 1,
            look_harder: false,
            items: items.iter().copied().map(MsgId).collect(),
            unsure: false,
            pending: false,
        }
    }

    const HISTORY: &str = r#"{"claim":4,"steps":[{"rev":5,"change":"Posted","status":"Open"},{"rev":7,"change":{"Evidence":{"msg":9,"author":"red"}},"status":"Open"},{"rev":8,"change":{"Evidence":{"msg":"msg:10","author":"nano"}},"status":"Open"}]}"#;

    #[test]
    fn why_options_parse() {
        let full = [
            "why", "--bbp", "b", "--dir", "/d", "--task", "t", "--claim", "msg:4", "--state", "/s",
            "--at", "12",
        ];
        assert_eq!(
            options(args(&full)),
            Ok(Options {
                bbp: PathBuf::from("b"),
                dir: PathBuf::from("/d"),
                task: "t".to_owned(),
                claim: MsgId(4),
                state: PathBuf::from("/s"),
                at: Some(12),
            })
        );
        let no_at = options(args(&full[..11])).expect("parses");
        assert_eq!((no_at.claim, no_at.at), (MsgId(4), None));
        assert_eq!(options(args(&["why", "--help"])), Err(ArgsError::Help));
        for bad in [
            &["why"][..],
            &[
                "why", "--bbp", "b", "--dir", "/d", "--task", "t", "--state", "/s",
            ],
            &[
                "why", "--bbp", "b", "--dir", "/d", "--task", "t", "--claim", "x", "--state", "/s",
            ],
            &[
                "why", "--bbp", "b", "--dir", "/d", "--task", "t", "--claim", "4", "--state", "/s",
                "--at", "r",
            ],
            &["why", "--claim", "4", "--claim", "5"],
            &["why", "--bbp", "b", "x"],
        ] {
            assert!(
                matches!(options(args(bad)), Err(ArgsError::Usage(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_lens_post_is_marked_with_its_job_and_others_are_left_alone() {
        let mut harder = review("skeptic", 4, &[10]);
        harder.look_harder = true;
        harder.pending = true;
        let rows = [
            review("adversary", 4, &[9]),
            harder,
            review("adversary", 5, &[9, 10]),
        ];
        let out: Value =
            rusty_serde::json::from_str(&annotate(HISTORY, MsgId(4), &rows).unwrap()).unwrap();
        let steps = out["steps"].as_seq().unwrap();
        assert!(steps[0].get("job").is_none(), "Posted has no job");
        let job = &steps[1]["job"];
        assert_eq!(job["lens"].as_str(), Some("adversary"));
        assert_eq!(job["epoch"].as_u64(), Some(1));
        assert_eq!(job["look_harder"].as_bool(), Some(false));
        assert_eq!(job["unsure"].as_bool(), Some(false));
        let job = &steps[2]["job"];
        assert_eq!(job["lens"].as_str(), Some("skeptic"));
        assert_eq!(job["look_harder"].as_bool(), Some(true));
        assert_eq!(job["unsure"].as_bool(), Some(true), "pending reads unsure");
        assert_eq!(out["claim"].as_u64(), Some(4), "the rest is kept");
    }

    #[test]
    fn rows_for_another_claim_mark_nothing() {
        let rows = [review("adversary", 5, &[9, 10])];
        let out = annotate(HISTORY, MsgId(4), &rows).unwrap();
        assert!(!out.contains("\"job\""), "{out}");
    }

    #[test]
    fn a_broken_history_is_an_error() {
        assert!(annotate("not json", MsgId(4), &[]).is_err());
        assert_eq!(
            annotate(r#"{"claim":4}"#, MsgId(4), &[]),
            Err("bbp why: no steps".to_owned())
        );
    }

    /// `run` reads the rows from the store and the history from `bbp why
    /// --at`, and prints one line.
    #[cfg(unix)]
    #[test]
    fn run_reads_the_history_through_bbp_why_with_at() {
        use super::super::record::tests::retry_locked;
        use std::os::unix::fs::PermissionsExt as _;
        let d = std::env::temp_dir().join(format!("orch-lens-why-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (dir, state) = (d.join("bbp"), d.join("state"));
        std::fs::create_dir_all(&dir).expect("dir");
        let board = super::super::run::board(&dir, "t").expect("board");
        retry_locked(|| {
            Store::open(&state)
                .and_then(|mut s| s.save_lens_review(&board, &review("adversary", 4, &[9])))
                .map_err(|e| e.to_string())
        })
        .expect("saved");
        let bbp = d.join("bbp.sh");
        let script = format!(
            "#!/bin/sh\n[ \"$1 $5 $7 $8 $9\" = \"why t 4 --at 7\" ] && echo '{HISTORY}' || exit 3\n"
        );
        std::fs::write(&bbp, script).expect("script");
        std::fs::set_permissions(&bbp, std::fs::Permissions::from_mode(0o755)).expect("mode");
        let o = Options {
            bbp,
            dir,
            task: "t".to_owned(),
            claim: MsgId(4),
            state,
            at: Some(7),
        };
        let out = retry_locked(|| run(&o)).expect("ran");
        assert!(
            out.ends_with('\n') && out.matches('\n').count() == 1,
            "{out}"
        );
        assert!(out.contains(r#""job":{"lens":"adversary""#), "{out}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
