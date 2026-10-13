//! Concurrency metrics: pure folds over the log. They change no rule and add
//! no event; they read what the log already records. The definitions are the
//! ones in `docs/bbp-concurrency-design.md` ("2b gate criteria").
//!
//! Model tokens are not visible to the core, so the cost of discarded work is
//! reported in what the protocol does charge: messages, reads and bytes.

use crate::command::Code;
use crate::event::{Event, TurnEnd};
use crate::ids::*;
use crate::record::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Charged work: messages and reads are agent actions; bytes are stored size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub messages: u64,
    pub reads: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub turns_granted: u64,
    /// The most turns live at once.
    pub peak_live: u64,
    /// `turn_ended` with cause `revoked`.
    pub revocations: u64,
    pub aborted_turns: u64,
    pub deadline_turns: u64,
    pub charged: Work,
    /// Work charged inside turns that ended `revoked`.
    pub discarded: Work,
    /// Discarded over charged actions (messages + reads); 0 when nothing was charged.
    pub discarded_share: f64,
    /// `rejected(turn_budget_exhausted)`. Each also ends its turn, so the two are not added.
    pub budget_refusals: u64,
    /// `rejected(stale_turn | token_expired)`: writes lost to a revoked or expired turn.
    pub stale_rejections: u64,
    pub verdicts: u64,
    /// Verdicts accepted although another principal (not the runner) appended
    /// content after the author's last read: the 2a known gap.
    pub stale_verdicts: u64,
}

/// Fold `events` into [`Metrics`].
pub fn of(events: &[Event]) -> Metrics {
    let mut m = Metrics::default();
    let revoked: HashSet<TurnId> = events
        .iter()
        .filter_map(|e| match e {
            Event::TurnEnded {
                turn,
                cause: TurnEnd::Revoked,
            } => Some(*turn),
            _ => None,
        })
        .collect();
    let mut live: HashSet<TurnId> = HashSet::new();
    let mut last_read: HashMap<PrincipalId, usize> = HashMap::new();
    let mut appends: Vec<(usize, PrincipalId)> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        match e {
            Event::TurnGranted { turn, .. } => {
                m.turns_granted += 1;
                live.insert(*turn);
                m.peak_live = m.peak_live.max(live.len() as u64);
            }
            Event::TurnEnded { turn, cause } => {
                live.remove(turn);
                match cause {
                    TurnEnd::Revoked => m.revocations += 1,
                    TurnEnd::Aborted => m.aborted_turns += 1,
                    TurnEnd::Deadline => m.deadline_turns += 1,
                    _ => {}
                }
            }
            Event::Charged {
                principal,
                turn,
                messages,
                bytes,
                reads,
            } => {
                add(&mut m.charged, *messages, *reads, *bytes);
                if turn.is_some_and(|t| revoked.contains(&t)) {
                    add(&mut m.discarded, *messages, *reads, *bytes);
                }
                if *reads > 0 {
                    last_read.insert(principal.clone(), i);
                }
            }
            Event::Rejected { code, .. } => match code {
                Code::TurnBudgetExhausted => m.budget_refusals += 1,
                Code::StaleTurn | Code::TokenExpired => m.stale_rejections += 1,
                _ => {}
            },
            Event::MessageAppended(msg) => {
                if matches!(msg.draft.body, Body::Verdict(_)) {
                    m.verdicts += 1;
                    let after = last_read.get(&msg.from).copied();
                    let unseen = after
                        .is_some_and(|r| appends.iter().any(|(at, by)| *at > r && *by != msg.from));
                    m.stale_verdicts += u64::from(unseen);
                }
                appends.push((i, msg.from.clone()));
            }
            Event::ArtifactStored(a) if a.author.0 != "runner" => {
                appends.push((i, a.author.clone()));
            }
            _ => {}
        }
    }
    let actions = m.charged.messages + m.charged.reads;
    if actions > 0 {
        m.discarded_share = (m.discarded.messages + m.discarded.reads) as f64 / actions as f64;
    }
    m
}

fn add(w: &mut Work, messages: u64, reads: u64, bytes: u64) {
    w.messages += messages;
    w.reads += reads;
    w.bytes += bytes;
}

