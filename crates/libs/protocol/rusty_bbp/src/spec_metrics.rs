//! Spec-authoring metrics for the 2b gate (`docs/bbp-concurrency-design.md`,
//! "2b gate criteria"): pure folds over the log and the stored spec bytes.
//! They change no rule and add no event.
//!
//! [`spec_attribution`] backs criterion 5 (what was lost) and, with
//! [`unused_conflicts`], criterion 4's "unused".
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
use std::collections::HashSet;

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

/// The spec versions in storage order, and the position of the one approved at
/// the plan gate (the last approval). `None` when nothing was approved.
fn scan(events: &[Event]) -> Option<(Vec<(ArtId, BlobRef)>, usize)> {
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
    let at = versions.iter().position(|(id, _)| Some(*id) == approved)?;
    Some((versions, at))
}

/// A stored spec's lines, or `None` if it cannot be read or decoded.
fn spec_lines(blob: &dyn Fn(&BlobRef) -> Option<Vec<u8>>, at: &BlobRef) -> Option<Vec<String>> {
    Some(lines(&spec_markdown(&blob(at)?).ok()?))
}

/// Compare every non-approved spec version with the approved one. `blob`
/// returns a stored spec's bytes; the core stays free of I/O.
pub fn spec_conflicts(
    events: &[Event],
    blob: &dyn Fn(&BlobRef) -> Option<Vec<u8>>,
) -> SpecConflicts {
    let Some((versions, a_at)) = scan(events) else {
        return SpecConflicts::default();
    };
    let approved = versions[a_at].0;
    let text = |at: usize| spec_lines(blob, &versions[at].1);
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

/// What became of a non-approved spec version's contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribution {
    /// At least half of its changed lines are in the approved spec: its work
    /// was carried forward, not lost.
    Retained,
    /// Fewer than half are: most of what it contributed was thrown away.
    Unused,
    /// It changed nothing relative to the version before it.
    Unchanged,
    /// Unreadable, beyond the exact diff limit, or stored after the approval.
    Unmeasured,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionAttribution {
    pub version: ArtId,
    /// The version it is measured against: the one stored just before it,
    /// which its author could have read. `None` for the first (empty base).
    pub base: Option<ArtId>,
    /// Non-blank lines it contributed relative to the base: the lines it
    /// added, or, when it only deleted, the lines it removed.
    pub changed: u64,
    /// Of those, the ones the approved spec agrees with: an added line the
    /// approved spec has, or (for a pure deletion) a line the approved spec
    /// also lacks. A rewrite counts by its new text, so a line replaced with
    /// something else in the approved spec is not carried just because both
    /// removed the old one.
    pub carried: u64,
    /// Stored size of the version, for `unused_share`.
    pub bytes: u64,
    pub outcome: Attribution,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecAttribution {
    pub approved: Option<ArtId>,
    /// Stored size of the approved version.
    pub approved_bytes: u64,
    /// Every non-approved spec version, in storage order.
    pub versions: Vec<VersionAttribution>,
}

impl SpecAttribution {
    /// `unused_spec_versions` of the 2b gate.
    pub fn unused_versions(&self) -> u64 {
        self.count(Attribution::Unused)
    }
    /// Bytes of the unused versions and of every stored spec version: the
    /// numerator and denominator of `unused_share` (kept apart so a reader can
    /// take the median over tasks without a float in the log).
    pub fn unused_bytes(&self) -> (u64, u64) {
        let unused = self
            .versions
            .iter()
            .filter(|v| v.outcome == Attribution::Unused)
            .map(|v| v.bytes);
        let all = self.versions.iter().map(|v| v.bytes);
        (unused.sum(), all.sum::<u64>() + self.approved_bytes)
    }
    pub fn count(&self, o: Attribution) -> u64 {
        self.versions.iter().filter(|v| v.outcome == o).count() as u64
    }
    pub fn is_unused(&self, version: ArtId) -> bool {
        self.versions
            .iter()
            .any(|v| v.version == version && v.outcome == Attribution::Unused)
    }
}

/// The unused versions that also conflict with the approved spec: the
/// quantity 2b criterion 4 asks for ("an unused spec version has a conflict").
pub fn unused_conflicts(conflicts: &SpecConflicts, attribution: &SpecAttribution) -> u64 {
    conflicts
        .pairs
        .iter()
        .filter(|p| p.outcome == PairOutcome::Conflict && attribution.is_unused(p.version))
        .count() as u64
}

/// The version's contribution as `(changed, carried)`: its added non-blank
/// lines and how many the approved spec has; or, if it added none, its removed
/// non-blank lines and how many the approved spec also lacks. Lines are
/// compared without trailing whitespace.
fn contribution(approved: &HashSet<String>, ops: &[DiffOp<String>]) -> (u64, u64) {
    let pick = |want_insert: bool| -> Vec<&str> {
        ops.iter()
            .filter_map(|o| match (o, want_insert) {
                (DiffOp::Insert(l), true) | (DiffOp::Delete(l), false) => Some(l.trim_end()),
                _ => None,
            })
            .filter(|l| !l.trim().is_empty())
            .collect()
    };
    let added = pick(true);
    if !added.is_empty() {
        let carried = added.iter().filter(|l| approved.contains(**l)).count();
        return (added.len() as u64, carried as u64);
    }
    let removed = pick(false);
    let carried = removed.iter().filter(|l| !approved.contains(**l)).count();
    (removed.len() as u64, carried as u64)
}

/// For every non-approved spec version, how much of what it changed (against
/// the version stored before it) the approved spec kept.
pub fn spec_attribution(
    events: &[Event],
    blob: &dyn Fn(&BlobRef) -> Option<Vec<u8>>,
) -> SpecAttribution {
    let Some((versions, a_at)) = scan(events) else {
        return SpecAttribution::default();
    };
    let Some(approved) = spec_lines(blob, &versions[a_at].1) else {
        return SpecAttribution {
            approved: Some(versions[a_at].0),
            approved_bytes: versions[a_at].1.len,
            versions: versions
                .iter()
                .enumerate()
                .filter(|(at, _)| *at != a_at)
                .map(|(_, (id, bref))| unmeasured(*id, None, bref.len))
                .collect(),
        };
    };
    let approved_set: HashSet<String> = approved.iter().map(|l| l.trim_end().to_owned()).collect();
    let mut out = SpecAttribution {
        approved: Some(versions[a_at].0),
        approved_bytes: versions[a_at].1.len,
        versions: Vec::new(),
    };
    for (at, (id, bref)) in versions.iter().enumerate() {
        if at == a_at {
            continue;
        }
        let base_at = at.checked_sub(1);
        let base_id = base_at.map(|b| versions[b].0);
        if at > a_at {
            out.versions.push(unmeasured(*id, base_id, bref.len));
            continue;
        }
        let base = match base_at {
            None => Some(Vec::new()),
            Some(b) => spec_lines(blob, &versions[b].1),
        };
        let (Some(base), Some(text)) = (base, spec_lines(blob, bref)) else {
            out.versions.push(unmeasured(*id, base_id, bref.len));
            continue;
        };
        if base.len() + text.len() > MAX_DIFF_INPUT_LEN {
            out.versions.push(unmeasured(*id, base_id, bref.len));
            continue;
        }
        let ops = diff_myers(&base, &text);
        if degraded(&base, &text, &ops) {
            out.versions.push(unmeasured(*id, base_id, bref.len));
            continue;
        }
        let (changed, carried) = contribution(&approved_set, &ops);
        let outcome = match (changed, carried * 2 >= changed) {
            (0, _) => Attribution::Unchanged,
            (_, true) => Attribution::Retained,
            (_, false) => Attribution::Unused,
        };
        out.versions.push(VersionAttribution {
            version: *id,
            base: base_id,
            changed,
            carried,
            bytes: bref.len,
            outcome,
        });
    }
    out
}

fn unmeasured(version: ArtId, base: Option<ArtId>, bytes: u64) -> VersionAttribution {
    VersionAttribution {
        version,
        base,
        changed: 0,
        carried: 0,
        bytes,
        outcome: Attribution::Unmeasured,
    }
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

    // ---- attribution

    impl Specs {
        fn attribution(&self) -> SpecAttribution {
            spec_attribution(&self.events, &|b| self.blobs.get(&b.sha).cloned())
        }
    }

    /// `base`, then `unused` (changes some lines), then `approved`.
    fn attributed(unused: &str, approved: &str) -> VersionAttribution {
        let mut s = Specs::new();
        s.store(2, &doc(10));
        s.store(3, unused);
        let a = s.store(4, approved);
        s.approve(a);
        let r = s.attribution();
        assert_eq!(r.approved, Some(ArtId(4)));
        r.versions
            .into_iter()
            .find(|v| v.version == ArtId(3))
            .unwrap()
    }

    #[test]
    fn work_carried_into_the_approved_spec_is_retained() {
        // v3 rewrites two lines; the approved spec keeps both and adds one.
        let v = attributed(
            &edit(10, &[(2, "A"), (5, "B")]),
            &edit(10, &[(2, "A"), (5, "B"), (9, "C")]),
        );
        // Two lines added; the approved spec has both.
        assert_eq!(
            (v.changed, v.carried, v.outcome),
            (2, 2, Attribution::Retained)
        );
        assert_eq!(v.base, Some(ArtId(2)));
    }

    #[test]
    fn work_the_approved_spec_dropped_is_unused() {
        let v = attributed(&edit(10, &[(2, "A"), (5, "B")]), &edit(10, &[(9, "C")]));
        // The approved spec has neither added line. Removals are not counted:
        // a rewrite of the same line would otherwise look half-retained.
        assert_eq!(
            (v.changed, v.carried, v.outcome),
            (2, 0, Attribution::Unused)
        );
    }

    #[test]
    fn exactly_half_carried_is_retained_and_less_is_unused() {
        // 2 added lines, 1 carried is half (retained).
        let half = attributed(
            &edit(10, &[(2, "A"), (5, "B")]),
            &edit(10, &[(2, "A"), (5, "other")]),
        );
        assert_eq!((half.changed, half.carried), (2, 1));
        assert_eq!(half.outcome, Attribution::Retained);
        // 3 added lines, 1 carried is under half (unused).
        let under = attributed(
            &edit(10, &[(2, "A"), (5, "B"), (8, "C")]),
            &edit(10, &[(2, "A"), (5, "x"), (8, "y")]),
        );
        assert_eq!((under.changed, under.carried), (3, 1));
        assert_eq!(under.outcome, Attribution::Unused);
    }

    #[test]
    fn a_removed_line_is_carried_only_if_the_approved_spec_also_lacks_it() {
        let gone: String = (1..=10)
            .filter(|i| *i != 4)
            .map(|i| format!("line {i}\n"))
            .collect();
        // The approved spec also lacks line 4: the removal is carried.
        let kept = attributed(&gone, &gone);
        assert_eq!(
            (kept.changed, kept.carried, kept.outcome),
            (1, 1, Attribution::Retained)
        );
        // The approved spec restores it: the removal is lost.
        let lost = attributed(&gone, &doc(10));
        assert_eq!(
            (lost.changed, lost.carried, lost.outcome),
            (1, 0, Attribution::Unused)
        );
    }

    #[test]
    fn unused_share_is_unused_bytes_over_all_spec_bytes() {
        let mut s = Specs::new();
        s.store(2, &doc(10));
        let dropped = edit(10, &[(2, "A"), (5, "B")]);
        s.store(3, &dropped);
        let a = s.store(4, &doc(10));
        s.approve(a);
        let r = s.attribution();
        assert_eq!(r.unused_versions(), 1);
        let (unused, all) = r.unused_bytes();
        let n = |t: &str| {
            let payload = ArtifactPayload::Spec { brief: ArtId(1) };
            encode_artifact(&payload, t.as_bytes()).unwrap().len() as u64
        };
        assert_eq!(unused, n(&dropped));
        assert_eq!(all, n(&doc(10)) * 2 + n(&dropped));
    }

    #[test]
    fn a_version_that_changes_nothing_is_unchanged_not_unused() {
        let v = attributed(&doc(10), &edit(10, &[(1, "Z")]));
        assert_eq!((v.changed, v.outcome), (0, Attribution::Unchanged));
    }

    #[test]
    fn blank_lines_carry_no_work() {
        let v = attributed(&format!("{}\n\n\n", doc(10)), &doc(10));
        assert_eq!((v.changed, v.outcome), (0, Attribution::Unchanged));
    }

    #[test]
    fn the_first_version_is_measured_against_an_empty_base() {
        let mut s = Specs::new();
        s.store(2, &doc(4)); // first draft: every line is its work
        let a = s.store(3, &doc(4)); // approved keeps all of it
        s.approve(a);
        let v = &s.attribution().versions[0];
        assert_eq!((v.base, v.changed, v.carried), (None, 4, 4));
        assert_eq!(v.outcome, Attribution::Retained);
    }

    #[test]
    fn a_version_stored_after_the_approval_is_unmeasured() {
        let mut s = Specs::new();
        s.store(2, &doc(4));
        let a = s.store(3, &doc(5));
        s.approve(a);
        s.store(4, &doc(6));
        let r = s.attribution();
        let late = r.versions.iter().find(|v| v.version == ArtId(4)).unwrap();
        assert_eq!(late.outcome, Attribution::Unmeasured);
    }

    #[test]
    fn a_missing_blob_is_unmeasured_and_nothing_is_approved_means_nothing_to_attribute() {
        let mut s = Specs::new();
        s.store(2, &doc(4));
        let a = s.store(3, &doc(5));
        assert_eq!(
            s.attribution(),
            SpecAttribution::default(),
            "no approval yet"
        );
        s.approve(a);
        let first = Sha256::of(
            &encode_artifact(
                &ArtifactPayload::Spec { brief: ArtId(1) },
                doc(4).as_bytes(),
            )
            .unwrap(),
        );
        s.blobs.remove(&first);
        assert_eq!(s.attribution().versions[0].outcome, Attribution::Unmeasured);
    }

    #[test]
    fn unused_conflicts_count_versions_that_are_both() {
        // v3 conflicts with the approved spec and its work was dropped; v5 conflicts but was retained.
        let mut s = Specs::new();
        s.store(2, &doc(10));
        s.store(3, &edit(10, &[(4, "dropped")]));
        s.store(5, &edit(10, &[(7, "kept")]));
        let a = s.store(6, &edit(10, &[(4, "final"), (7, "kept")]));
        s.approve(a);
        let (c, at) = (s.run(), s.attribution());
        let outcome = |id| {
            c.pairs
                .iter()
                .find(|p| p.version == ArtId(id))
                .unwrap()
                .outcome
        };
        // v5's base is v3 here (the latest stored before both v5 and v6).
        assert_eq!(outcome(3), PairOutcome::Conflict);
        assert!(at.is_unused(ArtId(3)));
        assert_eq!(
            unused_conflicts(&c, &at),
            c.pairs
                .iter()
                .filter(|p| { p.outcome == PairOutcome::Conflict && at.is_unused(p.version) })
                .count() as u64
        );
        assert!(unused_conflicts(&c, &at) >= 1);
        assert!(!at.is_unused(ArtId(5)), "v5's edit is in the approved spec");
    }
}
