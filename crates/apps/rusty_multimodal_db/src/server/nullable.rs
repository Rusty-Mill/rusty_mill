//! Nullable columns as a wire view over stored sentinels — `NLC-FR-001`..
//! `NLC-FR-005`, `ADR-0128`, protocol 32.
//!
//! A column the storage layer keeps as a sentinel (`Memory`'s
//! `deleted_at_unix_ms`: `0`, `node_id`: `""`, `ADR-0056`) can be a real
//! `NULL` to a client that negotiated 32 without changing the stored
//! layout: an adapter names the fields and their sentinel through
//! `ConnectionStore::nullable_fields`, and `handle_connection`
//! translates at the wire edge — `to_storage` turns a `Null` a request
//! carries for such a field into the sentinel before anything else reads
//! the request, and `to_wire` turns a stored sentinel in a response into
//! `Null`. Everything between (validation, the planner, sessions, MVCC,
//! the journal) sees only the sentinels it always saw, and a connection
//! below 32 sees exactly what it always saw.
//!
//! Only equality means "is null": `Eq`/`Ne` against `Null` are rewritten;
//! an ordering comparison against `Null` is left alone and answered
//! `Malformed`, as before (`ADR-0117`: ordering never holds). Aggregates
//! read the sentinel like any stored value (`SUM(deleted_at)` adds the
//! zeros, `MIN` is `0` when a row is null); `GROUP BY` a nullable field
//! keys its group `Null`. Named in the ADR, not hidden.

use super::protocol::{
    AggregateGroup, CompareOp, FieldRef, JoinedRow, Predicate, Request, Response, ScanValue,
    TransactionOp, WriteOp,
};

/// One field that is `NULL` on the wire while stored as `sentinel`.
#[derive(Debug, Clone, PartialEq)]
pub struct NullableField {
    /// The field's tag.
    pub tag: FieldRef,
    /// The stored value that means "no value" — lossless for the column
    /// (`ADR-0056`: no real record holds it).
    pub sentinel: ScanValue,
}

/// What a response's shape depends on in the request that produced it:
/// which field a [`Response::ScanValues`] holds, which fields a
/// [`Response::Groups`] is keyed by, and whether a joined row's right side
/// is this table's.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WireContext {
    scan_field: Option<FieldRef>,
    group_by: Vec<FieldRef>,
    join_right_is_ours: bool,
}

