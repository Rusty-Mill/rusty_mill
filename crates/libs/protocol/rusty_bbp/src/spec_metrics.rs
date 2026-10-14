//! Spec-authoring metrics for the 2b gate (`docs/bbp-concurrency-design.md`,
//! "2b gate criteria"): pure folds over the log and the stored spec bytes.
//! They change no rule and add no event.
//!
//! [`spec_conflicts`] backs criterion 4 ("contention is real"): did two
//! authors edit the same lines of a spec? It compares each non-approved spec
//! version with the approved one on a common base, so disjoint edits that sit
//! near each other (which header ranges and context lines would call
//! overlapping) are not conflicts.

use crate::codec::spec_markdown;
use crate::event::Event;
use crate::ids::*;
use crate::record::*;
use crate::state::Gate;
use rusty_diff::{diff_myers, DiffOp, MAX_DIFF_INPUT_LEN};
use rusty_serde::{Deserialize, Serialize};

/// What comparing one spec version with the approved one found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairOutcome {
    /// Both edited at least one common base line (or inserted at one point)
    /// differently.
    Conflict,
    /// Their edits touch different base lines, or are identical: a clean merge.
    Clean,
    /// No spec version was stored before both, so there is no common base:
    /// the two are independent rewrites. Reported apart, never as a conflict.
    ParallelRewrite,
    /// A stored spec could not be read or decoded, or the diff is larger than
    /// the diff crate's exact limit. Counted, not guessed.
    Unmeasured,
}

/// One non-approved spec version against the approved one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecPair {
    pub version: ArtId,
    /// The common base: the latest spec version stored before either of the
    /// two, which both authors could therefore have read.
    pub base: Option<ArtId>,
    pub outcome: PairOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecConflicts {
    /// The spec the human approved at the plan gate (the last approval), if any.
    pub approved: Option<ArtId>,
    /// Every other stored spec version, in storage order.
    pub pairs: Vec<SpecPair>,
}

impl SpecConflicts {
    fn count(&self, o: PairOutcome) -> u64 {
        self.pairs.iter().filter(|p| p.outcome == o).count() as u64
    }
    pub fn conflicts(&self) -> u64 {
        self.count(PairOutcome::Conflict)
    }
    pub fn parallel_rewrites(&self) -> u64 {
        self.count(PairOutcome::ParallelRewrite)
    }
    pub fn unmeasured(&self) -> u64 {
        self.count(PairOutcome::Unmeasured)
    }
}

/// A run of changed base lines: `[start, end)` replaced by `ins`. A pure
/// insertion has `start == end`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Region {
    start: usize,
    end: usize,
    ins: Vec<String>,
}

/// The changed regions turning `base` into `new`, in base coordinates.
fn regions(ops: &[DiffOp<String>]) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::new();
    let mut open: Option<Region> = None;
    let mut at = 0usize;
    for op in ops {
        match op {
            DiffOp::Keep(_) => {
                out.extend(open.take());
                at += 1;
            }
            DiffOp::Delete(_) => {
                let r = open.get_or_insert(Region {
                    start: at,
                    end: at,
                    ins: Vec::new(),
                });
                at += 1;
                r.end = at;
            }
            DiffOp::Insert(line) => {
                open.get_or_insert(Region {
                    start: at,
                    end: at,
                    ins: Vec::new(),
                })
                .ins
                .push(line.clone());
            }
        }
    }
    out.extend(open);
    out
}

/// The effect on shared base lines is empty for a deletion, or the matching
/// replacement lines for a line-for-line edit. A length-changing replacement
/// cannot be subdivided unambiguously; keep it atomic.
fn shared_effect(r: &Region, start: usize, end: usize) -> Option<&[String]> {
    if r.ins.is_empty() {
        Some(&[])
    } else if r.ins.len() == r.end - r.start {
        Some(&r.ins[start - r.start..end - r.start])
    } else {
        None
    }
}

