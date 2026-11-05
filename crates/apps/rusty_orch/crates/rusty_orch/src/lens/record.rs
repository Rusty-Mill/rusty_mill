//! The lens track record's shell (#599 §2, D92): each claim's history read
//! through `bbp why --claim` (the host CLI; orch never reads BBP's store in
//! process, D54), trimmed to the [`Trail`] the pure rule in
//! [`orch_lens::record`] scores.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use orch_lens::record::{
    tallies, ArtId, Change, Item, MsgId, Report, Review, Rule, Step, Tally, Trail,
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
            cites: cites(&e["refs"])?,
            delta: e["delta"].as_i64().ok_or("evidence has no delta")?,
            scoring: e["scoring"].as_bool().ok_or("evidence has no scoring")?,
        }));
    }
    if let Some(s) = v.get("Status") {
        return rule(s).map(Change::Status);
    }
    if let Some(r) = v.get("Report") {
        return Ok(Change::Report(Report {
            report: art(&r["report"], "report")?,
            passed: names(&r["passed"], "passed")?,
            failed: names(&r["failed"], "failed")?,
        }));
    }
    Ok(Change::Other)
}

/// The artifacts an evidence item's refs name; message refs are left out.
fn cites(v: &Value) -> Result<Vec<ArtId>, String> {
    let refs = v.as_seq().ok_or("evidence has no refs")?;
    Ok(refs
        .iter()
        .filter_map(|r| r.get("Art"))
        .filter_map(|a| a["id"].as_u64().map(ArtId))
        .collect())
}

fn art(v: &Value, what: &str) -> Result<ArtId, String> {
    v.as_u64().map(ArtId).ok_or_else(|| format!("no {what} id"))
}

fn names(v: &Value, what: &str) -> Result<Vec<String>, String> {
    let seq = v.as_seq().ok_or_else(|| format!("report has no {what}"))?;
    Ok(seq
        .iter()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect())
}

