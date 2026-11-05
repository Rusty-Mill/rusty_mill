//! The lens track record's shell (#599 §2, D92): each claim's history read
//! through `bbp why --claim` (the host CLI; orch never reads BBP's store in
//! process, D54), trimmed to the [`Trail`] the pure rule in
//! [`orch_lens::record`] scores.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use orch_lens::record::{
    tallies, Change, Failing, Item, MsgId, Review, Rule, Step, Tally, Trail, Wake,
};
use rusty_bbp::{ClaimStatus, Direction};
use rusty_serde::Value;

/// Each lens's tally over `reviews`, with every claim's history read from
/// `bbp why`.
///
/// # Errors
///
/// A message naming the claim whose history could not be read.
pub fn read(
    bbp: &Path,
    dir: &Path,
    task: &str,
    reviews: &[Review],
) -> Result<BTreeMap<String, Tally>, String> {
    let claims: BTreeSet<MsgId> = reviews.iter().map(|r| r.claim).collect();
    let mut trails = BTreeMap::new();
    for claim in claims {
        let json = why(bbp, dir, task, claim)?;
        let trail = trail(&json).map_err(|e| format!("{claim}: {e}"))?;
        trails.insert(claim, trail);
    }
    Ok(tallies(reviews, &trails))
}

/// `bbp why --claim`'s one line of JSON.
fn why(bbp: &Path, dir: &Path, task: &str, claim: MsgId) -> Result<String, String> {
    let out = Command::new(bbp)
        .arg("why")
        .arg("--dir")
        .arg(dir)
        .args(["--task", task, "--claim", &claim.0.to_string()])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bbp.display()))?;
    if !out.status.success() {
        return Err(format!(
            "bbp why --claim {}: {}",
            claim.0,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("bbp why --claim {}: {e}", claim.0))
}

/// A claim's [`Trail`] from `bbp why --claim` JSON.
///
/// # Errors
///
/// A message naming the first field that isn't what the history writes.
pub fn trail(json: &str) -> Result<Trail, String> {
    let v: Value = rusty_serde::json::from_str(json).map_err(|e| e.to_string())?;
    let steps = v["steps"].as_seq().ok_or("no steps")?;
    let steps = steps.iter().map(step).collect::<Result<_, _>>()?;
    Ok(Trail { steps })
}

fn step(v: &Value) -> Result<Step, String> {
    Ok(Step {
        change: change(&v["change"])?,
        status: status(v["status"].as_str().ok_or("no status")?)?,
        score: v["score"].as_i64().ok_or("no score")?,
        epoch: v["epoch"]
            .as_u64()
            .and_then(|e| u32::try_from(e).ok())
            .ok_or("no epoch")?,
    })
}

fn change(v: &Value) -> Result<Change, String> {
    if let Some(e) = v.get("Evidence") {
        return Ok(Change::Item(Item {
            msg: MsgId(id(&e["msg"]).ok_or("evidence has no msg")?),
            author: e["author"]
                .as_str()
                .ok_or("evidence has no author")?
                .to_owned(),
            direction: match e["direction"].as_str() {
                Some("Support") => Direction::Support,
                Some("Counter") => Direction::Counter,
                other => return Err(format!("unknown direction {other:?}")),
            },
            grounded: e["kind"].as_str() != Some("Opinion"),
            executed: e["kind"].as_str() == Some("Executed"),
            delta: e["delta"].as_i64().ok_or("evidence has no delta")?,
            scoring: e["scoring"].as_bool().ok_or("evidence has no scoring")?,
        }));
    }
    if let Some(s) = v.get("Status") {
        return Ok(Change::Status(rule(&s["rule"])));
    }
    if let Some(f) = v.get("Failing") {
        let profiles = f["profiles"].as_seq().ok_or("failing has no profiles")?;
        // The history doesn't name the candidate the report tested, nor
        // when the current candidate changes: no golden truth (D92, Q35).
        return Ok(Change::Failing(Failing {
            profiles: profiles
                .iter()
                .filter_map(|p| p.as_str().map(str::to_owned))
                .collect(),
            candidate: None,
        }));
    }
    Ok(Change::Other)
}

/// The rule as the record reads it. The history doesn't carry the task's
/// `min_authors` yet, so a line's minimum is unknown.
fn rule(v: &Value) -> Rule {
    let line = |l: &Value, at: &str, authors: &str| Rule::Line {
        at: l[at].as_i64().unwrap_or(0),
        authors: l[authors]
            .as_u64()
            .and_then(|a| u32::try_from(a).ok())
            .unwrap_or(0),
        min_authors: None,
    };
    if let Some(l) = v.get("AcceptLine") {
        return line(l, "accept_at", "support_authors");
    }
    if let Some(l) = v.get("DropLine") {
        return line(l, "drop_at", "counter_authors");
    }
    if v.get("Human").is_some() || v.get("Choose").is_some() {
        return Rule::Human;
    }
    if let Some(r) = v.get("Reopen") {
        return Rule::Reopen(wake(r));
    }
    Rule::Other
}

/// What a reopen woke. An unreadable cause is read as a human reopen,
/// which only makes jobs more pivotal.
fn wake(v: &Value) -> Wake {
    if let Some(m) = v.get("Evidence").and_then(id) {
        return Wake::Item(MsgId(m));
    }
    match v.as_str() {
        Some("Intent") => Wake::None,
        _ if v.get("Dep").is_some() => Wake::None,
        _ => Wake::Dormant,
    }
}

fn status(s: &str) -> Result<ClaimStatus, String> {
    Ok(match s {
        "Open" => ClaimStatus::Open,
        "Accepted" => ClaimStatus::Accepted,
        "Dropped" => ClaimStatus::Dropped,
        "Void" => ClaimStatus::Void,
        "Escalated" => ClaimStatus::Escalated,
        "Superseded" => ClaimStatus::Superseded,
        other => return Err(format!("unknown status {other:?}")),
    })
}

/// A message id, as a number or `msg:N`.
fn id(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str()?.strip_prefix("msg:")?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_bbp::history::{Change as BbpChange, History, Rule as BbpRule, Step as BbpStep};
    use rusty_bbp::{ClaimBody, EvidenceKind, PrincipalId, ReopenCause, Rev, Stakes};

    fn step(change: BbpChange, status: ClaimStatus, score: i64) -> BbpStep {
        BbpStep {
            rev: Rev(1),
            change,
            status,
            score,
            confidence: 0,
            epoch: 0,
        }
    }

    fn evidence(
        msg: u64,
        author: &str,
        direction: Direction,
        kind: EvidenceKind,
        delta: i64,
    ) -> BbpChange {
        BbpChange::Evidence {
            msg: MsgId(msg),
            author: PrincipalId(author.into()),
            kind,
            direction,
            delta,
            scoring: true,
        }
    }

    fn reopen(by: ReopenCause) -> BbpStep {
        step(
            BbpChange::Status {
                from: ClaimStatus::Accepted,
                rule: BbpRule::Reopen(by),
            },
            ClaimStatus::Open,
            0,
        )
    }

    /// The history `bbp why` prints, read back as a trail.
    #[test]
    fn a_bbp_history_reads_as_a_trail() {
        let history = History {
            claim: MsgId(1),
            author: PrincipalId("a".into()),
            body: ClaimBody {
                text: "x".into(),
                domains: vec![],
                stakes: Stakes::Low,
                reversible: true,
                deps: vec![],
                answers: None,
                supersedes: None,
            },
            steps: vec![
                step(BbpChange::Posted, ClaimStatus::Open, 0),
                step(
                    BbpChange::Failing {
                        profiles: vec!["golden:cases".into()],
                    },
                    ClaimStatus::Open,
                    0,
                ),
                step(
                    evidence(2, "sec", Direction::Counter, EvidenceKind::Opinion, -4),
                    ClaimStatus::Open,
                    -4,
                ),
                step(
                    evidence(3, "ops", Direction::Counter, EvidenceKind::Executed, -4),
                    ClaimStatus::Open,
                    -8,
                ),
                step(
                    BbpChange::Status {
                        from: ClaimStatus::Open,
                        rule: BbpRule::DropLine {
                            drop_at: -8,
                            counter_authors: 2,
                        },
                    },
                    ClaimStatus::Dropped,
                    -8,
                ),
                step(
                    BbpChange::Status {
                        from: ClaimStatus::Dropped,
                        rule: BbpRule::Human { accept: true },
                    },
                    ClaimStatus::Accepted,
                    -8,
                ),
                reopen(ReopenCause::Evidence(MsgId(3))),
                reopen(ReopenCause::Human),
                reopen(ReopenCause::Dep(MsgId(9))),
                reopen(ReopenCause::Intent),
            ],
        };
        let json = rusty_serde::json::to_string(&history).unwrap();
        let t = trail(&json).unwrap();
        let changes: Vec<&Change> = t.steps.iter().map(|s| &s.change).collect();
        assert_eq!(changes[0], &Change::Other);
        assert_eq!(
            changes[1],
            &Change::Failing(Failing {
                profiles: vec!["golden:cases".into()],
                candidate: None
            })
        );
        assert_eq!(
            changes[2],
            &Change::Item(Item {
                msg: MsgId(2),
                author: "sec".into(),
                direction: Direction::Counter,
                grounded: false,
                executed: false,
                delta: -4,
                scoring: true,
            })
        );
        assert!(
            matches!(changes[3], Change::Item(i) if i.grounded && i.executed && i.msg == MsgId(3))
        );
        assert_eq!(
            changes[4],
            &Change::Status(Rule::Line {
                at: -8,
                authors: 2,
                min_authors: None
            })
        );
        assert_eq!(changes[5], &Change::Status(Rule::Human));
        assert_eq!(t.steps[5].status, ClaimStatus::Accepted);
        let wakes: Vec<&Change> = changes[6..].to_vec();
        assert_eq!(
            wakes,
            [
                &Change::Status(Rule::Reopen(Wake::Item(MsgId(3)))),
                &Change::Status(Rule::Reopen(Wake::Dormant)),
                &Change::Status(Rule::Reopen(Wake::None)),
                &Change::Status(Rule::Reopen(Wake::None)),
            ]
        );
        assert_eq!(t.steps[3].score, -8);
    }

    #[test]
    fn a_broken_history_is_an_error() {
        assert!(trail("{}").unwrap_err().contains("no steps"));
        let bad = r#"{"steps":[{"change":"Posted","status":"Gone","score":0,"epoch":0}]}"#;
        assert!(trail(bad).unwrap_err().contains("Gone"));
    }
}
