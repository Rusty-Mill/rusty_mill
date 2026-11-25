//! Co-edited specs (event-based plan, step 4): a spec written as line edits
//! appended to the task log, and the one fold that turns them into text.
//!
//! Every line has a stable id, the version that inserted it and its index
//! there, and a deleted line stays as a tombstone. An op therefore names its
//! anchor by id rather than by position, and still applies on top of edits
//! made after its author read the base. A version's text is its predecessor's
//! with the edit's ops applied: inserted lines go directly after their anchor,
//! newest first. The fold is `rusty_crdt`'s [`Rga`]: each inserted line is an
//! element whose parent is its anchor (or the line before it in the same op),
//! and ids follow the log, so a new line is always the newest sibling. What a
//! CRDT could change beyond that is admission, and it does not: an edit is
//! still refused when it is anchored on a line that an edit made since its
//! base also anchored on ([`overlap`]).

use crate::command::{Code, Rejection};
use crate::ids::{ArtId, Sha256};
use rusty_crdt::{Applied, Op, Rga};
use rusty_serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// A line's identity: the version that inserted it and its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LineId {
    pub art: ArtId,
    pub n: u64,
}

/// One change to a co-edited spec, against lines its author saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineOp {
    /// Insert `lines` after `after`, or at the start when `None`.
    Insert {
        after: Option<LineId>,
        lines: Vec<String>,
    },
    Delete {
        line: LineId,
    },
}

impl LineOp {
    /// The line this op is placed against; `None` is the start of the spec.
    pub fn anchor(&self) -> Option<LineId> {
        match self {
            LineOp::Insert { after, .. } => *after,
            LineOp::Delete { line } => Some(*line),
        }
    }
}

/// The payload of a co-edited spec version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecEdit {
    /// The task brief, as a snapshot spec references it.
    pub brief: ArtId,
    /// The version its author read. `None` starts a new co-edited spec.
    pub base: Option<ArtId>,
    pub ops: Vec<LineOp>,
}

/// A co-edited spec's lines, tombstones included.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    rga: Rga<LineId, String>,
    /// The newest version that inserted a line. Derived from the lines, so
    /// equality need not compare it.
    newest: Option<ArtId>,
}

/// Equal when the lines, tombstones included, are equal in order.
impl PartialEq for Doc {
    fn eq(&self, other: &Self) -> bool {
        self.rga.elements().eq(other.rga.elements())
    }
}

impl Eq for Doc {}

impl Doc {
    /// The visible lines, in order.
    pub fn lines(&self) -> impl Iterator<Item = (LineId, &str)> {
        self.rga.iter().map(|(id, t)| (*id, t.as_str()))
    }

    /// The visible text, one line per visible line.
    pub fn text(&self) -> String {
        self.lines().map(|(_, t)| t).collect::<Vec<_>>().join("\n")
    }

    fn visible(&self, id: LineId) -> bool {
        self.lines().any(|(l, _)| l == id)
    }

    /// This doc with `ops` applied in order, inserted lines numbered under
    /// `at`. Fails with the first anchor that is not in the doc at all; a
    /// tombstoned anchor is fine. `at` must be newer than every version that
    /// inserted a line, as log order guarantees: the fold places a new line
    /// as its anchor's newest child, so an older `at` would land elsewhere
    /// than the edit meant, and an equal one would reuse line ids.
    pub fn apply(&self, at: ArtId, ops: &[LineOp]) -> Result<Doc, Unapplied> {
        if let Some(newest) = self.newest.filter(|n| at <= *n) {
            return Err(Unapplied::NotNewer { at, newest });
        }
        let mut doc = self.clone();
        let mut n = 0;
        for op in ops {
            match op {
                LineOp::Insert { after, lines } => {
                    // Checked here too, so an insert with no lines still
                    // fails on a missing anchor, as the splice fold did.
                    if let Some(a) = after.filter(|a| !doc.rga.contains(a)) {
                        return Err(Unapplied::Line(a));
                    }
                    let mut parent = *after;
                    for text in lines {
                        let id = LineId { art: at, n };
                        n += 1;
                        let value = text.clone();
                        doc.put(id, Op::Insert { id, parent, value })?;
                        doc.newest = Some(at);
                        parent = Some(id);
                    }
                }
                LineOp::Delete { line } => doc.put(*line, Op::Delete { target: *line })?,
            }
        }
        Ok(doc)
    }

    /// Apply one element op. `key` is the line it inserts or deletes. An
    /// insert must add a new line; `apply`'s check on `at` already rules out
    /// a reused one, which would fail here as not newer. Deleting a
    /// tombstone again is fine.
    fn put(&mut self, key: LineId, op: Op<LineId, String>) -> Result<(), Unapplied> {
        let insert = matches!(op, Op::Insert { .. });
        match self.rga.apply(&op) {
            Ok(Applied::Changed) => Ok(()),
            Ok(Applied::Duplicate) if !insert => Ok(()),
            Ok(Applied::Pending { missing }) => Err(Unapplied::Line(missing)),
            Ok(Applied::Duplicate) | Err(_) => Err(Unapplied::NotNewer {
                at: key.art,
                newest: key.art,
            }),
        }
    }
}

