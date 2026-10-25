//! Co-edited specs (event-based plan, step 4): a spec written as line edits
//! appended to the task log, and the one fold that turns them into text.
//!
//! Every line has a stable id, the version that inserted it and its index
//! there, and a deleted line stays as a tombstone. An op therefore names its
//! anchor by id rather than by position, and still applies on top of edits
//! made after its author read the base. A version's text is its predecessor's
//! with the edit's ops applied: inserted lines go directly after their anchor,
//! newest first. That is RGA's order whenever ids follow the log (a new line
//! is always the newest sibling), so this fold is already the CRDT fold. What
//! a CRDT would change is admission: today an edit is refused when it touches
//! a section that changed since its base ([`overlap`]); a CRDT accepts it.
//!
//! A section is a heading line (text starting with `#`) and the lines up to
//! the next heading; lines before the first heading are the preamble, `None`.

use crate::command::{Code, Rejection};
use crate::ids::ArtId;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    id: LineId,
    text: String,
    live: bool,
}

impl Line {
    fn heading(&self) -> bool {
        self.live && self.text.starts_with('#')
    }
}

/// A co-edited spec's lines, tombstones included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    lines: Vec<Line>,
}

impl Doc {
    /// The visible lines, in order.
    pub fn lines(&self) -> impl Iterator<Item = (LineId, &str)> {
        self.lines
            .iter()
            .filter(|l| l.live)
            .map(|l| (l.id, l.text.as_str()))
    }

    /// The visible text, one line per visible line.
    pub fn text(&self) -> String {
        self.lines().map(|(_, t)| t).collect::<Vec<_>>().join("\n")
    }

    fn visible(&self, id: LineId) -> bool {
        self.lines.iter().any(|l| l.id == id && l.live)
    }

    fn position(&self, id: LineId) -> Option<usize> {
        self.lines.iter().position(|l| l.id == id)
    }

    /// This doc with `ops` applied in order, inserted lines numbered under
    /// `at`. Fails with the first anchor that is not in the doc at all; a
    /// tombstoned anchor is fine.
    pub fn apply(&self, at: ArtId, ops: &[LineOp]) -> Result<Doc, LineId> {
        let mut doc = self.clone();
        let mut n = 0;
        for op in ops {
            match op {
                LineOp::Insert { after, lines } => {
                    let start = match after {
                        None => 0,
                        Some(a) => doc.position(*a).ok_or(*a)? + 1,
                    };
                    let new = lines.iter().map(|text| {
                        n += 1;
                        Line {
                            id: LineId { art: at, n: n - 1 },
                            text: text.clone(),
                            live: true,
                        }
                    });
                    doc.lines.splice(start..start, new.collect::<Vec<_>>());
                }
                LineOp::Delete { line } => {
                    let i = doc.position(*line).ok_or(*line)?;
                    doc.lines[i].live = false;
                }
            }
        }
        Ok(doc)
    }

    /// Each line's section, as `base` divides this doc: the nearest heading
    /// before it that is visible in `base`. This doc must extend `base` (hold
    /// its lines in the same order), as a later version always does.
    fn sections_as(&self, base: &Doc) -> HashMap<LineId, Option<LineId>> {
        let headings: HashSet<LineId> = base
            .lines
            .iter()
            .filter(|l| l.heading())
            .map(|l| l.id)
            .collect();
        let mut current = None;
        let mut out = HashMap::with_capacity(self.lines.len());
        for l in &self.lines {
            if headings.contains(&l.id) {
                current = Some(l.id);
            }
            out.insert(l.id, current);
        }
        out
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

/// The sections `ops` touch that the edits accepted since its base also
/// touched (`since`, their anchors), as `base` divides `head`. Empty means
/// the edit applies to `head` without overlapping anything its author did
/// not see. Measuring both sides against one division means two edits of
/// the same line always overlap, whatever headings moved in between.
pub fn overlap(
    head: &Doc,
    base: &Doc,
    ops: &[LineOp],
    since: &[Option<LineId>],
) -> Vec<Option<LineId>> {
    if since.is_empty() {
        return Vec::new();
    }
    let section = head.sections_as(base);
    let of = |a: Option<LineId>| a.and_then(|id| section.get(&id).copied().flatten());
    let theirs: HashSet<Option<LineId>> = since.iter().map(|a| of(*a)).collect();
    let mut out: Vec<Option<LineId>> = ops
        .iter()
        .map(|op| of(op.anchor()))
        .filter(|s| theirs.contains(s))
        .collect();
    out.sort();
    out.dedup();
    out
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
            Err(id(9, 0))
        );
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
    fn edits_overlap_only_in_a_shared_section() {
        let base = two_sections();
        let in_a = [ins(Some(id(2, 1)), &["x"])];
        let head = base.apply(at(3), &in_a).unwrap();
        let since = [Some(id(2, 1))];
        let in_b = [LineOp::Delete { line: id(2, 4) }];
        assert!(overlap(&head, &base, &in_b, &since).is_empty());
        let also_a = [LineOp::Delete { line: id(2, 2) }];
        assert_eq!(overlap(&head, &base, &also_a, &since), [Some(id(2, 0))]);
        assert!(
            overlap(&head, &base, &also_a, &[]).is_empty(),
            "nothing since"
        );
    }

    #[test]
    fn the_preamble_is_a_section_too() {
        let base = two_sections();
        let head = base.apply(at(3), &[ins(None, &["intro"])]).unwrap();
        let since = [None];
        let other_intro = [ins(None, &["hello"])];
        assert_eq!(overlap(&head, &base, &other_intro, &since), [None]);
    }

    #[test]
    fn a_moved_heading_cannot_hide_a_same_line_overlap() {
        // v3 splits section A with a new heading before a2; v4 (base v3) then
        // deletes a2, which v3's base would place in A and v4's base in `# C`.
        // An editor on base v2 also deleting a2 must still be refused.
        let base = two_sections();
        let v3 = base.apply(at(3), &[ins(Some(id(2, 1)), &["# C"])]).unwrap();
        let v4 = v3
            .apply(at(4), &[LineOp::Delete { line: id(2, 2) }])
            .unwrap();
        let since = [Some(id(2, 1)), Some(id(2, 2))];
        let same = [LineOp::Delete { line: id(2, 2) }];
        assert!(!overlap(&v4, &base, &same, &since).is_empty());
    }
}