/// The rule of status step `s` as the record reads it: a line with the
/// holdout report that spoke for the code then (`holdout`, left out when
/// none), a reopen with the items it woke (`woke`, left out when empty).
fn rule(s: &Value) -> Result<Rule, String> {
    let v = &s["rule"];
    let holdout = match s.get("holdout") {
        None => None,
        Some(h) => Some(art(h, "holdout report")?),
    };
    let num = |l: &Value, key: &str| l[key].as_i64().ok_or_else(|| format!("line has no {key}"));
    let count = |l: &Value, key: &str| {
        num(l, key).and_then(|n| u32::try_from(n).map_err(|e| format!("{key}: {e}")))
    };
    let line = |l: &Value, at: &str, authors: &str| {
        Ok(Rule::Line {
            at: num(l, at)?,
            authors: count(l, authors)?,
            min_authors: count(l, "min_authors")?,
            holdout,
        })
    };
    if let Some(l) = v.get("AcceptLine") {
        return line(l, "accept_at", "support_authors");
    }
    if let Some(l) = v.get("DropLine") {
        return line(l, "drop_at", "counter_authors");
    }
    if v.get("Human").is_some() || v.get("Choose").is_some() {
        return Ok(Rule::Human);
    }
    if v.get("Reopen").is_some() {
        let woke = s["woke"].as_seq().unwrap_or(&[]);
        let woke = woke.iter().map(|m| id(m).map(MsgId));
        return woke
            .collect::<Option<_>>()
            .map(Rule::Reopen)
            .ok_or_else(|| "reopen woke an unreadable id".to_owned());
    }
    Ok(Rule::Other)
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
    use rusty_bbp::{ClaimBody, EvidenceKind, PrincipalId, Ref, ReopenCause, Rev, Stakes};

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
        refs: Vec<Ref>,
    ) -> BbpChange {
        BbpChange::Evidence {
            msg: MsgId(msg),
            author: PrincipalId(author.into()),
            kind,
            direction,
            delta,
            scoring: true,
            refs,
        }
    }

    fn reopen(by: ReopenCause, woke: &[u64]) -> BbpStep {
        step(
            BbpChange::Status {
                from: ClaimStatus::Accepted,
                rule: BbpRule::Reopen(by),
                woke: woke.iter().copied().map(MsgId).collect(),
                holdout: None,
            },
            ClaimStatus::Open,
            0,
        )
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|p| (*p).to_owned()).collect()
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
                    BbpChange::Candidate {
                        candidate: ArtId(9),
                    },
                    ClaimStatus::Open,
                    0,
                ),
                step(
                    BbpChange::Report {
                        report: ArtId(50),
                        candidate: ArtId(9),
                        passed: names(&["golden:cases"]),
                        failed: names(&["holdout:api"]),
                    },
                    ClaimStatus::Open,
                    0,
                ),
                step(
                    BbpChange::Failing {
                        profiles: names(&["holdout:api"]),
                        candidate: Some(ArtId(9)),
                    },
                    ClaimStatus::Open,
                    0,
                ),
                step(
                    evidence(
                        2,
                        "sec",
                        Direction::Counter,
                        EvidenceKind::Opinion,
                        -4,
                        vec![Ref::Msg(MsgId(1))],
                    ),
                    ClaimStatus::Open,
                    -4,
                ),
                step(
                    evidence(
                        3,
                        "ops",
                        Direction::Counter,
                        EvidenceKind::Executed,
                        -4,
                        vec![Ref::art(ArtId(50))],
                    ),
                    ClaimStatus::Open,
                    -8,
                ),
                step(
                    BbpChange::Status {
                        from: ClaimStatus::Open,
                        rule: BbpRule::DropLine {
                            drop_at: -8,
                            counter_authors: 2,
                            min_authors: 2,
                        },
                        woke: vec![],
                        holdout: Some(ArtId(50)),
                    },
                    ClaimStatus::Dropped,
                    -8,
                ),
                step(
                    BbpChange::Status {
                        from: ClaimStatus::Dropped,
                        rule: BbpRule::Human { accept: true },
                        woke: vec![],
                        holdout: None,
                    },
                    ClaimStatus::Accepted,
                    -8,
                ),
                reopen(ReopenCause::Evidence(MsgId(3)), &[3]),
                reopen(ReopenCause::Human, &[2, 3]),
                reopen(ReopenCause::Dep(MsgId(9)), &[]),
            ],
        };
        let json = rusty_serde::json::to_string(&history).unwrap();
        let t = trail(&json).unwrap();
        let changes: Vec<&Change> = t.steps.iter().map(|s| &s.change).collect();
        assert_eq!(changes[0], &Change::Other);
        assert_eq!(changes[1], &Change::Other, "candidate steps aren't read");
        assert_eq!(
            changes[2],
            &Change::Report(Report {
                report: ArtId(50),
                passed: names(&["golden:cases"]),
                failed: names(&["holdout:api"]),
            })
        );
        assert_eq!(changes[3], &Change::Other, "the report step carries truth");
        assert_eq!(
            changes[4],
            &Change::Item(Item {
                msg: MsgId(2),
                author: "sec".into(),
                direction: Direction::Counter,
                grounded: false,
                cites: vec![],
                delta: -4,
                scoring: true,
            })
        );
        assert!(
            matches!(changes[5], Change::Item(i) if i.grounded && i.cites == [ArtId(50)] && i.msg == MsgId(3))
        );
        assert_eq!(
            changes[6],
            &Change::Status(Rule::Line {
                at: -8,
                authors: 2,
                min_authors: 2,
                holdout: Some(ArtId(50)),
            })
        );
        assert_eq!(changes[7], &Change::Status(Rule::Human));
        assert_eq!(t.steps[7].status, ClaimStatus::Accepted);
        assert_eq!(
            changes[8..],
            [
                &Change::Status(Rule::Reopen(vec![MsgId(3)])),
                &Change::Status(Rule::Reopen(vec![MsgId(2), MsgId(3)])),
                &Change::Status(Rule::Reopen(vec![])),
            ]
        );
        assert_eq!(t.steps[5].score, -8);
    }

    #[test]
    fn a_broken_history_is_an_error() {
        assert!(trail("{}").unwrap_err().contains("no steps"));
        let bad = r#"{"steps":[{"change":"Posted","status":"Gone","score":0,"epoch":0}]}"#;
        assert!(trail(bad).unwrap_err().contains("Gone"));
        // A line without its minimum is an older `bbp`: not read as a guess.
        let old = r#"{"steps":[{"change":{"Status":{"from":"Open","rule":{"AcceptLine":{"accept_at":8,"support_authors":2}}}},"status":"Accepted","score":8,"epoch":1}]}"#;
        assert_eq!(trail(old).unwrap_err(), "line has no min_authors");
    }
}