/// Refuse an edit that is malformed against the version it names as its base.
pub fn validate(base: &Doc, ops: &[LineOp]) -> Result<(), Rejection> {
    let bad = |detail: &str| Err(Rejection::new(Code::BadEdit, detail));
    if ops.is_empty() {
        return bad("an edit needs at least one op");
    }
    for op in ops {
        if let Some(a) = op.anchor().filter(|a| !base.visible(*a)) {
            return Err(Rejection::new(
                Code::RefUnresolved,
                format!("line {}#{} is not visible in the base", a.art, a.n),
            ));
        }
        let LineOp::Insert { lines, .. } = op else {
            continue;
        };
        if lines.is_empty() {
            return bad("an insert needs at least one line");
        }
        if lines.iter().any(|l| l.contains(['\n', '\r'])) {
            return bad("a line holds no line break");
        }
    }
    Ok(())
}

/// The anchors `ops` share with the edits accepted since its base (`since`,
/// their anchors), `None` being the start of the spec. Empty means no op is
/// placed against a line that someone else changed or placed lines against
/// after its author read the base, so the edit applies to the head as meant.
pub fn overlap(ops: &[LineOp], since: &[Option<LineId>]) -> Vec<Option<LineId>> {
    let theirs: HashSet<Option<LineId>> = since.iter().copied().collect();
    let mut out: Vec<Option<LineId>> = ops
        .iter()
        .map(LineOp::anchor)
        .filter(|a| theirs.contains(a))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Why a logged co-edited version did not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unapplied {
    /// An op's anchor is in no earlier version.
    Line(LineId),
    /// Its base is not an indexed version.
    Base(ArtId),
    /// Its id `at` is not newer than `newest`, the newest version that
    /// inserted a line: only a hand-made or out-of-order log does this.
    NotNewer { at: ArtId, newest: ArtId },
}

impl std::fmt::Display for Unapplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unapplied::Line(l) => write!(f, "line {}#{} is missing", l.art, l.n),
            Unapplied::Base(b) => write!(f, "base {b} is missing"),
            Unapplied::NotNewer { at, newest } => {
                write!(
                    f,
                    "{at} is not newer than {newest}, which already added lines"
                )
            }
        }
    }
}

