//! Concurrency metrics: pure folds over the log. They change no rule and add
//! no event; they read what the log already records. The definitions are the
//! ones in `docs/bbp-concurrency-design.md` ("2b gate criteria").
//!
//! Model tokens are not visible to the core, so the cost of discarded work is
//! reported in what the protocol does charge: messages, reads and bytes.

use crate::command::{Code, Response};
use crate::event::{Event, TurnEnd};
use crate::ids::*;
use crate::record::*;
use crate::role::Cap;
use crate::state::TaskState;
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
    /// Verdicts accepted although a message or artifact that the author could
    /// read, appended by another principal (not the runner) during the
    /// author's verdict turn, was never returned to the author by any read the
    /// log records (the 2a known gap). Exact, from the responses of accepted
    /// reads: a `get_artifact` does not count as having seen messages, and a
    /// page that stopped short does not count as having seen what it omitted.
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
    let folded = TaskState::fold(TaskId(String::new()), events);
    let mut turn_role: HashMap<TurnId, RoleId> = HashMap::new();
    let mut turn_start: HashMap<RoleId, usize> = HashMap::new();
    let mut seen_msgs: HashMap<PrincipalId, HashSet<MsgId>> = HashMap::new();
    let mut seen_arts: HashMap<PrincipalId, HashSet<ArtId>> = HashMap::new();
    let mut msg_appends: Vec<(usize, &Message)> = Vec::new();
    let mut art_appends: Vec<(usize, ArtId, &PrincipalId)> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        match e {
            Event::TurnGranted { turn, role, .. } => {
                turn_role.insert(*turn, role.clone());
                turn_start.insert(role.clone(), i);
                m.turns_granted += 1;
                live.insert(*turn);
                m.peak_live = m.peak_live.max(live.len() as u64);
            }
            Event::TurnEnded { turn, cause } => {
                live.remove(turn);
                if let Some(role) = turn_role.get(turn) {
                    turn_start.remove(role);
                }
                match cause {
                    TurnEnd::Revoked => m.revocations += 1,
                    TurnEnd::Aborted => m.aborted_turns += 1,
                    TurnEnd::Deadline => m.deadline_turns += 1,
                    _ => {}
                }
            }
            Event::Charged {
                turn,
                messages,
                bytes,
                reads,
                ..
            } => {
                add(&mut m.charged, *messages, *reads, *bytes);
                if turn.is_some_and(|t| revoked.contains(&t)) {
                    add(&mut m.discarded, *messages, *reads, *bytes);
                }
            }
            Event::OpAccepted {
                principal,
                response,
                ..
            } => match response {
                Response::Read { messages, .. } => {
                    seen_msgs
                        .entry(principal.clone())
                        .or_default()
                        .extend(messages.iter().map(|x| x.id));
                }
                Response::Artifact(rec) => {
                    seen_arts
                        .entry(principal.clone())
                        .or_default()
                        .insert(rec.id);
                }
                _ => {}
            },
            Event::Rejected { code, .. } => match code {
                Code::TurnBudgetExhausted => m.budget_refusals += 1,
                Code::StaleTurn | Code::TokenExpired => m.stale_rejections += 1,
                _ => {}
            },
            Event::MessageAppended(msg) => {
                if matches!(msg.draft.body, Body::Verdict(_)) {
                    m.verdicts += 1;
                    let start = msg.from_role.as_ref().and_then(|r| turn_start.get(r));
                    let reader = msg.from_role.as_ref().and_then(|r| folded.roster.get(r));
                    if let (Some(&start), Some(reader)) = (start, reader) {
                        let heard = seen_msgs.get(&msg.from);
                        let looked = seen_arts.get(&msg.from);
                        let unseen_msg = msg_appends.iter().any(|(at, other)| {
                            *at > start
                                && other.from != msg.from
                                && crate::engine::visible(&folded, reader, other)
                                && !heard.is_some_and(|h| h.contains(&other.id))
                        });
                        let unseen_art = art_appends.iter().any(|(at, id, by)| {
                            *at > start
                                && **by != msg.from
                                && reader.holds(Cap::Read(
                                    folded
                                        .artifacts
                                        .get(id)
                                        .map_or(ArtifactKind::Log, |a| a.kind),
                                ))
                                && !looked.is_some_and(|l| l.contains(id))
                        });
                        m.stale_verdicts += u64::from(unseen_msg || unseen_art);
                    }
                }
                msg_appends.push((i, msg));
            }
            Event::ArtifactStored(a) if a.author.0 != "runner" => {
                art_appends.push((i, a.id, &a.author));
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

/// A file path from a `---`/`+++` header: `None` for `/dev/null` (the file
/// does not exist on that side) and with the `a/`/`b/` prefix and any
/// trailing timestamp removed.
fn header_path(rest: &str) -> Option<String> {
    let p = rest.split('\t').next().unwrap_or(rest);
    if p == "/dev/null" {
        return None;
    }
    let p = p
        .strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p);
    Some(p.to_owned())
}

/// `-start,len` or `+start,len` from a hunk header, `len` defaulting to 1.
fn range(part: &str) -> Option<(u64, u64)> {
    let mut it = part.split(',');
    let start = it.next()?.parse::<u64>().ok()?;
    let len = it.next().map_or(Some(1), |s| s.parse::<u64>().ok())?;
    Some((start, len))
}

/// Hunks of a unified diff, each keyed by the path whose old lines it covers
/// (the `---` path, or the `+++` path for a new file). A deletion has
/// `+++ /dev/null`, so the old path, not the new one, is the key. Lines of a
/// hunk body are consumed by their counts, so a removed line that itself
/// starts with `-- ` is not read as a header. Lenient: a header it cannot read
/// is skipped, so a malformed diff under-counts overlap instead of failing the
/// measurement.
fn hunks(diff: &[u8]) -> Vec<Hunk> {
    let text = String::from_utf8_lossy(diff);
    let mut old_path: Option<String> = None;
    let mut key: Option<String> = None;
    let mut out = Vec::new();
    let (mut old_left, mut new_left) = (0u64, 0u64);
    for line in text.lines() {
        // A body line starts with ' ', '-', '+' or '\\', never `@@`, so this is a
        // header even if the previous hunk's counts promised more lines.
        if let Some(h) = line.strip_prefix("@@ -") {
            (old_left, new_left) = (0, 0);
            let mut parts = h.split(' ');
            let old = parts.next().and_then(range);
            let new = parts
                .next()
                .and_then(|n| n.strip_prefix('+'))
                .and_then(range);
            if let (Some((start, len)), Some((_, new_len)), Some(path)) = (old, new, key.clone()) {
                if let Some(end) = start.checked_add(len.max(1)) {
                    out.push((path, start, end));
                    (old_left, new_left) = (len, new_len);
                }
            }
        } else if old_left > 0 || new_left > 0 {
            match line.chars().next() {
                Some('-') => old_left = old_left.saturating_sub(1),
                Some('+') => new_left = new_left.saturating_sub(1),
                Some('\\') => {}
                _ => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                }
            }
        } else if let Some(p) = line.strip_prefix("--- ") {
            old_path = header_path(p);
            key = None;
        } else if let Some(p) = line.strip_prefix("+++ ") {
            key = old_path.clone().or_else(|| header_path(p));
        }
    }
    out
}