impl WireContext {
    /// The context of `req`, answered by the table called `own_table`.
    pub fn of(req: &Request, own_table: &str) -> Self {
        match req {
            Request::ScanField { field } => Self {
                scan_field: Some(*field),
                ..Self::default()
            },
            Request::Aggregate { group_by, .. } => Self {
                group_by: group_by.clone(),
                ..Self::default()
            },
            Request::Join(spec) => Self {
                join_right_is_ours: spec.right_table.as_deref().is_none_or(|t| t == own_table),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

fn to_stored(fields: &[NullableField], tag: FieldRef, value: ScanValue) -> ScanValue {
    match (&value, fields.iter().find(|f| f.tag == tag)) {
        (ScanValue::Null, Some(field)) => field.sentinel.clone(),
        _ => value,
    }
}

fn to_null(fields: &[NullableField], tag: FieldRef, value: ScanValue) -> ScanValue {
    match fields.iter().find(|f| f.tag == tag) {
        Some(field) if field.sentinel == value => ScanValue::Null,
        _ => value,
    }
}

fn map_pairs(
    pairs: Vec<(FieldRef, ScanValue)>,
    map: impl Fn(FieldRef, ScanValue) -> ScanValue,
) -> Vec<(FieldRef, ScanValue)> {
    pairs
        .into_iter()
        .map(|(tag, value)| (tag, map(tag, value)))
        .collect()
}

/// Only `Eq` and `Ne` mean "is (not) null"; a bound never does.
fn predicate_to_stored(fields: &[NullableField], mut p: Predicate) -> Predicate {
    if matches!(p.op, CompareOp::Eq | CompareOp::Ne) {
        p.value = to_stored(fields, p.field, p.value);
    }
    p
}

fn predicates_to_stored(fields: &[NullableField], filter: Vec<Predicate>) -> Vec<Predicate> {
    filter
        .into_iter()
        .map(|p| predicate_to_stored(fields, p))
        .collect()
}

fn op_to_stored(fields: &[NullableField], op: WriteOp) -> WriteOp {
    match op {
        WriteOp::Insert { id, fields: f } => WriteOp::Insert {
            id,
            fields: map_pairs(f, |t, v| to_stored(fields, t, v)),
        },
        WriteOp::Replace { id, fields: f } => WriteOp::Replace {
            id,
            fields: map_pairs(f, |t, v| to_stored(fields, t, v)),
        },
        WriteOp::ReplaceIf {
            id,
            fields: f,
            guard,
        } => WriteOp::ReplaceIf {
            id,
            fields: map_pairs(f, |t, v| to_stored(fields, t, v)),
            guard: predicate_to_stored(fields, guard),
        },
        other @ (WriteOp::Delete { .. } | WriteOp::Link { .. }) => other,
    }
}

/// `NLC-FR-002`: `req` with every `Null` a nullable field carries replaced
/// by its sentinel. A `Null` for any other field is left for validation to
/// refuse. `own_table` is the table answering, so a cross-table join's
/// right filter (another table's fields) is not touched.
pub fn to_storage(req: Request, fields: &[NullableField], own_table: &str) -> Request {
    if fields.is_empty() {
        return req;
    }
    let stored = |tag, value| to_stored(fields, tag, value);
    match req {
        Request::FilterEq { field, value } => Request::FilterEq {
            field,
            value: stored(field, value),
        },
        Request::UpdateField { id, field, value } => Request::UpdateField {
            id,
            field,
            value: stored(field, value),
        },
        Request::Transaction { updates } => Request::Transaction {
            updates: updates
                .into_iter()
                .map(|op| TransactionOp {
                    value: stored(op.field, op.value),
                    ..op
                })
                .collect(),
        },
        Request::Insert { id, fields: f } => Request::Insert {
            id,
            fields: map_pairs(f, stored),
        },
        Request::Replace { id, fields: f } => Request::Replace {
            id,
            fields: map_pairs(f, stored),
        },
        Request::ReplaceIf {
            id,
            fields: f,
            guard,
        } => Request::ReplaceIf {
            id,
            fields: map_pairs(f, stored),
            guard: predicate_to_stored(fields, guard),
        },
        Request::WriteBatch { ops, atomic } => Request::WriteBatch {
            ops: ops.into_iter().map(|op| op_to_stored(fields, op)).collect(),
            atomic,
        },
        Request::Query {
            select,
            filter,
            limit,
        } => Request::Query {
            select,
            filter: predicates_to_stored(fields, filter),
            limit,
        },
        Request::Aggregate {
            group_by,
            filter,
            aggregates,
            limit,
        } => Request::Aggregate {
            group_by,
            filter: predicates_to_stored(fields, filter),
            aggregates,
            limit,
        },
        Request::FilteredPage {
            order_by,
            after,
            limit,
            filter,
        } => Request::FilteredPage {
            order_by,
            after,
            limit,
            filter: predicates_to_stored(fields, filter),
        },
        Request::FilteredPageDesc {
            order_by,
            before,
            limit,
            filter,
        } => Request::FilteredPageDesc {
            order_by,
            before,
            limit,
            filter: predicates_to_stored(fields, filter),
        },
        Request::Join(mut spec) => {
            spec.left_filter = predicates_to_stored(fields, spec.left_filter);
            if spec.right_table.as_deref().is_none_or(|t| t == own_table) {
                spec.right_filter = predicates_to_stored(fields, spec.right_filter);
            }
            Request::Join(spec)
        }
        other => other,
    }
}

fn rows_to_wire(
    fields: &[NullableField],
    rows: Vec<(super::protocol::RecordId, Vec<(FieldRef, ScanValue)>)>,
) -> Vec<(super::protocol::RecordId, Vec<(FieldRef, ScanValue)>)> {
    rows.into_iter()
        .map(|(id, pairs)| (id, map_pairs(pairs, |t, v| to_null(fields, t, v))))
        .collect()
}

/// `NLC-FR-003`: `resp` with every stored sentinel of a nullable field
/// replaced by `Null`. `ctx` is what the answered request contributes
/// ([`WireContext::of`]).
pub fn to_wire(resp: Response, fields: &[NullableField], ctx: &WireContext) -> Response {
    if fields.is_empty() {
        return resp;
    }
    let null = |tag, value| to_null(fields, tag, value);
    match resp {
        Response::Record { id, fields: f } => Response::Record {
            id,
            fields: map_pairs(f, null),
        },
        Response::Rows { rows } => Response::Rows {
            rows: rows_to_wire(fields, rows),
        },
        Response::RowsClamped { rows, cap } => Response::RowsClamped {
            rows: rows_to_wire(fields, rows),
            cap,
        },
        Response::JoinedRows { rows } => Response::JoinedRows {
            rows: rows
                .into_iter()
                .map(|row| JoinedRow {
                    left: map_pairs(row.left, null),
                    right: if ctx.join_right_is_ours {
                        map_pairs(row.right, null)
                    } else {
                        row.right
                    },
                    ..row
                })
                .collect(),
        },
        Response::ScanValues { values } => match ctx.scan_field {
            Some(tag) => Response::ScanValues {
                values: values.into_iter().map(|v| null(tag, v)).collect(),
            },
            None => Response::ScanValues { values },
        },
        Response::Groups { groups } => Response::Groups {
            groups: groups
                .into_iter()
                .map(|g| AggregateGroup {
                    key: map_pairs(g.key, |t, v| {
                        if ctx.group_by.contains(&t) {
                            null(t, v)
                        } else {
                            v
                        }
                    }),
                    ..g
                })
                .collect(),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::RecordId;

    const DELETED: FieldRef = 11;
    const NODE: FieldRef = 12;
    const OTHER: FieldRef = 5;

    fn fields() -> Vec<NullableField> {
        vec![
            NullableField {
                tag: DELETED,
                sentinel: ScanValue::I64(0),
            },
            NullableField {
                tag: NODE,
                sentinel: ScanValue::Str(String::new()),
            },
        ]
    }

    fn pred(field: FieldRef, op: CompareOp, value: ScanValue) -> Predicate {
        Predicate { field, op, value }
    }

    /// `NLC-FR-002`: `Eq`/`Ne` against `Null` on a nullable field become the
    /// sentinel; a bound stays `Null`; another field's `Null` stays `Null`.
    #[test]
    fn only_equality_against_null_on_a_nullable_field_is_rewritten() {
        let req = Request::Query {
            select: super::super::protocol::Selection::All,
            filter: vec![
                pred(DELETED, CompareOp::Eq, ScanValue::Null),
                pred(NODE, CompareOp::Ne, ScanValue::Null),
                pred(DELETED, CompareOp::Lt, ScanValue::Null),
                pred(OTHER, CompareOp::Eq, ScanValue::Null),
            ],
            limit: None,
        };
        let Request::Query { filter, .. } = to_storage(req, &fields(), "memory") else {
            panic!("a Query stays a Query");
        };
        assert_eq!(filter[0].value, ScanValue::I64(0));
        assert_eq!(filter[1].value, ScanValue::Str(String::new()));
        assert_eq!(filter[2].value, ScanValue::Null, "a bound is not null-ness");
        assert_eq!(filter[3].value, ScanValue::Null, "not a nullable field");
    }

    /// `NLC-FR-002`: writes carry `Null` for a nullable field as the sentinel.
    #[test]
    fn a_write_of_null_stores_the_sentinel() {
        let id = RecordId::from_u128(1);
        let Request::Insert { fields: f, .. } = to_storage(
            Request::Insert {
                id,
                fields: vec![
                    (DELETED, ScanValue::Null),
                    (OTHER, ScanValue::Null),
                    (NODE, ScanValue::Str("n1".into())),
                ],
            },
            &fields(),
            "memory",
        ) else {
            panic!("an Insert stays an Insert");
        };
        assert_eq!(f[0].1, ScanValue::I64(0));
        assert_eq!(f[1].1, ScanValue::Null, "left for validation to refuse");
        assert_eq!(f[2].1, ScanValue::Str("n1".into()), "a real value is kept");
    }

    /// `NLC-FR-003`: a stored sentinel reads as `Null`, a real value does not.
    #[test]
    fn a_sentinel_reads_as_null_and_a_real_value_does_not() {
        let id = RecordId::from_u128(1);
        let resp = Response::Record {
            id,
            fields: vec![
                (DELETED, ScanValue::I64(0)),
                (NODE, ScanValue::Str("n1".into())),
                (OTHER, ScanValue::I64(0)),
            ],
        };
        let Response::Record { fields: f, .. } = to_wire(resp, &fields(), &WireContext::default())
        else {
            panic!("a Record stays a Record");
        };
        assert_eq!(f[0].1, ScanValue::Null);
        assert_eq!(f[1].1, ScanValue::Str("n1".into()));
        assert_eq!(f[2].1, ScanValue::I64(0), "0 in another field is just 0");
    }

    /// `NLC-FR-003`: a cross-table join's right side is another table's and
    /// is not translated; a same-table join's is.
    #[test]
    fn a_joined_rows_right_side_is_translated_only_when_it_is_ours() {
        let row = || JoinedRow {
            left_id: RecordId::from_u128(1),
            left: vec![(DELETED, ScanValue::I64(0))],
            right_id: RecordId::from_u128(2),
            right: vec![(DELETED, ScanValue::I64(0))],
        };
        let ours = WireContext {
            join_right_is_ours: true,
            ..WireContext::default()
        };
        let Response::JoinedRows { rows } =
            to_wire(Response::JoinedRows { rows: vec![row()] }, &fields(), &ours)
        else {
            panic!("JoinedRows stays JoinedRows");
        };
        assert_eq!(rows[0].right[0].1, ScanValue::Null);
        let Response::JoinedRows { rows } = to_wire(
            Response::JoinedRows { rows: vec![row()] },
            &fields(),
            &WireContext::default(),
        ) else {
            panic!("JoinedRows stays JoinedRows");
        };
        assert_eq!(
            rows[0].left[0].1,
            ScanValue::Null,
            "the left is always ours"
        );
        assert_eq!(rows[0].right[0].1, ScanValue::I64(0), "the right is not");
    }

    /// `NLC-FR-003`: a group keyed by a nullable field keys `Null`; an
    /// aggregate value is left as stored.
    #[test]
    fn a_group_keyed_by_a_nullable_field_is_keyed_null() {
        let ctx = WireContext {
            group_by: vec![DELETED],
            ..WireContext::default()
        };
        let resp = Response::Groups {
            groups: vec![AggregateGroup {
                key: vec![(DELETED, ScanValue::I64(0))],
                values: vec![ScanValue::I64(0)],
            }],
        };
        let Response::Groups { groups } = to_wire(resp, &fields(), &ctx) else {
            panic!("Groups stays Groups");
        };
        assert_eq!(groups[0].key[0].1, ScanValue::Null);
        assert_eq!(groups[0].values[0], ScanValue::I64(0));
    }

    /// `NLC-FR-004`: no nullable field, no change — the request and response
    /// come back untouched.
    #[test]
    fn with_no_nullable_field_nothing_changes() {
        let req = Request::FilterEq {
            field: DELETED,
            value: ScanValue::Null,
        };
        assert_eq!(
            format!("{:?}", to_storage(req, &[], "memory")),
            "FilterEq { field: 11, value: Null }"
        );
    }
}