/// Compare effects on shared base lines, not whole coalesced regions. Touching
/// edits remain disjoint. Identical atomic replacements and insertions agree.
fn clash(a: &Region, b: &Region) -> bool {
    if a == b {
        return false;
    }
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    if start < end {
        return match (shared_effect(a, start, end), shared_effect(b, start, end)) {
            (Some(x), Some(y)) => x != y,
            _ => true,
        };
    }
    a.start == a.end && b.start == b.end && a.start == b.start
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

/// A diff the exact algorithm declined (it falls back to delete-all,
/// insert-all) shows up as no kept line although the texts share one.
fn degraded(base: &[String], new: &[String], ops: &[DiffOp<String>]) -> bool {
    base.len() + new.len() > MAX_DIFF_INPUT_LEN
        || (!ops.iter().any(|o| matches!(o, DiffOp::Keep(_)))
            && base.iter().any(|l| new.contains(l)))
}

fn compare(base: &[String], a: &[String], b: &[String]) -> PairOutcome {
    if base.len() + a.len() > MAX_DIFF_INPUT_LEN || base.len() + b.len() > MAX_DIFF_INPUT_LEN {
        return PairOutcome::Unmeasured;
    }
    let (da, db) = (diff_myers(base, a), diff_myers(base, b));
    if degraded(base, a, &da) || degraded(base, b, &db) {
        return PairOutcome::Unmeasured;
    }
    let (ra, rb) = (regions(&da), regions(&db));
    if ra.iter().any(|x| rb.iter().any(|y| clash(x, y))) {
        PairOutcome::Conflict
    } else {
        PairOutcome::Clean
    }
}

/// Compare every non-approved spec version with the approved one. `blob`
/// returns a stored spec's bytes; the core stays free of I/O.
pub fn spec_conflicts(
    events: &[Event],
    blob: &dyn Fn(&BlobRef) -> Option<Vec<u8>>,
) -> SpecConflicts {
    let mut versions: Vec<(ArtId, BlobRef)> = Vec::new();
    let mut approved = None;
    for e in events {
        match e {
            Event::ArtifactStored(a) if a.kind == ArtifactKind::Spec => {
                versions.push((a.id, a.blob));
            }
            Event::HumanApproval {
                gate: Gate::Plan,
                subject,
                ..
            } => approved = Some(*subject),
            _ => {}
        }
    }
    let Some(approved) = approved else {
        return SpecConflicts::default();
    };
    let Some(a_at) = versions.iter().position(|(id, _)| *id == approved) else {
        return SpecConflicts::default();
    };
    let text = |at: usize| -> Option<Vec<String>> {
        let bytes = blob(&versions[at].1)?;
        Some(lines(&spec_markdown(&bytes).ok()?))
    };
    let mut out = SpecConflicts {
        approved: Some(approved),
        pairs: Vec::new(),
    };
    for (v_at, (id, _)) in versions.iter().enumerate() {
        if v_at == a_at {
            continue;
        }
        // Stored earlier than both, so both authors could have read it.
        let base_at = (0..v_at.min(a_at)).next_back();
        let outcome = match base_at {
            None => PairOutcome::ParallelRewrite,
            Some(b) => match (text(b), text(v_at), text(a_at)) {
                (Some(base), Some(v), Some(a)) => compare(&base, &v, &a),
                _ => PairOutcome::Unmeasured,
            },
        };
        out.pairs.push(SpecPair {
            version: *id,
            base: base_at.map(|b| versions[b].0),
            outcome,
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::codec::encode_artifact;
    use std::collections::HashMap;

    /// A log of stored specs plus their bytes, built by hand.
    struct Specs {
        events: Vec<Event>,
        blobs: HashMap<Sha256, Vec<u8>>,
    }

    impl Specs {
        fn new() -> Specs {
            Specs {
                events: Vec::new(),
                blobs: HashMap::new(),
            }
        }
        fn store(&mut self, id: u64, markdown: &str) -> ArtId {
            let payload = ArtifactPayload::Spec { brief: ArtId(1) };
            let bytes = encode_artifact(&payload, markdown.as_bytes()).expect("encode");
            let sha = Sha256::of(&bytes);
            self.blobs.insert(sha, bytes.clone());
            self.events.push(Event::ArtifactStored(ArtifactRecord {
                id: ArtId(id),
                author: PrincipalId("p".into()),
                kind: ArtifactKind::Spec,
                blob: BlobRef {
                    sha,
                    len: bytes.len() as u64,
                },
                payload,
                spec: None,
                candidate: None,
                run: None,
            }));
            ArtId(id)
        }
        fn approve(&mut self, id: ArtId) {
            self.events.push(Event::HumanApproval {
                gate: Gate::Plan,
                subject: id,
                run: None,
            });
        }
        fn run(&self) -> SpecConflicts {
            spec_conflicts(&self.events, &|b| self.blobs.get(&b.sha).cloned())
        }
    }

    fn doc(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    /// `doc(n)` with these 1-based lines replaced.
    fn edit(n: usize, changes: &[(usize, &str)]) -> String {
        (1..=n)
            .map(|i| match changes.iter().find(|(l, _)| *l == i) {
                Some((_, t)) => format!("{t}\n"),
                None => format!("line {i}\n"),
            })
            .collect()
    }

    /// base, then `unused` stored first and `approved` second.
    fn pair(unused: &str, approved: &str) -> SpecPair {
        let mut s = Specs::new();
        s.store(2, &doc(12));
        s.store(3, unused);
        let a = s.store(4, approved);
        s.approve(a);
        let r = s.run();
        assert_eq!(r.approved, Some(ArtId(4)));
        assert_eq!(r.pairs.len(), 2, "the base and the unused version: {r:?}");
        r.pairs.into_iter().find(|p| p.version == ArtId(3)).unwrap()
    }

    #[test]
    fn edits_to_different_lines_merge_cleanly() {
        let p = pair(&edit(12, &[(2, "X")]), &edit(12, &[(9, "Y")]));
        assert_eq!((p.base, p.outcome), (Some(ArtId(2)), PairOutcome::Clean));
    }

    #[test]
    fn nearby_disjoint_edits_are_not_conflicts() {
        // Lines 10 and 12 with context between them: header ranges would overlap, lines do not.
        let p = pair(&edit(12, &[(10, "X")]), &edit(12, &[(12, "Y")]));
        assert_eq!(p.outcome, PairOutcome::Clean);
    }

    #[test]
    fn adjacent_lines_are_different_lines() {
        let p = pair(&edit(12, &[(3, "X")]), &edit(12, &[(4, "Y")]));
        assert_eq!(p.outcome, PairOutcome::Clean);
    }

    #[test]
    fn the_same_line_edited_differently_conflicts() {
        let p = pair(&edit(12, &[(5, "X")]), &edit(12, &[(5, "Y")]));
        assert_eq!(p.outcome, PairOutcome::Conflict);
    }

    #[test]
    fn the_same_edit_on_both_sides_is_clean() {
        let p = pair(&edit(12, &[(5, "X")]), &edit(12, &[(5, "X")]));
        assert_eq!(p.outcome, PairOutcome::Clean);
    }

    #[test]
    fn partial_overlaps_compare_only_shared_effects() {
        let base = lines("a\nb\nc\nd\n");
        for (other, approved, expected) in [
            ("a\nX\nY\nd\n", "a\nX\nc\nd\n", PairOutcome::Clean),
            ("a\nX\nY\nd\n", "a\nb\nY\nd\n", PairOutcome::Clean),
            ("a\nX\nY\nd\n", "a\nZ\nc\nd\n", PairOutcome::Conflict),
            ("a\nX\nY\nd\n", "a\nb\nZ\nd\n", PairOutcome::Conflict),
            ("a\nd\n", "a\nb\nd\n", PairOutcome::Clean),
            ("a\nd\n", "a\nb\nY\nd\n", PairOutcome::Conflict),
        ] {
            let (a, b) = (lines(other), lines(approved));
            assert_eq!(compare(&base, &a, &b), expected);
            assert_eq!(compare(&base, &b, &a), expected, "symmetric");
        }
    }

    #[test]
    fn overlapping_deletions_with_different_extents_are_compatible() {
        let base = lines("a\nb\nc\nd\ne\n");
        assert_eq!(
            compare(&base, &lines("a\nd\ne\n"), &lines("a\nb\ne\n")),
            PairOutcome::Clean
        );
    }

    #[test]
    fn a_deleted_block_and_an_edit_inside_it_conflict() {
        let gone: String = (1..=12)
            .filter(|i| !(4..=7).contains(i))
            .map(|i| format!("line {i}\n"))
            .collect();
        let p = pair(&gone, &edit(12, &[(6, "Y")]));
        assert_eq!(p.outcome, PairOutcome::Conflict);
    }

    #[test]
    fn insertions_at_one_point_conflict_unless_identical() {
        let ins = |t: &str| -> String {
            let mut v: Vec<String> = (1..=12).map(|i| format!("line {i}\n")).collect();
            v.insert(6, format!("{t}\n"));
            v.concat()
        };
        assert_eq!(pair(&ins("A"), &ins("B")).outcome, PairOutcome::Conflict);
        assert_eq!(pair(&ins("A"), &ins("A")).outcome, PairOutcome::Clean);
    }

    #[test]
    fn an_insertion_beside_an_edited_line_is_a_different_line() {
        let mut ins: Vec<String> = (1..=12).map(|i| format!("line {i}\n")).collect();
        ins.insert(4, "new\n".to_owned()); // before line 5
        let p = pair(&ins.concat(), &edit(12, &[(5, "Y")]));
        assert_eq!(p.outcome, PairOutcome::Clean, "touching is not overlapping");
    }

    #[test]
    fn no_earlier_version_is_a_parallel_rewrite_not_a_conflict() {
        let mut s = Specs::new();
        s.store(2, &doc(5));
        let a = s.store(3, &edit(5, &[(1, "Y")]));
        s.approve(a);
        let r = s.run();
        assert_eq!(r.parallel_rewrites(), 1);
        assert_eq!((r.conflicts(), r.pairs[0].base), (0, None));
    }

    #[test]
    fn the_base_is_the_latest_version_stored_before_both() {
        let mut s = Specs::new();
        s.store(2, "old\n");
        s.store(3, &doc(12)); // the base
        s.store(4, &edit(12, &[(2, "X")])); // unused, stored first of the pair
        s.store(5, &edit(12, &[(2, "Z")])); // stored between, after the earlier of the pair
        let a = s.store(6, &edit(12, &[(2, "Y")]));
        s.approve(a);
        let r = s.run();
        let four = r.pairs.iter().find(|p| p.version == ArtId(4)).unwrap();
        assert_eq!(
            four.base,
            Some(ArtId(3)),
            "v5 is later than v4, so not a base for it"
        );
        assert_eq!(four.outcome, PairOutcome::Conflict);
        let five = r.pairs.iter().find(|p| p.version == ArtId(5)).unwrap();
        assert_eq!(five.base, Some(ArtId(4)));
    }

    #[test]
    fn no_approved_spec_means_nothing_to_compare() {
        let mut s = Specs::new();
        s.store(2, &doc(3));
        s.store(3, &doc(4));
        assert_eq!(s.run(), SpecConflicts::default());
    }

    #[test]
    fn a_missing_blob_is_unmeasured_not_guessed() {
        let mut s = Specs::new();
        s.store(2, &doc(5));
        s.store(3, &edit(5, &[(1, "X")]));
        let a = s.store(4, &edit(5, &[(1, "Y")]));
        s.approve(a);
        let first = s.blobs.keys().next().copied().unwrap();
        s.blobs.remove(&first);
        let r = s.run();
        assert!(r.unmeasured() >= 1, "{r:?}");
        assert_eq!(r.conflicts(), 0);
    }

    #[test]
    fn a_spec_beyond_the_exact_diff_limit_is_unmeasured() {
        let mut s = Specs::new();
        s.store(2, &doc(MAX_DIFF_INPUT_LEN));
        s.store(3, &edit(5, &[(1, "X")]));
        let a = s.store(4, &edit(5, &[(1, "Y")]));
        s.approve(a);
        let r = s.run();
        assert!(r.unmeasured() >= 1, "{r:?}");
    }

    #[test]
    fn regions_report_changed_lines_in_base_coordinates() {
        let base: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let new: Vec<String> = ["a", "X", "Y", "d", "e"].map(String::from).to_vec();
        let r = regions(&diff_myers(&base, &new));
        assert_eq!(
            r,
            vec![
                Region {
                    start: 1,
                    end: 3,
                    ins: vec!["X".into(), "Y".into()]
                },
                Region {
                    start: 4,
                    end: 4,
                    ins: vec!["e".into()]
                },
            ]
        );
    }
}
