//! `ADR-0133` (`STC-FR-001`): the **strict commit** check. A strict session's
//! `Commit` applies its ordered `WriteOp` list all or nothing, soft outcomes
//! included: `first_soft_failure` walks the list against the locked store's
//! current records plus an in-batch overlay (what earlier ops in the same list
//! inserted, replaced, updated, deleted or linked) and names the first op that
//! would be `Duplicate`, `NotFound`, `GuardFailed` or `AlreadyLinked` — before
//! anything is written.
//!
//! Pure over two read closures, so `Memory`, `Entity` and `Relation` share one
//! definition and each runs it inside its own exclusive section, where the
//! answer cannot go stale. It must predict exactly what the apply step would
//! answer; a property test in `memory.rs` runs random op lists through both.
//! The same overlay answers a strict session's `GetById` (`overlay_get`).

use super::protocol::{predicate_matches, ErrorCode, FieldRef, RecordId, ScanValue, WriteOp};
use std::collections::{HashMap, HashSet};

/// A record's fields, as the adapters' `get` returns them.
pub(crate) type Fields = Vec<(FieldRef, ScanValue)>;

/// The store as strict-checked ops see it: stored records, plus the batch's
/// own effect so far.
pub(crate) struct Overlay<'a> {
    get: &'a dyn Fn(RecordId) -> Option<Fields>,
    linked: &'a dyn Fn(RecordId, RecordId, &str) -> bool,
    /// `Some(fields)` — written by the batch; `None` — deleted by it.
    records: HashMap<RecordId, Option<Fields>>,
    /// Edges the batch added, endpoints ordered.
    added: HashSet<(RecordId, RecordId, String)>,
    /// Ids the batch deleted: their stored edges are gone for good.
    fresh: HashSet<RecordId>,
}

fn edge(left: RecordId, right: RecordId, relation: &str) -> (RecordId, RecordId, String) {
    (left.min(right), left.max(right), relation.to_string())
}

impl<'a> Overlay<'a> {
    pub(crate) fn new(
        get: &'a dyn Fn(RecordId) -> Option<Fields>,
        linked: &'a dyn Fn(RecordId, RecordId, &str) -> bool,
    ) -> Self {
        Self {
            get,
            linked,
            records: HashMap::new(),
            added: HashSet::new(),
            fresh: HashSet::new(),
        }
    }

    /// The record as the batch has left it so far.
    pub(crate) fn current(&self, id: RecordId) -> Option<Fields> {
        match self.records.get(&id) {
            Some(written) => written.clone(),
            None => (self.get)(id),
        }
    }

    fn edge_exists(&self, left: RecordId, right: RecordId, relation: &str) -> bool {
        self.added.contains(&edge(left, right, relation))
            || (!self.fresh.contains(&left)
                && !self.fresh.contains(&right)
                && (self.linked)(left, right, relation))
    }

    /// Fold one op in; the soft failure it would have, if any. A failed op
    /// changes nothing (the caller stops at the first).
    pub(crate) fn step(&mut self, op: &WriteOp) -> Option<ErrorCode> {
        match op {
            WriteOp::Insert { id, fields } => {
                if self.current(*id).is_some() {
                    return Some(ErrorCode::Duplicate);
                }
                self.records.insert(*id, Some(fields.clone()));
            }
            WriteOp::Replace { id, fields } => {
                if self.current(*id).is_none() {
                    return Some(ErrorCode::RecordNotFound);
                }
                self.records.insert(*id, Some(fields.clone()));
            }
            WriteOp::ReplaceIf { id, fields, guard } => {
                let Some(stored) = self.current(*id) else {
                    return Some(ErrorCode::RecordNotFound);
                };
                if !predicate_matches(&stored, guard) {
                    return Some(ErrorCode::GuardFailed);
                }
                self.records.insert(*id, Some(fields.clone()));
            }
            WriteOp::UpdateField { id, field, value } => {
                let Some(mut stored) = self.current(*id) else {
                    return Some(ErrorCode::RecordNotFound);
                };
                match stored.iter_mut().find(|(tag, _)| tag == field) {
                    Some(slot) => slot.1 = value.clone(),
                    None => stored.push((*field, value.clone())),
                }
                self.records.insert(*id, Some(stored));
            }
            WriteOp::Delete { id } => {
                if self.current(*id).is_none() {
                    return Some(ErrorCode::RecordNotFound);
                }
                self.records.insert(*id, None);
                self.fresh.insert(*id);
                self.added.retain(|(l, r, _)| l != id && r != id);
            }
            WriteOp::Link {
                left,
                right,
                relation,
            } => {
                if self.current(*left).is_none() {
                    return Some(ErrorCode::RecordNotFound);
                }
                // A self-loop is a hard `Malformed` at apply time; a strict
                // batch must refuse it before writing anything.
                if left == right {
                    return Some(ErrorCode::Malformed);
                }
                if self.edge_exists(*left, *right, relation) {
                    return Some(ErrorCode::Duplicate);
                }
                self.added.insert(edge(*left, *right, relation));
            }
        }
        None
    }
}

/// The first op of `ops` that would end soft, with its code; `None` when the
/// whole list would apply cleanly.
pub(crate) fn first_soft_failure(
    ops: &[WriteOp],
    get: &dyn Fn(RecordId) -> Option<Fields>,
    linked: &dyn Fn(RecordId, RecordId, &str) -> bool,
) -> Option<(usize, ErrorCode)> {
    let mut overlay = Overlay::new(get, linked);
    ops.iter()
        .enumerate()
        .find_map(|(i, op)| overlay.step(op).map(|code| (i, code)))
}