/// Does `theirs` touch a path `kept` touches, and do any of its old-line
/// ranges intersect a kept range on that path? Header ranges include unchanged
/// context lines, so this is a screening proxy and overcounts disjoint edits
/// that sit close together; the gate in the design note uses a three-way merge.
fn relate(kept: &[Hunk], theirs: &[Hunk]) -> (bool, bool) {
    let file = theirs.iter().any(|t| kept.iter().any(|k| k.0 == t.0));
    let lines = theirs
        .iter()
        .any(|t| kept.iter().any(|k| k.0 == t.0 && t.1 < k.2 && k.1 < t.2));
    (file, lines)
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
        let (file, hunk) = relate(&kept, &theirs);
        o.file_overlap_diffs += u64::from(file);
        o.hunk_overlap_diffs += u64::from(hunk);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_read_paths_and_old_ranges() {
        let d = b"--- a/x.rs\n+++ b/x.rs\n@@ -3,2 +3,2 @@\n a\n-b\n+c\n@@ -10 +12 @@\n-b\n+c\n";
        assert_eq!(
            hunks(d),
            vec![("x.rs".into(), 3, 5), ("x.rs".into(), 10, 11)]
        );
    }

    #[test]
    fn a_pure_insertion_still_occupies_one_line() {
        let d = b"--- a/y\n+++ b/y\n@@ -7,0 +8,2 @@\n+a\n+b\n";
        assert_eq!(hunks(d), vec![("y".into(), 7, 8)]);
    }

    #[test]
    fn garbage_yields_no_hunks_instead_of_failing() {
        assert!(hunks(b"@@ -x,y +1 @@\nnot a diff").is_empty());
        assert!(hunks(&[0xff, 0xfe, 0x00]).is_empty());
    }

    const DEL_A: &[u8] = b"--- a/a.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n";
    const DEL_B: &[u8] = b"--- a/b.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n";
    const MOD_A: &[u8] = b"--- a/a.rs\n+++ b/a.rs\n@@ -2 +2 @@\n-y\n+z\n";

    #[test]
    fn two_unrelated_deletions_do_not_overlap() {
        // Both have `+++ /dev/null`; keying by the new path would call them the same file.
        assert_eq!(hunks(DEL_A), vec![("a.rs".into(), 1, 3)]);
        assert_eq!(relate(&hunks(DEL_A), &hunks(DEL_B)), (false, false));
    }

    #[test]
    fn a_deletion_and_a_modification_of_one_file_overlap() {
        assert_eq!(relate(&hunks(DEL_A), &hunks(MOD_A)), (true, true));
        assert_eq!(relate(&hunks(MOD_A), &hunks(DEL_A)), (true, true));
    }

    #[test]
    fn two_new_files_at_one_path_overlap_and_at_two_paths_do_not() {
        let new = |p: &str| format!("--- /dev/null\n+++ b/{p}\n@@ -0,0 +1 @@\n+x\n").into_bytes();
        assert_eq!(
            relate(&hunks(&new("n.rs")), &hunks(&new("n.rs"))),
            (true, true)
        );
        assert_eq!(
            relate(&hunks(&new("n.rs")), &hunks(&new("m.rs"))),
            (false, false)
        );
    }

    #[test]
    fn a_removed_line_that_looks_like_a_header_is_body_not_a_header() {
        // The removed line is `-- b` and renders as `--- b`; the counts say it is body.
        let d = b"--- a/c.sql\n+++ b/c.sql\n@@ -1,2 +1,1 @@\n---- b\n x\n@@ -9 +9 @@\n-q\n+r\n";
        assert_eq!(
            hunks(d),
            vec![("c.sql".into(), 1, 3), ("c.sql".into(), 9, 10)]
        );
    }

    #[test]
    fn numeric_headers_cannot_overflow() {
        let d = b"--- a/o\n+++ b/o\n@@ -18446744073709551615,5 +1 @@\n-x\n@@ -1 +1 @@\n-y\n+z\n";
        assert_eq!(
            hunks(d),
            vec![("o".into(), 1, 2)],
            "the overflowing header is skipped"
        );
    }

    #[test]
    fn an_empty_log_is_all_zero() {
        assert_eq!(of(&[]), Metrics::default());
        assert_eq!(overlap(&[], &|_| None), Overlap::default());
    }
}