/// How stored diffs that did not reach the final candidate relate to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlap {
    /// Stored `diff` artifacts not in the final candidate.
    pub unused_diffs: u64,
    /// Of those, the ones touching a path the candidate's diffs touch.
    pub file_overlap_diffs: u64,
    /// Of those, the ones with a hunk whose old-file lines intersect a candidate hunk on the same path.
    pub hunk_overlap_diffs: u64,
}

/// One hunk's old-file line range on a path, `[start, end)`.
type Hunk = (String, u64, u64);

/// Hunks of a unified diff. Lenient: a line it cannot read is skipped, so a
/// malformed diff under-counts overlap instead of failing the measurement.
fn hunks(diff: &[u8]) -> Vec<Hunk> {
    let text = String::from_utf8_lossy(diff);
    let mut path = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("+++ ") {
            let p = p.split('\t').next().unwrap_or(p);
            path = p.strip_prefix("b/").unwrap_or(p).to_owned();
        } else if let Some(h) = line.strip_prefix("@@ -") {
            let old = h.split(' ').next().unwrap_or("");
            let mut it = old.split(',');
            let start = it.next().and_then(|s| s.parse::<u64>().ok());
            let len = it.next().map_or(Some(1), |s| s.parse::<u64>().ok());
            if let (Some(start), Some(len)) = (start, len) {
                out.push((path.clone(), start, start + len.max(1)));
            }
        }
    }
    out
}

/// Compare stored diffs outside the final candidate with the candidate's.
/// `blob` returns a diff's bytes; a blob it cannot return counts as unused with no overlap.
pub fn overlap(events: &[Event], blob: &dyn Fn(&BlobRef) -> Option<Vec<u8>>) -> Overlap {
    let mut diffs: Vec<&ArtifactRecord> = Vec::new();
    let mut by_id: HashMap<ArtId, &ArtifactRecord> = HashMap::new();
    let mut candidate = None;
    for e in events {
        match e {
            Event::ArtifactStored(a) => {
                by_id.insert(a.id, a);
                if a.kind == ArtifactKind::Diff {
                    diffs.push(a);
                }
            }
            Event::CandidateSubmitted { candidate: c, .. } => candidate = Some(*c),
            _ => {}
        }
    }
    let used: HashSet<ArtId> = match candidate.and_then(|c| by_id.get(&c)) {
        Some(ArtifactRecord {
            payload: ArtifactPayload::Candidate(c),
            ..
        }) => c.diffs.iter().copied().collect(),
        _ => return Overlap::default(),
    };
    let kept: Vec<Hunk> = diffs
        .iter()
        .filter(|d| used.contains(&d.id))
        .filter_map(|d| blob(&d.blob))
        .flat_map(|b| hunks(&b))
        .collect();
    let mut o = Overlap::default();
    for d in diffs.iter().filter(|d| !used.contains(&d.id)) {
        o.unused_diffs += 1;
        let theirs = blob(&d.blob).map(|b| hunks(&b)).unwrap_or_default();
        let same_path = |t: &Hunk| kept.iter().any(|k| k.0 == t.0);
        let same_lines = |t: &Hunk| kept.iter().any(|k| k.0 == t.0 && t.1 < k.2 && k.1 < t.2);
        o.file_overlap_diffs += u64::from(theirs.iter().any(same_path));
        o.hunk_overlap_diffs += u64::from(theirs.iter().any(same_lines));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_read_paths_and_old_ranges() {
        let d = b"--- a/x.rs\n+++ b/x.rs\n@@ -3,2 +3,4 @@\n a\n@@ -10 +12 @@\n-b\n+c\n";
        assert_eq!(
            hunks(d),
            vec![("x.rs".into(), 3, 5), ("x.rs".into(), 10, 11)]
        );
    }

    #[test]
    fn a_pure_insertion_still_occupies_one_line() {
        let d = b"+++ b/y\n@@ -7,0 +8,2 @@\n+a\n+b\n";
        assert_eq!(hunks(d), vec![("y".into(), 7, 8)]);
    }

    #[test]
    fn garbage_yields_no_hunks_instead_of_failing() {
        assert!(hunks(b"@@ -x,y +1 @@\nnot a diff").is_empty());
        assert!(hunks(&[0xff, 0xfe, 0x00]).is_empty());
    }

    #[test]
    fn an_empty_log_is_all_zero() {
        assert_eq!(of(&[]), Metrics::default());
        assert_eq!(overlap(&[], &|_| None), Overlap::default());
    }
}