/// One accepted version of a co-edited spec, as the task state indexes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// The spec's first version; every version of one spec shares it.
    pub root: ArtId,
    /// The version this one was applied on: the head when it was accepted,
    /// which is its author's base or a later version.
    pub prev: Option<ArtId>,
    pub doc: Arc<Doc>,
    /// SHA-256 of `doc`'s visible text: the bytes an approval of this
    /// version judged (2b, d). Equal text gives an equal snapshot.
    pub snapshot: Sha256,
    /// The anchors of this version's ops.
    pub anchors: Arc<[Option<LineId>]>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn at(n: u64) -> ArtId {
        ArtId(n)
    }

    fn ins(after: Option<LineId>, lines: &[&str]) -> LineOp {
        LineOp::Insert {
            after,
            lines: lines.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn id(art: u64, n: u64) -> LineId {
        LineId { art: ArtId(art), n }
    }

    /// `# A`, `a1`, `a2`, `# B`, `b1`, all under art 2.
    fn two_sections() -> Doc {
        Doc::default()
            .apply(at(2), &[ins(None, &["# A", "a1", "a2", "# B", "b1"])])
            .unwrap()
    }

    #[test]
    fn inserts_go_right_after_their_anchor_newest_first() {
        let d = two_sections();
        let d = d.apply(at(3), &[ins(Some(id(2, 1)), &["x"])]).unwrap();
        let d = d.apply(at(4), &[ins(Some(id(2, 1)), &["y"])]).unwrap();
        assert_eq!(d.text(), "# A\na1\ny\nx\na2\n# B\nb1");
    }

    #[test]
    fn a_tombstoned_anchor_still_places_an_insert() {
        let d = two_sections();
        let d = d
            .apply(at(3), &[LineOp::Delete { line: id(2, 1) }])
            .unwrap();
        let d = d.apply(at(4), &[ins(Some(id(2, 1)), &["x"])]).unwrap();
        assert_eq!(d.text(), "# A\nx\na2\n# B\nb1");
        assert_eq!(
            d.apply(at(5), &[ins(Some(id(9, 0)), &["?"])]),
            Err(Unapplied::Line(id(9, 0)))
        );
    }

    #[test]
    fn a_reused_line_id_fails_even_when_the_insert_is_identical() {
        let d = two_sections();
        let not_newer = Err(Unapplied::NotNewer {
            at: at(2),
            newest: at(2),
        });
        let same = [ins(None, &["# A", "a1", "a2", "# B", "b1"])];
        assert_eq!(d.apply(at(2), &same), not_newer);
        let other = [ins(Some(id(2, 4)), &["new"])];
        assert_eq!(d.apply(at(2), &other), not_newer);
    }

    #[test]
    fn an_older_version_id_is_refused_not_placed_elsewhere() {
        // (5, 0) is after a1; an edit numbered 3 placing a line after a1
        // would land after (5, 0) in the RGA, where the edit did not mean.
        let d = two_sections();
        let d = d.apply(at(5), &[ins(Some(id(2, 1)), &["new"])]).unwrap();
        let older = [ins(Some(id(2, 1)), &["old"])];
        assert_eq!(
            d.apply(at(3), &older),
            Err(Unapplied::NotNewer {
                at: at(3),
                newest: at(5)
            })
        );
        assert!(d.apply(at(6), &older).is_ok());
    }

    #[test]
    fn a_delete_only_version_does_not_raise_the_floor() {
        let d = two_sections();
        let d = d
            .apply(at(9), &[LineOp::Delete { line: id(2, 1) }])
            .unwrap();
        assert!(d.apply(at(3), &[ins(Some(id(2, 0)), &["x"])]).is_ok());
    }

    #[test]
    fn unapplied_names_what_went_wrong() {
        assert_eq!(
            Unapplied::Line(id(2, 1)).to_string(),
            "line art:2#1 is missing"
        );
        assert_eq!(
            Unapplied::NotNewer {
                at: at(3),
                newest: at(5)
            }
            .to_string(),
            "art:3 is not newer than art:5, which already added lines"
        );
    }

    #[test]
    fn deleting_a_tombstone_again_is_fine() {
        let d = two_sections();
        let del = [LineOp::Delete { line: id(2, 1) }];
        let once = d.apply(at(3), &del).unwrap();
        let twice = once.apply(at(4), &del).unwrap();
        assert_eq!(twice, once);
        let both = d.apply(at(3), &[del[0].clone(), del[0].clone()]).unwrap();
        assert_eq!(both, once);
    }

    #[test]
    fn an_empty_insert_still_fails_on_a_missing_anchor() {
        let d = two_sections();
        assert_eq!(
            d.apply(at(3), &[ins(Some(id(9, 0)), &[])]),
            Err(Unapplied::Line(id(9, 0)))
        );
        assert_eq!(d.apply(at(3), &[ins(Some(id(2, 1)), &[])]), Ok(d.clone()));
        assert_eq!(d.apply(at(3), &[ins(None, &[])]), Ok(d));
    }

    #[test]
    fn validate_refuses_empty_edits_hidden_anchors_and_line_breaks() {
        let d = two_sections();
        let code = |ops: &[LineOp], d: &Doc| validate(d, ops).map_err(|r| r.code);
        assert_eq!(code(&[], &d), Err(Code::BadEdit));
        assert_eq!(code(&[ins(None, &[])], &d), Err(Code::BadEdit));
        assert_eq!(code(&[ins(None, &["a\nb"])], &d), Err(Code::BadEdit));
        let unknown = [LineOp::Delete { line: id(7, 0) }];
        assert_eq!(code(&unknown, &d), Err(Code::RefUnresolved));
        let gone = d
            .apply(at(3), &[LineOp::Delete { line: id(2, 1) }])
            .unwrap();
        let after_gone = [ins(Some(id(2, 1)), &["x"])];
        assert_eq!(code(&after_gone, &gone), Err(Code::RefUnresolved));
        assert!(validate(&d, &[ins(Some(id(2, 4)), &["b2"])]).is_ok());
    }

    #[test]
    fn edits_overlap_only_on_a_shared_line() {
        let since = [Some(id(2, 1))];
        let same_section = [LineOp::Delete { line: id(2, 2) }];
        assert!(overlap(&same_section, &since).is_empty());
        let same_line = [LineOp::Delete { line: id(2, 1) }];
        assert_eq!(overlap(&same_line, &since), [Some(id(2, 1))]);
        let after_it = [ins(Some(id(2, 1)), &["y"])];
        assert_eq!(overlap(&after_it, &since), [Some(id(2, 1))]);
        assert!(overlap(&same_line, &[]).is_empty(), "nothing since");
    }

    #[test]
    fn the_start_of_the_spec_is_a_shared_anchor_too() {
        let other_intro = [ins(None, &["hello"])];
        assert_eq!(overlap(&other_intro, &[None]), [None]);
        assert!(overlap(&other_intro, &[Some(id(2, 0))]).is_empty());
    }
}