/// `record` (`id`'s stored fields, or `None`) as `staged` would leave it — the
/// answer to a strict session's `GetById` (`STC-FR-004`). Soft-failing ops are
/// skipped here: the commit will refuse the batch, the read just shows what
/// would have applied.
pub(crate) fn overlay_get(
    id: RecordId,
    stored: Option<Fields>,
    staged: &[WriteOp],
) -> Option<Fields> {
    let only = |other: RecordId| (other == id).then(|| stored.clone()).flatten();
    let no_edges = |_: RecordId, _: RecordId, _: &str| false;
    let mut overlay = Overlay::new(&only, &no_edges);
    for op in staged {
        let _ = overlay.step(op);
    }
    overlay.current(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::{CompareOp, Predicate};
    use uuid::Uuid;

    fn id(n: u128) -> RecordId {
        Uuid::from_u128(n)
    }

    fn rec(v: i64) -> Fields {
        vec![(1, ScanValue::I64(v))]
    }

    fn store(present: &[(u128, i64)]) -> impl Fn(RecordId) -> Option<Fields> {
        let map: HashMap<RecordId, Fields> =
            present.iter().map(|(n, v)| (id(*n), rec(*v))).collect();
        move |i| map.get(&i).cloned()
    }

    fn none(_: RecordId, _: RecordId, _: &str) -> bool {
        false
    }

    fn insert(n: u128, v: i64) -> WriteOp {
        WriteOp::Insert {
            id: id(n),
            fields: rec(v),
        }
    }

    #[test]
    fn a_clean_list_has_no_failure_and_earlier_ops_feed_later_ones() {
        let get = store(&[(1, 10)]);
        let ops = [
            insert(2, 20),
            WriteOp::UpdateField {
                id: id(2),
                field: 1,
                value: ScanValue::I64(21),
            },
            WriteOp::Delete { id: id(1) },
            insert(1, 11),
        ];
        assert_eq!(first_soft_failure(&ops, &get, &none), None);
    }

    #[test]
    fn the_first_soft_outcome_is_named_with_its_index_and_code() {
        let get = store(&[(1, 10)]);
        assert_eq!(
            first_soft_failure(&[insert(1, 0)], &get, &none),
            Some((0, ErrorCode::Duplicate))
        );
        assert_eq!(
            first_soft_failure(&[insert(2, 0), WriteOp::Delete { id: id(3) }], &get, &none),
            Some((1, ErrorCode::RecordNotFound))
        );
        assert_eq!(
            first_soft_failure(
                &[insert(2, 0), WriteOp::Delete { id: id(2) }, insert(2, 0)],
                &get,
                &none
            ),
            None,
            "insert, delete, insert again is clean"
        );
        assert_eq!(
            first_soft_failure(
                &[WriteOp::Delete { id: id(1) }, insert(1, 0), insert(1, 0)],
                &get,
                &none
            ),
            Some((2, ErrorCode::Duplicate))
        );
    }

    #[test]
    fn a_guard_reads_the_record_as_the_batch_left_it() {
        let get = store(&[(1, 10)]);
        let guard = Predicate {
            field: 1,
            op: CompareOp::Eq,
            value: ScanValue::I64(11),
        };
        let replace_if = WriteOp::ReplaceIf {
            id: id(1),
            fields: rec(12),
            guard,
        };
        let bump = WriteOp::UpdateField {
            id: id(1),
            field: 1,
            value: ScanValue::I64(11),
        };
        assert_eq!(
            first_soft_failure(std::slice::from_ref(&replace_if), &get, &none),
            Some((0, ErrorCode::GuardFailed))
        );
        assert_eq!(first_soft_failure(&[bump, replace_if], &get, &none), None);
    }

    #[test]
    fn links_see_the_store_the_batch_and_a_deleted_endpoint() {
        let get = store(&[(1, 0), (2, 0), (3, 0)]);
        let stored = |l: RecordId, r: RecordId, rel: &str| {
            rel == "r" && l.min(r) == id(1) && l.max(r) == id(2)
        };
        let link = |a, b| WriteOp::Link {
            left: id(a),
            right: id(b),
            relation: "r".into(),
        };
        assert_eq!(
            first_soft_failure(&[link(2, 1)], &get, &stored),
            Some((0, ErrorCode::Duplicate)),
            "the stored edge, endpoints either way round"
        );
        assert_eq!(
            first_soft_failure(
                &[WriteOp::Delete { id: id(1) }, insert(1, 0), link(1, 2)],
                &get,
                &stored
            ),
            None,
            "a re-inserted record has none of its old edges"
        );
        assert_eq!(
            first_soft_failure(&[link(1, 3), link(3, 1)], &get, &none),
            Some((1, ErrorCode::Duplicate)),
            "the batch's own edge"
        );
    }

    #[test]
    fn overlay_get_shows_the_staged_record() {
        let staged = [
            insert(5, 1),
            WriteOp::UpdateField {
                id: id(5),
                field: 1,
                value: ScanValue::I64(2),
            },
        ];
        assert_eq!(overlay_get(id(5), None, &staged), Some(rec(2)));
        assert_eq!(overlay_get(id(6), Some(rec(9)), &staged), Some(rec(9)));
        let del = [WriteOp::Delete { id: id(6) }];
        assert_eq!(overlay_get(id(6), Some(rec(9)), &del), None);
    }
}
