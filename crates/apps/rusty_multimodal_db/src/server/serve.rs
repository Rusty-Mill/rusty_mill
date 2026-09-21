//! The server half of [`super`] — everything that *serves*: the
//! [`ConnectionStore`] trait every domain adapter implements, `dispatch`,
//! `handle_connection`/[`serve`], [`ServeOptions`] (authentication, rate
//! limiting, TLS, audit and access logs), [`TlsConfig`], and the query,
//! aggregate, join, and version-downgrade evaluators. Compiled only under
//! the `server` Cargo feature and re-exported at `crate::server::*`, so no
//! public path changed when this split landed (`ECO-FR-001`/`002`,
//! ADR-0043): a consumer that only *talks* to a server depends on the
//! `client` feature and compiles none of this file. See `super`'s own
//! module docs for the server's contract; this file is its body.

use super::metrics::{ConnectionMetricsGuard, PlanKind, ServerMetrics};
use super::protocol::{
    AggregateFn, AggregateGroup, AggregateSpec, CompareOp, DomainSchema, ErrorCode, FieldRef,
    JoinRelation, JoinSpec, JoinedRow, ParentLookup, Predicate, RecordId, RelationDescriptor,
    Request, Response, ScanValue, Selection, TransactionOp, WriteOp, WriteResult, MAX_BATCH_OPS,
    MAX_SNAPSHOT_BYTES, MAX_STAGED_OPS, MAX_TRACKED_READS, PROTOCOL_VERSION,
    SESSION_MVCC_ISOLATION, SESSION_READ_YOUR_WRITES, SESSION_SNAPSHOT_ISOLATION,
    SESSION_VALIDATE_ON_STAGE,
};
use super::{access, audit, framing, pem, protocol, TlsConfigError};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// One shared trait the dispatch loop is generic over, implemented by a
/// thin per-domain adapter — the dispatch loop itself never depends on
/// which concrete store it's serving. See [`super::dog::DogConnectionStore`]
/// (`Neighbors` only), [`super::order::OrderConnectionStore`] (`Parent`/`Children`
/// only), and [`super::employee::EmployeeConnectionStore`] (both — the first
/// domain to combine them) for the three domains this crate validates
/// against, matching this project's own "validate against a second,
/// structurally different domain" discipline
/// (`docs/decisions/ADR-0009-generic-schema-design-proposal.md`).
/// What [`ConnectionStore::insert_record`] did (`INS-FR-006`, ADR-0046):
/// the record is now stored, or its id already had one and nothing was
/// written — the normal-outcome pair `dispatch` maps to [`Response::Ok`]
/// / `Err { Duplicate }`, kept apart from the `ErrorCode` refusals
/// (`Malformed`, `UnknownField`, `Unsupported`) the same way
/// `update_field`'s `Ok(false)` is kept apart from its errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

/// What [`ConnectionStore::link_records`] did (`LNK-FR-009`, ADR-0047):
/// the edge is new, or it was already present and nothing was written.
/// Both are [`Response::Ok`] on the wire — insert-or-ignore, the
/// consumer's own semantics — kept apart here for the adapters' tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked,
    AlreadyLinked,
}

/// What [`ConnectionStore::replace_record`] did (`REP-FR-005`, ADR-0049):
/// the record is now the new version, or its id had no record and
/// nothing was written — the normal-outcome pair `dispatch` maps to
/// [`Response::Ok`] / [`Response::NotFound`], kept apart from the
/// `ErrorCode` refusals exactly as [`InsertOutcome`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceOutcome {
    Replaced,
    NotFound,
}

/// What [`ConnectionStore::delete_record`] did (`DEL-FR-006`, ADR-0051):
/// the record is gone, or its id had none and nothing was written — the
/// normal-outcome pair `dispatch` maps to [`Response::Ok`] /
/// [`Response::NotFound`], exactly as [`ReplaceOutcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
}

/// What [`ConnectionStore::replace_record_if`] did (`GRD-FR-003`,
/// ADR-0054): the guard held and the record is the new version; the id
/// had no record (the guard never evaluated); or the guard did not hold
/// — nothing written in the latter two. `dispatch` maps them to
/// [`Response::Ok`] / [`Response::NotFound`] / `Err { GuardFailed }`.
/// One row of a [`Response::Rows`] page or query: a record's id and
/// every field of it in tag order (`PAG-FR-002`).
pub type PageRow = (RecordId, Vec<(FieldRef, ScanValue)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceIfOutcome {
    Replaced,
    NotFound,
    GuardFailed,
}

/// What [`ConnectionStore::backup`] copied (`BAK-FR-001`, ADR-0065) — the
/// same shape [`crate::generic::CompactionReport`] uses for a
/// filesystem-facing operator report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackupReport {
    /// Files copied into the target directory.
    pub files: u64,
    /// Total bytes copied.
    pub bytes: u64,
}

pub trait ConnectionStore: Send + Sync {
    /// Full-record read. `None` if `id` has no record — an ordinary
    /// outcome, not an error, matching [`crate::store::DogStore::get`]'s
    /// own convention.
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>>;

    /// Equality filter on an indexed field. `Err(ErrorCode::UnknownField)`
    /// for a tag this adapter doesn't recognize at all;
    /// `Err(ErrorCode::Unsupported)` for a recognized field with no
    /// equality-index in-process; `Err(ErrorCode::Malformed)` if `value`'s
    /// variant doesn't match the field's real type.
    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode>;

    /// Every record's value for a scannable field, unspecified order —
    /// generalizes `scan_ages`/`ScanField::scan`.
    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode>;

    /// `Ok(true)` if `id` was found and updated, `Ok(false)` if `id` has no
    /// record (an ordinary outcome, matching `update_age`'s own
    /// `NotFound` case at this layer) — `Err` only for a field/value
    /// problem, not a missing record.
    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode>;

    /// The "one hop up" side of a directed relation. See
    /// [`ParentLookup`]'s own doc comment for why this preserves the
    /// not-found/no-parent distinction rather than collapsing it.
    /// `Err(ErrorCode::Unsupported)` for a domain with no directed
    /// relation at all (e.g. `Dog`).
    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode>;

    /// The "one hop down" side of a directed relation.
    /// `Err(ErrorCode::Unsupported)` for a domain with no directed
    /// relation at all.
    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode>;

    /// A symmetric relation (e.g. `Dog`'s `littermate_of`). Returns the
    /// union of every named relation this domain has, if it has more
    /// than one — the same "no relation-type discriminant" shape
    /// [`Request::Neighbors`] has always had. `Err(ErrorCode::Unsupported)`
    /// for a domain with no symmetric relation at all (e.g.
    /// `Order`/`Customer`).
    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode>;

    /// `ENT2-FR-004` (ADR-0039): a symmetric relation filtered to one
    /// named label — for a domain with more than one `SymmetricRelation`
    /// (`Entity`, the first). `Err(ErrorCode::Malformed)` for a label
    /// this domain doesn't have at all; `Err(ErrorCode::Unsupported)`
    /// for a domain with no symmetric relation, matching `neighbors`'s
    /// own convention. Every adapter with at most one relation label
    /// implements this as `Err(ErrorCode::Unsupported)` unconditionally
    /// — the same "not yet needed" shape `parent`/`children` had before
    /// `Employee` needed them for real.
    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode>;

    /// `ENT2-FR-005` (ADR-0039): every relation label this domain knows
    /// — `[]` for a domain with no symmetric relation at all, one label
    /// for a single-relation domain, more than one for `Entity`.
    /// Infallible, the same shape `describe` already has.
    fn list_relation_kinds(&self) -> Vec<String>;

    /// `JOIN-FR-002` (ADR-0044, protocol 12): every relation a
    /// [`Request::Join`] on this table may name. The default is derived
    /// from [`ConnectionStore::describe`] and
    /// [`ConnectionStore::list_relation_kinds`] by [`default_relation_
    /// descriptors`] and is deliberately **conservative**: symmetric
    /// relations are listed (`neighbors` plus one entry per label — they
    /// are self-referential by construction), `parent`/`children` are
    /// **omitted**, because the default cannot know whether the parent
    /// type is this same table (`Employee`: yes; `Order`: no, its parent
    /// is a `Customer` no store holds). An adapter whose directed relation
    /// is self-referential overrides this to add them with `target_table:
    /// None`. A wrong claim here returns wrong rows silently — the same
    /// trust `describe()` already carries (`FR-010`).
    fn describe_relations(&self) -> Vec<RelationDescriptor> {
        default_relation_descriptors(&self.describe(), self.list_relation_kinds())
    }

    /// `TBL-FR-001` (ADR-0045/ADR-0050): the name [`serve`] registers this
    /// adapter under, what [`Request::Use`] selects it by, and what a
    /// `RelationDescriptor::target_table` on *another* table's relation
    /// must equal to reach it. Every shipped adapter overrides this with
    /// its domain's name (`"dog"`, `"entity"`, `"memory"`, …); the
    /// default exists so a bespoke adapter compiles unchanged.
    fn table_name(&self) -> &str {
        "table"
    }

    /// `INS-FR-006` (ADR-0046, protocol 13): add one whole record to
    /// this table at runtime. `fields` is [`Response::Record`]'s own
    /// shape; an implementor validates it against its own schema
    /// **before any write** — every described tag present exactly once
    /// with a value of its `ValueKind` (`Malformed` for a missing,
    /// repeated, or wrong-kind field; `UnknownField` for a tag the
    /// schema doesn't describe), then its domain's own rule (`Reminder`:
    /// the `status` discriminant) — and answers
    /// [`InsertOutcome::Duplicate`] when `id` already has a record, with
    /// nothing written. The default answers `Unsupported`: `Dog`'s
    /// bespoke `ProductionStore` has no insert, and `Order`/`Employee`
    /// are `research`-gated reference material; `Reminder` and `Entity`
    /// implement it. A durability failure — the insert log or slot
    /// append failed, nothing applied — is [`ErrorCode::Storage`]
    /// (ADR-0046 option (d)): its own code, so a client's handling of
    /// `Journal` (a transaction batch that was not journaled) never
    /// fires for an insert.
    fn insert_record(
        &self,
        _id: RecordId,
        _fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `LNK-FR-009` (ADR-0047, protocol 14): add one edge between two
    /// records of this table under a symmetric relation label. An
    /// implementor validates `relation` with
    /// [`crate::generic::store::valid_relation_label`] (`Malformed`) and,
    /// on a fixed-label domain, against its own labels (`Malformed`);
    /// a missing endpoint is `RecordNotFound`, a self-loop `Malformed`, a
    /// durability failure `Storage`. The default answers `Unsupported`:
    /// `Dog`'s bespoke store, and `Reminder`/`Order`, which have no
    /// symmetric relation. `Entity` (open labels) and `Employee` (its one
    /// fixed label, `research`) implement it.
    fn link_records(
        &self,
        _left: RecordId,
        _right: RecordId,
        _relation: &str,
    ) -> Result<LinkOutcome, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `REP-FR-005` (ADR-0049, protocol 15): replace the record at `id`
    /// whole. An implementor validates `fields` exactly as
    /// [`Self::insert_record`] does — the whole list before any write,
    /// the domain's own rules included — and answers
    /// [`ReplaceOutcome::NotFound`] when `id` has no record, with nothing
    /// written. The default answers `Unsupported`: `Dog`'s bespoke store,
    /// and `Order`/`Employee` as reference material; `Memory`, `Reminder`,
    /// and `Entity` implement it. A durability failure is
    /// [`ErrorCode::Storage`], as for an insert.
    fn replace_record(
        &self,
        _id: RecordId,
        _fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `GRD-FR-003` (ADR-0054, protocol 19): replace the record at `id`
    /// whole **only if `guard` holds against the stored record**, the
    /// read, the comparison, and the write under one acquisition of the
    /// store's write lock. `fields` is validated exactly as
    /// [`Self::replace_record`]'s; `guard` has already been validated
    /// against the schema by `dispatch` (`validate_predicate`), so an
    /// implementor evaluates it with [`predicate_matches`] over its own
    /// wire shape of the stored record. The default answers
    /// `Unsupported`, as [`Self::replace_record`] does; `Memory`,
    /// `Reminder`, and `Entity` implement it. A durability failure is
    /// [`ErrorCode::Storage`].
    fn replace_record_if(
        &self,
        _id: RecordId,
        _fields: Vec<(FieldRef, ScanValue)>,
        _guard: &Predicate,
    ) -> Result<ReplaceIfOutcome, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `PAG-FR-002` (ADR-0055, protocol 20): one ordered keyset page —
    /// every record sorted ascending by `order_by` with the id as the
    /// tie-break, strictly after `after`, at most `limit` rows. `dispatch`
    /// has already validated `order_by` as an orderable field, `after`'s
    /// kind, and `limit` (`validate_page`). The default keeps `Query`'s
    /// full-scan posture but touches only sort keys until the page is
    /// chosen: [`Self::page_keys`] for every record, [`page_ids`] to pick
    /// the page, then [`Self::get`] for just those rows. An adapter with
    /// a cheaper order may override it.
    fn page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        Ok(page_by_scan(self, order_by, after, limit))
    }

    /// The `(id, sort key)` of every record for a [`Self::page`] ordered
    /// by `order_by` — the key is [`page_key`]'s. The default derives it
    /// from [`Self::scan_all`], materializing every field of every
    /// record; an adapter that can read one numeric field off its own
    /// record type overrides this to skip that work (the consumer's
    /// three tables do).
    fn page_keys(&self, order_by: FieldRef) -> Vec<(RecordId, i128)> {
        self.scan_all()
            .into_iter()
            .map(|(id, fields)| (id, page_key(&fields, order_by, id).0))
            .collect()
    }

    /// `QPR-FR-002` (ADR-0075): the one field this adapter keeps an
    /// `Ordered` index (`ADR-0059`) over, if any — the field a `WHERE`
    /// range can be walked on through [`Self::range_ids`] instead of
    /// scanned. Server-side only, deliberately not a `FieldCapabilities`
    /// flag: that is wire shape, and a client cannot act on the answer.
    /// The default is `None` — `Dog`, `Order`, `Employee`, `Reminder`,
    /// and `Entity` keep no such index; `Memory` and `Relation` answer
    /// `updated_at_unix_ms`.
    fn range_field(&self) -> Option<FieldRef> {
        None
    }

    /// `QPR-FR-002` (ADR-0075): every id whose stored `field` value lies
    /// within `lower..upper`, ascending by `(value, id)` — a range walk
    /// of the `Ordered` index [`Self::range_field`] names, the same set
    /// [`Self::page`] walks from a cursor. `Unsupported` for any other
    /// field, `Malformed` for a bound whose value is not the field's
    /// kind; `dispatch` reaches neither (it asks only for `range_field`'s
    /// own tag, with literals `validate_predicate` has already
    /// kind-checked) and treats any error as "walk refused, scan instead"
    /// (`QPR-FR-004`). The default answers `Unsupported`, as
    /// `range_field`'s default `None` implies.
    fn range_ids(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `QPB-FR-002` (ADR-0079): [`Self::range_ids`] with a budget — the
    /// whole range if it holds at most `limit` ids, `Ok(None)` if it
    /// holds more (the walk abandoned at the first id past the budget),
    /// the same errors as `range_ids`. What the intersection plan
    /// (`QPI-FR-002`) walks with, so a range far wider than the
    /// equality bucket costs at most the budget in id-level work and
    /// then yields to the bucket alone. The default answers
    /// `Unsupported`, as `range_ids`'s does; `Memory` and `Relation`
    /// implement it.
    fn range_ids_limited(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
        _limit: usize,
    ) -> Result<Option<Vec<RecordId>>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `QCW-FR-002` (ADR-0081): how many records lie within `lower..upper`
    /// on [`Self::range_field`] — [`Self::range_ids`]'s length with no
    /// `Vec` and no record read, the same errors. What a `COUNT(*)` whose
    /// every predicate is a bound on the range field answers with
    /// ([`counted_walk`]). The default answers `Unsupported`, as
    /// `range_ids`'s does; `Memory`, `Relation`, and `Reminder` implement
    /// it.
    fn range_count(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<u64, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `QKW-FR-002` (ADR-0082): the keys within `lower..upper` on
    /// [`Self::range_field`], ascending — the walk yielding the key of
    /// each pair instead of its id, no record read; the same errors as
    /// `range_ids`. What an aggregate over the range field itself
    /// (`MIN`/`MAX`/`SUM`/`AVG` of it) answers with ([`keyed_walk`]).
    /// Every shipped range field is `I64`. The default answers
    /// `Unsupported`; `Memory`, `Relation`, and `Reminder` implement it.
    fn range_keys(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<Vec<i64>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `QRF-FR-002` (ADR-0087): [`Self::range_keys`] reduced in one pass
    /// — the count, sum, least and greatest of the keys within
    /// `lower..upper` on [`Self::range_field`] — with no `Vec` of keys
    /// materialized; the same errors as `range_ids`. What an ungrouped
    /// aggregate over the range field answers with ([`keyed_walk`]). The
    /// default answers `Unsupported`; `Memory`, `Relation`, and
    /// `Reminder` implement it over [`crate::generic::query::RangeBy::range_fold`].
    fn range_stats(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<KeyStats, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `FPG-FR-003`/`FPG-FR-004` (ADR-0068, protocol 26): one ordered
    /// keyset page over only the rows every predicate in `filter`
    /// matches — [`Self::page`]'s own contract plus a `WHERE`-shaped
    /// filter. `dispatch` has already validated `order_by`/`after`/
    /// `limit` (`validate_page`'s checks) and every `filter` predicate
    /// (`validate_predicate`, `Request::Query`'s own rule). The default
    /// answers every domain correctly: the candidate rows — since
    /// `QPC-FR-003` (ADR-0074), `Query`'s own [`indexed_candidates`]: the
    /// declared equality index's bucket when `filter` has an `Eq` on an
    /// indexed field, [`Self::scan_all`] otherwise — keep only rows
    /// [`predicate_matches`] every predicate for (the identical filter
    /// step `Request::Query`'s own [`evaluate_query`] uses), then
    /// [`page_rows`] over that already-filtered, already-materialized
    /// subset. The page's order is [`page_rows`]'s `(key, id)` order over
    /// the filtered *set*, so which plan gathered the set is invisible
    /// here. Since `QPR-FR-004` (ADR-0075) a range on `Memory`/
    /// `Relation`'s `Ordered` field (`ADR-0059`) is walked, not scanned,
    /// by that same candidate step — but the walk still reads every
    /// in-range record before `page_rows` cuts the page. Since
    /// `FPW-FR-003` (ADR-0076) `Memory`/`Relation` override this method
    /// with [`bounded_filtered_page`] when `order_by` is the range field
    /// — the O(page) walk from `max(cursor, tightest lower bound)`, cut
    /// at the first upper-bound reject, and since `FPM-FR-002`
    /// (ADR-0077) continued past any other predicate's rejects until the
    /// page fills — and fall back to [`filtered_page_by_candidates`],
    /// this default's own body, when the filter plans the declared
    /// equality index instead ([`bounded_walk_applies`]). The
    /// [`Self::page`] fast path's unfiltered case is untouched.
    fn filtered_page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
        filter: &[Predicate],
    ) -> Result<Vec<PageRow>, ErrorCode> {
        Ok(filtered_page_by_candidates(
            self, order_by, after, limit, filter,
        ))
    }

    /// `CNT-FR-002` (ADR-0057, protocol 21): how many edges this table
    /// holds under `relation`, each undirected edge once, a cross-table
    /// label included. `Malformed` for a label the table has no relation
    /// under — [`Self::neighbors_by_relation`]'s own rule. The default
    /// answers `Unsupported`; `Memory` and `Entity` implement it.
    fn count_edges(&self, _relation: &str) -> Result<u64, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `WBT-FR-002` (ADR-0060, protocol 22): apply one [`WriteOp`] through
    /// this adapter's single-shot write methods and map its outcome to a
    /// [`WriteResult`]. The per-op step of the default pipelined
    /// [`Self::write_batch`]; a domain with no runtime write answers
    /// `Failed(Unsupported)` here, from the write methods' own defaults.
    fn apply_write_op(&self, op: &WriteOp) -> WriteResult {
        match op {
            WriteOp::Insert { id, fields } => match self.insert_record(*id, fields.clone()) {
                Ok(InsertOutcome::Inserted) => WriteResult::Inserted,
                Ok(InsertOutcome::Duplicate) => WriteResult::Duplicate,
                Err(code) => WriteResult::Failed(code),
            },
            WriteOp::Replace { id, fields } => match self.replace_record(*id, fields.clone()) {
                Ok(ReplaceOutcome::Replaced) => WriteResult::Replaced,
                Ok(ReplaceOutcome::NotFound) => WriteResult::NotFound,
                Err(code) => WriteResult::Failed(code),
            },
            WriteOp::ReplaceIf { id, fields, guard } => {
                match self.replace_record_if(*id, fields.clone(), guard) {
                    Ok(ReplaceIfOutcome::Replaced) => WriteResult::Replaced,
                    Ok(ReplaceIfOutcome::GuardFailed) => WriteResult::GuardFailed,
                    Ok(ReplaceIfOutcome::NotFound) => WriteResult::NotFound,
                    Err(code) => WriteResult::Failed(code),
                }
            }
            WriteOp::Delete { id } => match self.delete_record(*id) {
                Ok(DeleteOutcome::Deleted) => WriteResult::Deleted,
                Ok(DeleteOutcome::NotFound) => WriteResult::NotFound,
                Err(code) => WriteResult::Failed(code),
            },
            WriteOp::Link {
                left,
                right,
                relation,
            } => match self.link_records(*left, *right, relation) {
                Ok(LinkOutcome::Linked) => WriteResult::Linked,
                Ok(LinkOutcome::AlreadyLinked) => WriteResult::AlreadyLinked,
                Err(code) => WriteResult::Failed(code),
            },
        }
    }

    /// `WBT-FR-002`/`WBT-FR-003` (ADR-0060, protocol 22): a batch of
    /// runtime writes for this table. The default is **pipelined** —
    /// each op applied through [`Self::apply_write_op`], its outcome
    /// recorded, each standing on its own; `atomic` here can only abort
    /// on the first hard `Failed(code)` (a domain with no runtime write
    /// aborts at op 0 with `Unsupported`), because the default cannot
    /// hold one lock across the batch. An adapter that supports runtime
    /// writes overrides this to run the whole atomic batch under one
    /// exclusive section (`Memory`/`Entity`/`Relation`).
    fn write_batch(
        &self,
        ops: &[WriteOp],
        atomic: bool,
    ) -> Result<Vec<WriteResult>, (usize, ErrorCode)> {
        let mut results = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let result = self.apply_write_op(op);
            if atomic {
                if let WriteResult::Failed(code) = result {
                    return Err((i, code));
                }
            }
            results.push(result);
        }
        Ok(results)
    }

    /// `DEL-FR-006` (ADR-0051, protocol 17): remove the record at `id`
    /// and, within this table, every edge touching it. Answers
    /// [`DeleteOutcome::NotFound`] when `id` has no record, nothing
    /// written. The default answers `Unsupported`: `Dog`'s bespoke
    /// store, and `Order`/`Employee` as reference material; `Memory`,
    /// `Reminder`, and `Entity` implement it. A durability failure is
    /// [`ErrorCode::Storage`].
    fn delete_record(&self, _id: RecordId) -> Result<DeleteOutcome, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `DEL-FR-005` (ADR-0051): drop every edge under `relation` that
    /// touches `id`, where `id` is **another table's** record — the
    /// relation's descriptor names that table as `target_table` — and
    /// that table just deleted it. Returns the number of edges dropped;
    /// `Malformed` for a relation this table does not have. The server
    /// calls this on every other table after a `Delete` (`DEL-FR-007`).
    /// The default answers `Unsupported`; `Memory` implements it for
    /// `mentions`.
    fn detach_record(&self, _relation: &str, _id: RecordId) -> Result<usize, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `CMP-FR-006` (ADR-0052, protocol 18): compact this table's files in
    /// place under its write lock and report what was reclaimed. The
    /// default answers `Unsupported`: `Dog`'s bespoke store, and
    /// `Order`/`Employee` as reference material; `Memory`, `Reminder`,
    /// and `Entity` implement it. A file that could not be rewritten is
    /// [`ErrorCode::Storage`] — the files are left reopenable.
    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `BAK-FR-001` (ADR-0065, protocol 24): copy every file this
    /// table's on-disk stack owns into `target_dir` (already resolved,
    /// confinement-checked, and not yet at its final name — see
    /// `handle_connection`'s own `Request::Backup` arm), under the
    /// store's write lock so no concurrent write can land mid-copy. The
    /// default answers `Unsupported`: an adapter built with no known
    /// data directory (scratch-mode binaries, and any adapter that never
    /// calls its own `with_backup_source`). A copy failure is
    /// [`ErrorCode::Storage`]; the caller removes a partial `target_dir`
    /// on any `Err`.
    fn backup(&self, _target_dir: &Path) -> Result<BackupReport, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `RPL-FR-003`/`RPL-FR-007` (ADR-0067, protocol 25): read every file
    /// this table's on-disk stack owns into memory and return it as
    /// `(file name, bytes)` pairs, under the store's write lock (the
    /// same [`ConnectionStore::backup`] takes) so the read is
    /// lock-consistent with a concurrent write, never a partial mix of
    /// before/after bytes. The default answers `Unsupported` — the
    /// identical "no known data directory, no feature" posture
    /// [`ConnectionStore::backup`] already established; an adapter that
    /// implements `backup` also implements this over the same
    /// `backup_source`. `ErrorCode::TooLarge` if the table's total
    /// on-disk size exceeds [`super::protocol::MAX_SNAPSHOT_BYTES`],
    /// checked before any byte is read; `ErrorCode::Storage` for any
    /// other read failure.
    fn fetch_snapshot(&self) -> Result<Vec<(String, Vec<u8>)>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// This domain's schema, for a client that doesn't know it at compile
    /// time — ADR-0011. Infallible: every `ConnectionStore` implementor
    /// knows its own field/relation shape unconditionally, no store access
    /// needed.
    fn describe(&self) -> DomainSchema;

    /// `STV-FR-002` (`ADR-0024`'s second trigger): the precondition check
    /// [`ConnectionStore::apply_transaction`] would run for `op` alone —
    /// id exists, field known and updatable, value type matches — with no
    /// write, so a stage-time validating session can refuse the write
    /// now with the code `Commit` would have reported. Existence may
    /// change before `Commit` only in the direction this crate never
    /// takes (no runtime deletion), so `Ok` here is `Ok` at `Commit`.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode>;

    /// Apply every operation in `updates` atomically: every precondition
    /// (id exists, field known and updatable, value type matches) is
    /// checked before any write is applied; either every write in
    /// `updates` is applied, or none are. `Err((index, code))` names the
    /// first operation that failed its precondition check — see
    /// `docs/design/SERVER-TRANSACTION-DESIGN.md`, ADR-0013,
    /// `TXN-FR-002`/`TXN-FR-003`.
    ///
    /// `read_set` (`ISO-FR-006`, ADR-0033) is a snapshot-isolated session's
    /// tracked `(id, field) -> value` reads, empty when the session has
    /// snapshot isolation off (`SESSION_SNAPSHOT_ISOLATION` unset) — every
    /// entry is re-checked against current state inside the same exclusive
    /// section this method already applies writes under, atomically with
    /// that apply. Any mismatch fails the whole call with
    /// `(0, ErrorCode::Conflict)` before any write happens, the same
    /// sentinel-index shape a precondition failure from `updates` itself
    /// uses. See `docs/design/SERVER-SESSION-SNAPSHOT-ISOLATION-DESIGN.md`,
    /// `ISO-FR-002`.
    /// Every record's id and full field set, unspecified order — the
    /// `ScanField`-style "no meaningful order" convention, generalized
    /// from one field to all of them. The one new primitive
    /// `Request::Query` needs (`SQL-FR-004`, ADR-0034): unconditionally a
    /// full scan, no index — `dispatch`'s `Query` arm filters, projects,
    /// and limits the result centrally, the same way for every domain, so
    /// this method itself does neither. See
    /// `docs/design/SERVER-SQL-SELECT-DESIGN.md`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)>;

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)>;

    /// `MVCC2-FR-007`, `ADR-0072`: [`Self::apply_transaction`]'s own
    /// contract, for a table where a real-MVCC session
    /// (`SESSION_MVCC_ISOLATION`) is open — `mvcc_snapshot`'s write-write
    /// conflict check runs inside the *same* exclusive section as
    /// `read_set`'s check and the apply itself, never a separate call
    /// (which would reopen exactly the window `read_set`'s own check
    /// avoids). A **separate method, not an added parameter on
    /// [`Self::apply_transaction`]**, so `Dog`/`Order`/`Employee`/
    /// `Reminder`'s existing signature and every call site are untouched
    /// (`MVCC2-FR-012`). Only ever called when [`Self::mvcc_supported`]
    /// is `true`; the default is unreachable in practice and answers
    /// `Unsupported` defensively.
    fn apply_transaction_mvcc(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
        mvcc_snapshot: u64,
    ) -> Result<(), (usize, ErrorCode)> {
        let _ = (updates, read_set, mvcc_snapshot);
        Err((0, ErrorCode::Unsupported))
    }

    /// `MVCC2-FR-012`: whether this table implements real MVCC —
    /// `Memory`/`Entity`/`Relation` only. `BeginWith { flags:
    /// SESSION_MVCC_ISOLATION }` on any other table is refused
    /// `Unsupported`, the same domain-scoping precedent `Insert`/
    /// `Compact`/etc. already established.
    fn mvcc_supported(&self) -> bool {
        false
    }

    /// `MVCC2-FR-001`/`005`: activate this table for MVCC if this is its
    /// first-ever call (idempotent — seeds a baseline for every
    /// currently-live record, `MVCC2-FR-001`), then open a new snapshot
    /// and return its `snapshot_txn`. Only ever called when
    /// [`Self::mvcc_supported`] is `true`.
    fn mvcc_begin(&self) -> u64 {
        0
    }

    /// `Commit`/`Rollback`/disconnect: release one registration of
    /// `snapshot_txn` from the table's open-snapshot set (`MVCC2-FR-005`).
    fn mvcc_release(&self, _snapshot_txn: u64) {}

    /// `MVCC2-FR-006`: `id`'s value as of `snapshot_txn` — the same wire
    /// shape [`Self::get`] returns. `Err(ErrorCode::Conflict)` reused as
    /// the typed "history reclaimed" signal a snapshot below GC's
    /// boundary hits (distinct from `RecordNotFound`/`None`, matching
    /// `MVCC-SPIKE-DESIGN.md`'s own distinction). Only ever called when
    /// [`Self::mvcc_supported`] is `true`.
    fn mvcc_get(
        &self,
        _id: RecordId,
        _snapshot_txn: u64,
    ) -> Result<Option<Vec<(FieldRef, ScanValue)>>, ErrorCode> {
        Ok(None)
    }
}

/// `BAK-FR-006` (ADR-0065): every file a table's on-disk stack owns
/// shares one base path — `<dir>/memories.mmap`, `<dir>/memories.mmap.records`,
/// `<dir>/memories.mmap.inserts`, `<dir>/memories.mmap.relations`,
/// `<dir>/memories.mmap.<label>.edges`, and so on, every companion this
/// crate has ever added or will add sharing that one prefix
/// (`crate::generic::mmap_store::blob_path`,
/// `crate::generic::insert_log::log_path`, `crate::generic::store`'s own
/// label-manifest/edge-blob helpers). Rather than a `ConnectionStore`
/// implementor enumerating its own stack's exact file set by type — real
/// duplication of knowledge `crate::generic::query::Compact`'s own
/// per-layer implementations already carry, and silently incomplete the
/// day a future round adds one more companion file — this copies every
/// entry in `base`'s directory whose name starts with `base`'s own file
/// name, unconditionally. `target_dir` is created if missing; a file
/// that already exists there (a caller error, since `handle_connection`
/// always passes a fresh temporary directory) is overwritten.
pub(crate) fn copy_table_files(base: &Path, target_dir: &Path) -> io::Result<BackupReport> {
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{}: no file name to match companions against",
                base.display()
            ),
        )
    })?;
    let stem = stem.to_string_lossy();
    std::fs::create_dir_all(target_dir)?;
    let mut report = BackupReport::default();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(stem.as_ref()) {
            let bytes = std::fs::copy(entry.path(), target_dir.join(&name))?;
            report.files += 1;
            report.bytes += bytes;
        }
    }
    Ok(report)
}

/// [`copy_table_files`]'s failure — a plain `io::Error` uses `Storage`
/// for every failure kind, which cannot distinguish "this table is too
/// big to snapshot" (`ErrorCode::TooLarge`, refuse and try a real
/// backup instead) from "a file could not be read" (`ErrorCode::Storage`,
/// retry or investigate the disk). [`read_table_files`] returns this
/// instead so its caller can tell them apart. The underlying `io::Error`
/// is deliberately not carried — [`ConnectionStore::backup`]'s own
/// `io::Result` is discarded the same way at its own call site; neither
/// error is logged or surfaced to a client (`AUTH-FR-005`'s "never echo
/// internal detail back over the wire" posture, extended here to local
/// filesystem paths).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadTableFilesError {
    Io,
    TooLarge,
}

impl From<io::Error> for ReadTableFilesError {
    fn from(_: io::Error) -> Self {
        ReadTableFilesError::Io
    }
}

/// `RPL-FR-003`/`RPL-FR-007` (ADR-0067): [`copy_table_files`]'s reading
/// twin — every file `base`'s on-disk stack owns (the identical
/// starts-with-`base`'s-file-name match), read into memory as
/// `(file name, bytes)` pairs instead of copied to a target directory.
/// Every matched file's size is summed from its metadata and checked
/// against [`super::protocol::MAX_SNAPSHOT_BYTES`] **before any file's
/// bytes are read** (`RPL-FR-007`'s acceptance criterion: a table over
/// the ceiling is refused, never partially streamed) — `TooLarge` short-
/// circuits the whole call the moment the running total would exceed
/// the ceiling, so a huge single companion file is caught exactly as
/// early as many small ones would be.
pub(crate) fn read_table_files(base: &Path) -> Result<Vec<(String, Vec<u8>)>, ReadTableFilesError> {
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{}: no file name to match companions against",
                base.display()
            ),
        )
    })?;
    let stem = stem.to_string_lossy();
    let mut matched = Vec::new();
    let mut total_bytes: u64 = 0;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(stem.as_ref()) {
            continue;
        }
        total_bytes = total_bytes.saturating_add(entry.metadata()?.len());
        if total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(ReadTableFilesError::TooLarge);
        }
        matched.push((name.to_string_lossy().into_owned(), entry.path()));
    }
    let mut files = Vec::with_capacity(matched.len());
    for (name, path) in matched {
        files.push((name, std::fs::read(path)?));
    }
    Ok(files)
}

/// One message per [`ErrorCode`] variant — shared by [`err_response`] (a
/// single request's failure) and `dispatch`'s `Request::Transaction` arm
/// (a batch operation's failure, `Response::TransactionFailed`), so both
/// paths report the same wording for the same code.
fn error_message(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnknownField => "unrecognized field tag for this domain",
        ErrorCode::Unsupported => "this operation is not available for this field/domain",
        ErrorCode::Malformed => "the supplied value does not match this field's type",
        ErrorCode::Unauthenticated => "this connection has not presented a recognized token",
        ErrorCode::Unauthorized => "this connection's token does not permit this operation",
        ErrorCode::RecordNotFound => "this operation's id has no record",
        ErrorCode::NoSession => "no transaction session is open on this connection",
        ErrorCode::SessionOpen => "a transaction session is already open on this connection",
        ErrorCode::SessionFull => "this session already holds the maximum number of staged writes",
        ErrorCode::Journal => {
            "the batch could not be journaled before applying it; nothing was applied"
        }
        ErrorCode::Conflict => {
            "this session's read set no longer matches current state; nothing was applied"
        }
        ErrorCode::Duplicate => "a record with this id already exists; nothing was written",
        ErrorCode::Storage => "the record could not be made durable; nothing was written",
        ErrorCode::GuardFailed => {
            "the guard did not hold against the stored record; nothing was written"
        }
        ErrorCode::TooLarge => {
            "this table's on-disk size exceeds the snapshot size limit; nothing was read"
        }
    }
}

/// `RYW-FR-002` (ADR-0027): lay a read-your-writes session's staged
/// writes over a committed `Record`'s fields. For each field the record
/// carries, the *last* staged operation with this `id` and `field`
/// replaces the value — provided the staged value's kind equals the
/// committed one's and the field is one of `updatable` (the schema's
/// `update`-capable tags, read once at `BeginWith`). Everything else is
/// untouched: a missing id never gains a record (the caller only reaches
/// here with a `Record`), an absent field, a kind mismatch, or a
/// read-only field is ignored — each would fail at `Commit`, and the read
/// must not pretend otherwise. A pure function; linear in the buffer.
pub(crate) fn overlay_staged(
    id: RecordId,
    fields: &mut [(FieldRef, ScanValue)],
    staged: &[TransactionOp],
    updatable: &[FieldRef],
) {
    for (field, value) in fields.iter_mut() {
        if !updatable.contains(field) {
            continue;
        }
        if let Some(op) = staged
            .iter()
            .rev()
            .find(|op| op.id == id && op.field == *field)
        {
            if same_kind(&op.value, value) {
                *value = op.value.clone();
            }
        }
    }
}

fn same_kind(a: &ScanValue, b: &ScanValue) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// `ISO-FR-002`/`ISO-FR-004`/`ISO-FR-005` (ADR-0033): fold `id`'s raw,
/// committed `fields` — the exact result `dispatch` returned, before any
/// read-your-writes overlay — into a snapshot-isolated session's read
/// set. Each `(id, field)` key holds the most recently read value; past
/// `MAX_TRACKED_READS` distinct keys a *new* key is simply not added,
/// while an already-tracked key keeps updating on re-read — the read
/// never fails, and `Commit` still runs on whatever *was* tracked. A
/// pure function over the map, the same shape `overlay_staged` is over
/// `fields`.
pub(crate) fn record_read_set(
    reads: &mut HashMap<(RecordId, FieldRef), ScanValue>,
    id: RecordId,
    fields: &[(FieldRef, ScanValue)],
) {
    for (field, value) in fields {
        let key = (id, *field);
        if reads.contains_key(&key) || reads.len() < MAX_TRACKED_READS {
            reads.insert(key, value.clone());
        }
    }
}

/// `SQL-FR-007` (ADR-0034): every rejection `Request::Query` can produce,
/// checked against `schema` alone — before `ConnectionStore::scan_all`
/// ever runs, so a malformed query costs nothing beyond this. An unknown
/// tag in `select` or `filter` is `ErrorCode::UnknownField`; a predicate
/// whose value kind doesn't match its field's real type, or whose
/// comparator is an ordering one (`Lt`/`Le`/`Gt`/`Ge`) against a
/// `Str`/`Bool` field, is `ErrorCode::Malformed` — both existing codes,
/// no new wire addition.
fn validate_query(
    schema: &DomainSchema,
    select: &Selection,
    filter: &[Predicate],
) -> Result<(), ErrorCode> {
    let kind_of = |tag: FieldRef| {
        schema
            .fields
            .iter()
            .find(|f| f.tag == tag)
            .map(|f| f.value_kind)
    };
    if let Selection::Fields(fields) = select {
        for &tag in fields {
            if kind_of(tag).is_none() {
                return Err(ErrorCode::UnknownField);
            }
        }
    }
    for predicate in filter {
        validate_predicate(schema, predicate)?;
    }
    Ok(())
}

/// One predicate against a schema (`SQL-FR-007`; shared with
/// `Request::ReplaceIf`'s guard, `GRD-FR-004`): `UnknownField` for a tag
/// the schema lacks, `Malformed` for a value of another kind or an
/// ordering comparator on a field that is not `U32`/`I64`.
pub(crate) fn validate_predicate(
    schema: &DomainSchema,
    predicate: &Predicate,
) -> Result<(), ErrorCode> {
    let kind = schema
        .fields
        .iter()
        .find(|f| f.tag == predicate.field)
        .map(|f| f.value_kind)
        .ok_or(ErrorCode::UnknownField)?;
    if !value_matches_kind(kind, &predicate.value) {
        return Err(ErrorCode::Malformed);
    }
    let orderable_kind = matches!(kind, protocol::ValueKind::U32 | protocol::ValueKind::I64);
    if predicate.op.is_ordering() && !orderable_kind {
        return Err(ErrorCode::Malformed);
    }
    Ok(())
}

/// Deliberately no `StrList` arm (`ENT4-FR-002`, ADR-0041): a predicate
/// against a list-kinded field, whatever value it carries, is `Malformed`
/// — `aliases` is read-only over the wire; resolving *by* alias is
/// `FilterEq` on `label`.
fn value_matches_kind(kind: protocol::ValueKind, value: &ScanValue) -> bool {
    matches!(
        (kind, value),
        (protocol::ValueKind::U32, ScanValue::U32(_))
            | (protocol::ValueKind::I64, ScanValue::I64(_))
            | (protocol::ValueKind::Bool, ScanValue::Bool(_))
            | (protocol::ValueKind::Str, ScanValue::Str(_))
    )
}

/// `SQL-FR-006` (ADR-0034): filter, project, and limit `rows` — the one
/// place this logic is written, shared by every domain's
/// `Request::Query` (`ConnectionStore::scan_all` itself returns every
/// record's every field, unfiltered and unprojected). `filter`'s
/// predicates are `AND`-ed; `limit` truncates whatever order the input
/// arrived in, not a meaningful top-N (no `ORDER BY`).
fn evaluate_query(
    rows: Vec<(RecordId, Vec<(FieldRef, ScanValue)>)>,
    select: &Selection,
    filter: &[Predicate],
    limit: Option<usize>,
) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
    let mut matched: Vec<_> = rows
        .into_iter()
        .filter(|(_, fields)| filter.iter().all(|p| predicate_matches(fields, p)))
        .map(|(id, fields)| (id, select_fields(fields, select)))
        .collect();
    if let Some(limit) = limit {
        matched.truncate(limit);
    }
    matched
}

/// `QPL-FR-001` (ADR-0073): how `dispatch` fetches a `Request::Query`'s
/// candidate rows — the whole table, or one declared equality index's
/// bucket. `evaluate_query` re-checks every predicate over the result
/// either way, so the plan changes what is *read*, never what is
/// *returned*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryPlan {
    FullScan,
    /// Position in `filter` of the `Eq` predicate whose field's declared
    /// index narrows the read.
    IndexEq(usize),
    /// `QPR-FR-003` (ADR-0075): positions in `filter` of the predicates
    /// supplying each bound of a walk over the adapter's `range_field` —
    /// at least one `Some`; one `Eq` may supply both.
    IndexRange {
        lower: Option<usize>,
        upper: Option<usize>,
    },
    /// `QPI-FR-001` (ADR-0078): both — the equality index's bucket and
    /// the range walk's ids, intersected before any record is read;
    /// `eq` is what `IndexEq` would have carried, the bounds what
    /// `IndexRange` would have.
    IndexIntersect {
        eq: usize,
        lower: Option<usize>,
        upper: Option<usize>,
    },
}

/// `QPL-FR-001` (ADR-0073), `QPR-FR-003` (ADR-0075), `QPI-FR-001`
/// (ADR-0078): the plan for a `WHERE` — the first `Eq` predicate in wire
/// order whose field the schema declares `filter_eq: true` names the
/// bucket to read; independently, the first lower bound (`Gt`/`Ge`/
/// `Eq`) and the first upper bound (`Lt`/`Le`/`Eq`) on the adapter's
/// `range_field` name a walk. Both present → `IndexIntersect` (the
/// bucket's ids and the walk's ids, intersected, then read); the
/// bucket alone → `IndexEq`; the walk alone → `IndexRange`; neither →
/// `FullScan`. Since ADR-0078 the equality no longer *hides* a range
/// beside it — the one deliberate change to the equality-first rule,
/// and the result set is unchanged by construction since every
/// consumer re-checks every predicate. Since ADR-0083 each side is the
/// *tightest* bound ([`tightest_bounds`]), not the first in wire order
/// — the literals are compared to each other, never to the table, so
/// the plan stays deterministic and free of any cost model; the
/// intersection needs no estimate because both id lists are exact.
/// `validate_query` has already rejected unknown tags, so a tag
/// `schema` lacks simply never plans.
fn plan_query(
    schema: &DomainSchema,
    range_field: Option<FieldRef>,
    filter: &[Predicate],
) -> QueryPlan {
    let eq = filter.iter().position(|p| {
        p.op == protocol::CompareOp::Eq
            && schema
                .fields
                .iter()
                .any(|f| f.tag == p.field && f.capabilities.filter_eq)
    });
    let (lower, upper) = tightest_bounds(range_field, filter);
    match (eq, lower.is_some() || upper.is_some()) {
        (Some(eq), true) => QueryPlan::IndexIntersect { eq, lower, upper },
        (Some(eq), false) => QueryPlan::IndexEq(eq),
        (None, true) => QueryPlan::IndexRange { lower, upper },
        (None, false) => QueryPlan::FullScan,
    }
}

/// `QBT-FR-001` (ADR-0083): the positions in `filter` of the tightest
/// lower and tightest upper bound on `range_field` — the greatest lower
/// literal (an exclusive `Gt` tighter than an inclusive `Ge` at the same
/// literal) and the least upper literal (`Lt` tighter than `Le`), an
/// `Eq` a candidate for both sides. Exact, not an estimate: every bound
/// on one side is implied by the tightest, so a walk between the two
/// tightest admits exactly the records every bound admits. `None` for a
/// side with no bound, or when `range_field` is `None`. Literals of
/// another kind than the field's never reach here (`validate_predicate`);
/// two literals are compared as `i128` so `U32` and `I64` share one
/// order, as `page_key` does.
pub fn tightest_bounds(
    range_field: Option<FieldRef>,
    filter: &[Predicate],
) -> (Option<usize>, Option<usize>) {
    use protocol::CompareOp::{Eq, Ge, Gt, Le, Lt};
    let Some(field) = range_field else {
        return (None, None);
    };
    let literal = |p: &Predicate| page_key_value(&p.value).unwrap_or(i128::MIN);
    let mut lower: Option<usize> = None;
    let mut upper: Option<usize> = None;
    for (i, p) in filter.iter().enumerate() {
        if p.field != field {
            continue;
        }
        // (literal, exclusive): a greater pair is a tighter lower bound.
        if matches!(p.op, Gt | Ge | Eq) {
            let key = (literal(p), p.op == Gt);
            if lower.is_none_or(|j| key > (literal(&filter[j]), filter[j].op == Gt)) {
                lower = Some(i);
            }
        }
        // (literal, inclusive): a lesser pair is a tighter upper bound.
        if matches!(p.op, Lt | Le | Eq) {
            let key = (literal(p), p.op != Lt);
            if upper.is_none_or(|j| key < (literal(&filter[j]), filter[j].op != Lt)) {
                upper = Some(i);
            }
        }
    }
    (lower, upper)
}

/// `QPR-FR-004` (ADR-0075): the two `Bound`s a range walk takes, from the
/// supplying predicates' comparators — `Gt`/`Lt` exclusive, `Ge`/`Le`/
/// `Eq` inclusive, an absent side `Unbounded`. The literal is passed
/// through as is; `validate_predicate` already matched its kind to the
/// field's.
fn range_bounds(
    filter: &[Predicate],
    lower: Option<usize>,
    upper: Option<usize>,
) -> (Bound<ScanValue>, Bound<ScanValue>) {
    let bound = |i: Option<usize>| match i.map(|i| &filter[i]) {
        None => Bound::Unbounded,
        Some(p) if matches!(p.op, protocol::CompareOp::Gt | protocol::CompareOp::Lt) => {
            Bound::Excluded(p.value.clone())
        }
        Some(p) => Bound::Included(p.value.clone()),
    };
    (bound(lower), bound(upper))
}

/// `QPL-FR-002`/`QPL-FR-004` (ADR-0073) and `QPR-FR-004` (ADR-0075): the
/// rows `evaluate_query` will filter. `FullScan` is `scan_all` exactly
/// as before. `IndexEq(i)` asks the adapter's own `filter_eq` for
/// `filter[i]`'s bucket; `IndexRange` asks its `range_ids` for the walk
/// between the supplying predicates' bounds (`range_bounds`). Either
/// way each id is read back through `get`, dropping an id whose record
/// vanished in between — the identical per-id drop every adapter's
/// `scan_all` (`all_ids` then `get`) already performs, so the
/// consistency class is unchanged. A refusal on a field the adapter
/// claimed indexed (a `describe()`/`filter_eq` or `range_field`/
/// `range_ids` contract mismatch no shipped adapter has) falls back to
/// the full scan rather than surfacing: the error surface stays
/// validation's alone (`SQL-FR-007`). The index narrows what is read
/// only — the caller still runs every predicate, the supplying ones
/// included, over what comes back (`QPL-FR-003`/`QPR-FR-005`);
/// `Entity`'s `label` index is a normalized *superset* of exact `Eq`,
/// and that re-check is what keeps `Query`'s `Eq` exact on every plan.
fn query_candidates<S: ConnectionStore + ?Sized>(
    store: &S,
    plan: QueryPlan,
    filter: &[Predicate],
) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
    let bucket = |i: usize| store.filter_eq(filter[i].field, &filter[i].value);
    // `plan_query` never builds a range with neither side; a refusal is
    // the honest answer if one ever appears, and the caller scans.
    let bounds = |lower: Option<usize>, upper: Option<usize>| {
        lower
            .or(upper)
            .map(|i| (filter[i].field, range_bounds(filter, lower, upper)))
            .ok_or(ErrorCode::Unsupported)
    };
    let walk = |lower, upper| {
        let (field, (lower, upper)) = bounds(lower, upper)?;
        store.range_ids(field, lower, upper)
    };
    let ids = match plan {
        QueryPlan::FullScan => return store.scan_all(),
        QueryPlan::IndexEq(i) => bucket(i),
        QueryPlan::IndexRange { lower, upper } => walk(lower, upper),
        // `QPI-FR-002` (ADR-0078): both id lists, no record read for
        // either, intersected — the smaller hashed, the larger filtered,
        // so the order is the larger list's. Since `QPB-FR-003`
        // (ADR-0079) the walk carries a budget of `INTERSECT_WALK_BUDGET`
        // ids per bucket id: a range wider than that is abandoned at the
        // first id past it and the bucket alone is read — the worst case
        // is the bucket's cost plus a bounded id walk, not an unbounded
        // one. An empty bucket walks nothing. A walk refusal degrades to
        // the bucket alone (what `IndexEq` read before ADR-0078); a
        // bucket refusal to the scan, as `IndexEq`'s always did.
        QueryPlan::IndexIntersect { eq, lower, upper } => bucket(eq).map(|bucket| {
            if bucket.is_empty() {
                return bucket;
            }
            let budget = bucket.len().saturating_mul(INTERSECT_WALK_BUDGET);
            let walked = bounds(lower, upper).and_then(|(field, (lower, upper))| {
                store.range_ids_limited(field, lower, upper, budget)
            });
            match walked {
                Ok(Some(walked)) => intersect_ids(bucket, walked),
                Ok(None) | Err(_) => bucket,
            }
        }),
    };
    match ids {
        Ok(ids) => ids
            .into_iter()
            .filter_map(|id| store.get(id).map(|fields| (id, fields)))
            .collect(),
        Err(_) => store.scan_all(),
    }
}

/// `QPB-FR-003` (ADR-0079): how many range-walk ids the intersection
/// plan may visit per equality-bucket id before giving the walk up and
/// reading the bucket alone. Set from this crate's own measurement
/// (`RESULTS.md`, step four): a record decode costs ~1 µs on the
/// planner table, a `BTreeSet` id visit plus a hash lookup ~40 ns, so
/// the intersection stops paying for itself once the range holds
/// roughly twenty-five ids per bucket id; ten keeps the abandoned
/// walk's cost under half of one bucket read, so the worst case for a
/// filter carrying both indexes is ~1.4× the bucket alone, never the
/// range's whole id list. A constant, not a setting — named as this
/// crate's first cost ratio, and the one number a future cost model
/// would replace.
pub const INTERSECT_WALK_BUDGET: usize = 10;

/// `QPI-FR-002` (ADR-0078): the ids in both lists, in the larger list's
/// order — the smaller list is hashed, the larger walked once. Exact:
/// no estimate of either side is taken or needed, and the cost is the
/// two lists' lengths in id-level work, never a record decode.
pub fn intersect_ids(a: Vec<RecordId>, b: Vec<RecordId>) -> Vec<RecordId> {
    let (smaller, larger) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let members: std::collections::HashSet<RecordId> = smaller.into_iter().collect();
    larger
        .into_iter()
        .filter(|id| members.contains(id))
        .collect()
}

/// `QCW-FR-003` (ADR-0081): whether a `Request::Aggregate` is a count of
/// a range the `Ordered` index can answer without reading a record —
/// no `group_by`, every aggregate `COUNT(*)`, a range field, and a
/// filter that is nothing but bounds on it — any number per side since
/// `QBT-FR-002` (ADR-0083), the tightest implying the rest; an `Eq` is
/// both sides. An empty filter qualifies: the whole index. Any other
/// predicate (`Ne`, another field) needs a re-check over decoded rows,
/// so the walk's length would over-count. Pure.
pub fn counted_walk_applies(
    range_field: Option<FieldRef>,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
) -> bool {
    use protocol::CompareOp::{Eq, Ge, Gt, Le, Lt};
    let Some(field) = range_field else {
        return false;
    };
    if !group_by.is_empty() || aggregates.is_empty() {
        return false;
    }
    if !aggregates
        .iter()
        .all(|a| a.func == AggregateFn::Count && a.field.is_none())
    {
        return false;
    }
    // `QBT-FR-002` (ADR-0083): any number of bounds per side — the
    // tightest implies the rest, so the walk between the two tightest
    // is exact; only `Ne` and another field's predicate need a decode.
    filter
        .iter()
        .all(|p| p.field == field && matches!(p.op, Gt | Ge | Lt | Le | Eq))
}

/// `QRF-FR-002` (ADR-0087): one pass over a range's keys, reduced —
/// what every ungrouped reduction of the range field needs (`COUNT` the
/// count, `SUM` the sum, `AVG` their quotient, `MIN`/`MAX` the extremes)
/// and nothing more. `min`/`max` are `None` over an empty range, as the
/// keys would be absent. The sum is the same `i64` addition
/// `evaluate_aggregate`'s `Sum` performs over decoded rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyStats {
    pub count: u64,
    pub sum: i64,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

impl KeyStats {
    /// The fold step: `self` with one more key — for
    /// [`crate::generic::query::RangeBy::range_fold`], which visits keys
    /// ascending, so `min` is the first and `max` the latest.
    pub fn with(self, key: &i64) -> Self {
        Self {
            count: self.count + 1,
            sum: self.sum + *key,
            min: Some(self.min.map_or(*key, |m| m.min(*key))),
            max: Some(self.max.map_or(*key, |m| m.max(*key))),
        }
    }
}

/// `QKW-FR-003` (ADR-0082): whether a `Request::Aggregate` is answerable
/// from the walked keys alone — [`counted_walk_applies`]'s shape (a
/// range field, a filter of nothing but bounds on it), with every
/// aggregate either `COUNT(*)` or `SUM`/`AVG`/`MIN`/`MAX` *of the range
/// field itself*. Any aggregate over another field needs that field's
/// value, hence a decode. Since `QKG-FR-001` (ADR-0084) `group_by` may
/// also be exactly the range field: a group per distinct key is a run
/// of equal keys in the walk. Any other `group_by` needs a decode. Pure.
pub fn keyed_walk_applies(
    range_field: Option<FieldRef>,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
) -> bool {
    let Some(field) = range_field else {
        return false;
    };
    if !(group_by.is_empty() || group_by == [field]) {
        return false;
    }
    let over_the_key = |a: &AggregateSpec| match a.func {
        AggregateFn::Count => a.field.is_none(),
        AggregateFn::Sum | AggregateFn::Avg | AggregateFn::Min | AggregateFn::Max => {
            a.field == Some(field)
        }
    };
    let counts_only: Vec<AggregateSpec> = aggregates
        .iter()
        .filter(|a| over_the_key(a))
        .map(|_| AggregateSpec {
            func: AggregateFn::Count,
            field: None,
        })
        .collect();
    counts_only.len() == aggregates.len()
        && counted_walk_applies(range_field, &[], filter, &counts_only)
}

/// `QKW-FR-004` (ADR-0082): the answer to an eligible keyed aggregate —
/// [`counted_walk`] when every column is `COUNT(*)` (no key
/// materialized), otherwise the walk's keys once
/// ([`ConnectionStore::range_keys`]) and each column reduced over them
/// exactly as [`evaluate_aggregate`] reduces decoded rows: `COUNT` the
/// length; `SUM` the `i64` sum; `AVG` an `F64` mean, `0.0` over nothing;
/// `MIN`/`MAX` the extreme in the field's own kind, `0` over nothing —
/// the same one-group, `limit`-truncated shape; since `QRF-FR-003`
/// (ADR-0087) that one group comes from [`ConnectionStore::range_stats`],
/// a one-pass fold with no key materialized. Since `QKG-FR-002`
/// (ADR-0084) a `group_by` of the range field yields one group per run
/// of equal keys, keyed by that key in the field's kind, ascending —
/// the decode path's own groups whenever a bound is present (its
/// candidates are the same walk); with no bound the decode path
/// buckets in scan order, so the groups are the same set in another
/// order, and a `limit` there would truncate a different set: that one
/// shape decodes (`QKG-FR-003`). `None` when ineligible
/// ([`keyed_walk_applies`]) or refused, so the caller decodes.
pub fn keyed_walk<S: ConnectionStore + ?Sized>(
    store: &S,
    schema: &DomainSchema,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
    limit: Option<usize>,
) -> Option<Vec<AggregateGroup>> {
    let field = store.range_field()?;
    if !keyed_walk_applies(Some(field), group_by, filter, aggregates) {
        return None;
    }
    let grouped = !group_by.is_empty();
    if !grouped && aggregates.iter().all(|a| a.func == AggregateFn::Count) {
        return counted_walk(store, group_by, filter, aggregates, limit);
    }
    if grouped && filter.is_empty() && limit.is_some() {
        return None;
    }
    let (lower, upper) = tightest_bounds(Some(field), filter);
    let (lower, upper) = range_bounds(filter, lower, upper);
    let kind = schema
        .fields
        .iter()
        .find(|f| f.tag == field)
        .map(|f| f.value_kind)?;
    let render = |v: i64| match kind {
        protocol::ValueKind::U32 => ScanValue::U32(v as u32),
        _ => ScanValue::I64(v),
    };
    // `QRF-FR-003` (ADR-0087): every reduction from one `KeyStats` —
    // over a run of equal keys when grouped, over the whole walk's
    // one-pass fold when not, so the ungrouped shape materializes
    // nothing.
    let reduce = |stats: KeyStats| -> Vec<ScanValue> {
        aggregates
            .iter()
            .map(|a| match a.func {
                AggregateFn::Count => ScanValue::I64(stats.count as i64),
                AggregateFn::Sum => ScanValue::I64(stats.sum),
                AggregateFn::Avg if stats.count == 0 => ScanValue::F64(0.0),
                AggregateFn::Avg => ScanValue::F64(stats.sum as f64 / stats.count as f64),
                AggregateFn::Min => render(stats.min.unwrap_or(0)),
                AggregateFn::Max => render(stats.max.unwrap_or(0)),
            })
            .collect()
    };
    let mut groups: Vec<AggregateGroup> = if grouped {
        let keys = store.range_keys(field, lower, upper).ok()?;
        keys.chunk_by(|a, b| a == b)
            .map(|run| AggregateGroup {
                key: vec![(field, render(run[0]))],
                values: reduce(run.iter().fold(KeyStats::default(), KeyStats::with)),
            })
            .collect()
    } else {
        let stats = store.range_stats(field, lower, upper).ok()?;
        vec![AggregateGroup {
            key: Vec::new(),
            values: reduce(stats),
        }]
    };
    if let Some(limit) = limit {
        groups.truncate(limit);
    }
    Some(groups)
}

/// `QCW-FR-004` (ADR-0081): the answer to an eligible count — the index's
/// own count between the filter's bounds ([`ConnectionStore::range_count`]),
/// once per `COUNT(*)` column, under the same one-group, `limit`-truncated
/// shape [`evaluate_aggregate`] gives a `group_by`-less request. `None`
/// when the request is not eligible ([`counted_walk_applies`]) or the
/// adapter refuses the count (a declared range field whose `range_count`
/// answers `Unsupported` — no shipped adapter), so the caller takes the
/// decode path: the answer is the same either way, only the reading
/// differs.
pub fn counted_walk<S: ConnectionStore + ?Sized>(
    store: &S,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
    limit: Option<usize>,
) -> Option<Vec<AggregateGroup>> {
    let field = store.range_field()?;
    if !counted_walk_applies(Some(field), group_by, filter, aggregates) {
        return None;
    }
    let (lower, upper) = tightest_bounds(Some(field), filter);
    let (lower, upper) = range_bounds(filter, lower, upper);
    let count = store.range_count(field, lower, upper).ok()?;
    let mut groups = vec![AggregateGroup {
        key: Vec::new(),
        values: aggregates
            .iter()
            .map(|_| ScanValue::I64(count as i64))
            .collect(),
    }];
    if let Some(limit) = limit {
        groups.truncate(limit);
    }
    Some(groups)
}

/// `QPC-FR-001` (ADR-0074): the one candidate step every filtered read
/// shares — [`plan_query`] then [`query_candidates`] — so `Query`,
/// `Aggregate`, the default [`ConnectionStore::filtered_page`], and
/// `Join`'s left side all narrow the same way and none narrows
/// differently. Since `QPR-FR-004` (ADR-0075) the plan also sees the
/// adapter's [`ConnectionStore::range_field`], so all four gain the
/// range walk here with no call-site change. Callers keep re-checking
/// every predicate over the result; this only decides what is read.
fn indexed_candidates<S: ConnectionStore + ?Sized>(
    store: &S,
    schema: &DomainSchema,
    filter: &[Predicate],
) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
    query_candidates(
        store,
        plan_query(schema, store.range_field(), filter),
        filter,
    )
}

/// `PAG-FR-003` (ADR-0055): `Request::Page`'s validation, before any
/// scan — `UnknownField` for an `order_by` the schema lacks, `Malformed`
/// for a field that is not `U32`/`I64` (the one kind pair with an order,
/// `CompareOp::is_ordering`'s rule), a cursor value of another kind, or
/// a zero `limit`.
fn validate_page(
    schema: &DomainSchema,
    order_by: FieldRef,
    after: Option<&(ScanValue, RecordId)>,
    limit: u64,
) -> Result<(), ErrorCode> {
    let kind = schema
        .fields
        .iter()
        .find(|f| f.tag == order_by)
        .map(|f| f.value_kind)
        .ok_or(ErrorCode::UnknownField)?;
    if !matches!(kind, protocol::ValueKind::U32 | protocol::ValueKind::I64) {
        return Err(ErrorCode::Malformed);
    }
    if let Some((value, _)) = after {
        if !value_matches_kind(kind, value) {
            return Err(ErrorCode::Malformed);
        }
    }
    if limit == 0 {
        return Err(ErrorCode::Malformed);
    }
    Ok(())
}

/// `FPG-FR-002` (ADR-0068): `Request::FilteredPage`'s validation, before
/// any scan — composed from the two existing checks, not a new rule
/// set: [`validate_page`]'s four checks over `order_by`/`after`/`limit`,
/// then every `filter` predicate through [`validate_predicate`] (the
/// identical check `Request::Query`'s own filter already uses).
fn validate_filtered_page(
    schema: &DomainSchema,
    order_by: FieldRef,
    after: Option<&(ScanValue, RecordId)>,
    limit: u64,
    filter: &[Predicate],
) -> Result<(), ErrorCode> {
    validate_page(schema, order_by, after, limit)?;
    filter
        .iter()
        .try_for_each(|p| validate_predicate(schema, p))
}

/// The sort key one row contributes to a page: its `order_by` value as
/// an `i128` (so `U32` and `I64` share one order) and its id. A row
/// without the field, or with one of another kind, sorts first — a
/// state `validate_page` has already ruled out for every adapter whose
/// `scan_all` describes its own schema.
pub fn page_key(
    fields: &[(FieldRef, ScanValue)],
    order_by: FieldRef,
    id: RecordId,
) -> (i128, RecordId) {
    let value = fields
        .iter()
        .find(|(field, _)| *field == order_by)
        .and_then(|(_, value)| page_key_value(value))
        .unwrap_or(i128::MIN);
    (value, id)
}

/// One `order_by` value as [`page_key`]'s `i128`: `U32` and `I64` share
/// the order, any other kind has none.
pub fn page_key_value(value: &ScanValue) -> Option<i128> {
    match value {
        ScanValue::U32(v) => Some(i128::from(*v)),
        ScanValue::I64(v) => Some(i128::from(*v)),
        _ => None,
    }
}

/// `PAG-FR-002` (ADR-0055): the ids of one ordered keyset page over
/// `keys` — ascending by `(key, id)`, every entry strictly greater than
/// `after`'s key, the first `limit` of them. The one place the order is
/// written, shared by [`page_rows`] and every adapter's default
/// [`ConnectionStore::page`]. Selects the page in one pass over the keys
/// (`select_nth_unstable` then a sort of the page) rather than sorting
/// every key, so the cost past the scan is linear in the table.
pub fn page_ids(
    keys: Vec<(RecordId, i128)>,
    after: Option<(ScanValue, RecordId)>,
    limit: usize,
) -> Vec<RecordId> {
    let cursor = after.map(|(value, id)| (page_key_value(&value).unwrap_or(i128::MIN), id));
    let mut keyed: Vec<(i128, RecordId)> = keys
        .into_iter()
        .map(|(id, key)| (key, id))
        .filter(|key| match cursor {
            None => true,
            Some(cursor) => *key > cursor,
        })
        .collect();
    if limit == 0 {
        return Vec::new();
    }
    if limit < keyed.len() {
        keyed.select_nth_unstable(limit);
        keyed.truncate(limit);
    }
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, id)| id).collect()
}

/// The trait default of [`ConnectionStore::page`] as a free function, so
/// an adapter that answers one field from a sorted index (`ORD-FR-005`,
/// ADR-0059) can fall back to the scan for every other field:
/// [`ConnectionStore::page_keys`] for every record, [`page_ids`] to pick
/// the page, [`ConnectionStore::get`] for just those rows.
pub fn page_by_scan<S: ConnectionStore + ?Sized>(
    store: &S,
    order_by: FieldRef,
    after: Option<(ScanValue, RecordId)>,
    limit: usize,
) -> Vec<PageRow> {
    let winners = page_ids(store.page_keys(order_by), after, limit);
    winners
        .into_iter()
        .filter_map(|id| store.get(id).map(|fields| (id, fields)))
        .collect()
}

/// `PAG-FR-002` (ADR-0055): one ordered keyset page over already
/// materialized `rows` — [`page_ids`] over their keys, then those rows
/// in that order. For an adapter (or test) that holds every row already;
/// the trait default pages by key first and materializes only the page.
pub fn page_rows(
    rows: Vec<PageRow>,
    order_by: FieldRef,
    after: Option<(ScanValue, RecordId)>,
    limit: usize,
) -> Vec<PageRow> {
    let keys = rows
        .iter()
        .map(|(id, fields)| (*id, page_key(fields, order_by, *id).0))
        .collect();
    let winners = page_ids(keys, after, limit);
    let mut by_id: std::collections::HashMap<RecordId, Vec<(FieldRef, ScanValue)>> =
        rows.into_iter().collect();
    winners
        .into_iter()
        .filter_map(|id| by_id.remove(&id).map(|fields| (id, fields)))
        .collect()
}

/// `FPG-FR-003`/`QPC-FR-003`: the [`ConnectionStore::filtered_page`]
/// default's body as a free function, so an adapter that overrides the
/// method for one shape (`ADR-0076`'s bounded walk) can answer every
/// other shape exactly as the default would — a Rust default body cannot
/// be called from its override. Candidates through [`indexed_candidates`]
/// (the equality bucket or the range walk when the filter allows,
/// `scan_all` otherwise), every predicate re-checked, then [`page_rows`].
pub fn filtered_page_by_candidates<S: ConnectionStore + ?Sized>(
    store: &S,
    order_by: FieldRef,
    after: Option<(ScanValue, RecordId)>,
    limit: usize,
    filter: &[Predicate],
) -> Vec<PageRow> {
    let filtered: Vec<PageRow> = indexed_candidates(store, &store.describe(), filter)
        .into_iter()
        .filter(|(_, fields)| filter.iter().all(|p| predicate_matches(fields, p)))
        .collect();
    page_rows(filtered, order_by, after, limit)
}

/// `FPW-FR-001` (ADR-0076) as widened by `FPM-FR-001` (ADR-0077): whether
/// a `FilteredPage` can be answered by the bounded walk — `order_by` is
/// the adapter's range field, and the filter does not plan the declared
/// equality index ([`plan_query`]'s own equality-first rule: a request
/// that read a `filter_eq` bucket before this round still does — since
/// `QPI-FR-003` (ADR-0078) intersected with the range's ids — so the
/// walk reaches only filters that walked or scanned every in-range
/// record). Any other predicate — a bound on the walked field, a
/// comparison on another field, `Ne` on either — is the walk's to cut
/// or pass ([`bounded_filtered_page`]). An empty filter qualifies (it is
/// the unfiltered `Page`). Pure over its inputs.
pub fn bounded_walk_applies(
    order_by: FieldRef,
    range_field: Option<FieldRef>,
    schema: &DomainSchema,
    filter: &[Predicate],
) -> bool {
    range_field == Some(order_by)
        && matches!(
            plan_query(schema, range_field, filter),
            QueryPlan::FullScan | QueryPlan::IndexRange { .. }
        )
}

/// `FPW-FR-002` (ADR-0076): the pair the bounded walk starts strictly
/// after — the later, in `(key, id)` order, of the client's cursor and
/// every lower-bound predicate's own cursor: `key > k` is `(k, max)`;
/// `key >= k` and `key = k` are `(k - 1, max)` (nothing lies strictly
/// between `(k - 1, max)` and `(k, nil)`), or no cursor at all when `k`
/// is `i64::MIN`. `Lt`/`Le` contribute nothing (they are the cut,
/// [`bounded_filtered_page`]); so does any predicate on a field other
/// than `order_by` (since `FPM-FR-002`, ADR-0077, those are the walk's
/// rejects). `None` when nothing applies. Taking the tightest lower
/// bound is exact, not a cost estimate — every bound is on the one
/// walked key. `Malformed` for a non-`I64` literal on that key or a
/// non-`I64` cursor value; unreachable through `dispatch`, which
/// validated both.
pub fn bounded_walk_start(
    order_by: FieldRef,
    after: Option<(ScanValue, RecordId)>,
    filter: &[Predicate],
) -> Result<Option<(i64, RecordId)>, ErrorCode> {
    let key = |value: &ScanValue| match value {
        ScanValue::I64(k) => Ok(*k),
        _ => Err(ErrorCode::Malformed),
    };
    let mut start: Option<(i64, RecordId)> = match after {
        None => None,
        Some((value, id)) => Some((key(&value)?, id)),
    };
    for predicate in filter.iter().filter(|p| p.field == order_by) {
        let cursor = match predicate.op {
            CompareOp::Gt => Some((key(&predicate.value)?, RecordId::max())),
            CompareOp::Ge | CompareOp::Eq => key(&predicate.value)?
                .checked_sub(1)
                .map(|k| (k, RecordId::max())),
            CompareOp::Lt | CompareOp::Le | CompareOp::Ne => None,
        };
        if let Some(cursor) = cursor {
            start = Some(match start {
                Some(current) if current >= cursor => current,
                _ => cursor,
            });
        }
    }
    Ok(start)
}

/// `FPM-FR-002` (ADR-0077): the walked key of one record as read back —
/// its `order_by` value, the `I64` every range field carries — so the
/// walk can resume strictly after the last record it read. `Malformed`
/// for a record lacking the field or carrying another kind; unreachable
/// for an adapter's own records, present so the helper is honest as
/// surface.
pub fn walked_key(fields: &[(FieldRef, ScanValue)], order_by: FieldRef) -> Result<i64, ErrorCode> {
    match fields.iter().find(|(field, _)| *field == order_by) {
        Some((_, ScanValue::I64(key))) => Ok(*key),
        _ => Err(ErrorCode::Malformed),
    }
}

/// `FPW-FR-003` (ADR-0076) extended by `FPM-FR-002`/`003` (ADR-0077): the
/// bounded walk — [`bounded_walk_start`], then chunks of `limit` pairs
/// through `walk` (the adapter's own `page_by` over its `Ordered`
/// index), each id read back through `get`, until the page holds `limit`
/// matching rows, a *cut* predicate rejects a row, or the index ends.
/// A cut predicate is a non-`Ne` bound on the walked field: the walk is
/// ascending on that very key, so the first row one rejects ends the
/// page and no later row could match. Every other predicate — another
/// field's, or `Ne` on the walked field — is a *reject*: a row failing
/// only those is skipped and the walk continues, resuming strictly
/// after the last `(key, id)` read. For a filter of bounds alone (every
/// predicate a cut) the page is one chunk: at most `limit` pairs walked
/// and records read, however many the bounds admit — `ADR-0076`'s own
/// cost. For a mixed filter the cost is the page divided by the
/// rejects' selectivity — O(page) typically, every in-range record in
/// the worst case, never more than the default read.
///
/// `Page`'s consistency class (`ORD-FR-005`): an id whose record
/// vanished between the walk and its read is skipped, and the walk goes
/// on — the page is back-filled from the next pair, no longer shortened;
/// a record re-keyed in between is read at its new key, which becomes
/// the resume point if later. A chunk that advances the cursor by
/// nothing (every id vanished, or re-keyed to before the cursor) ends
/// the page rather than walk the same pairs again. The caller has
/// already checked [`bounded_walk_applies`].
pub fn bounded_filtered_page<S: ConnectionStore + ?Sized>(
    store: &S,
    walk: impl Fn(Option<(i64, RecordId)>, usize) -> Vec<RecordId>,
    order_by: FieldRef,
    after: Option<(ScanValue, RecordId)>,
    limit: usize,
    filter: &[Predicate],
) -> Result<Vec<PageRow>, ErrorCode> {
    let is_cut = |p: &&Predicate| p.field == order_by && p.op != CompareOp::Ne;
    let mut cursor = bounded_walk_start(order_by, after, filter)?;
    let mut page: Vec<PageRow> = Vec::with_capacity(limit);
    loop {
        let ids = walk(cursor, limit);
        let exhausted = ids.len() < limit;
        let mut advanced = false;
        for id in ids {
            let Some(fields) = store.get(id) else {
                continue;
            };
            if !filter
                .iter()
                .filter(is_cut)
                .all(|p| predicate_matches(&fields, p))
            {
                return Ok(page);
            }
            let pair = (walked_key(&fields, order_by)?, id);
            if cursor.is_none_or(|current| pair > current) {
                cursor = Some(pair);
                advanced = true;
            }
            if !filter.iter().all(|p| predicate_matches(&fields, p)) {
                continue;
            }
            page.push((id, fields));
            if page.len() == limit {
                return Ok(page);
            }
        }
        if exhausted || !advanced {
            return Ok(page);
        }
    }
}

/// Whether `predicate` holds over one record's wire shape — a `Query`
/// filter's per-row test, and (`GRD-FR-003`) the evaluation of a
/// `ReplaceIf` guard against the stored record inside an adapter.
pub fn predicate_matches(fields: &[(FieldRef, ScanValue)], predicate: &Predicate) -> bool {
    fields
        .iter()
        .find(|(field, _)| *field == predicate.field)
        .is_some_and(|(_, value)| compare(value, predicate.op, &predicate.value))
}

/// The two `(key, id)` bounds of one `Ordered` walk over a `Uuid`-keyed,
/// `i64`-ordered record — what [`uuid_pair_bounds`] produces and
/// `GenericProductionStore::range_by` takes.
pub type UuidPairBounds = (Bound<(i64, RecordId)>, Bound<(i64, RecordId)>);

/// `QPR-FR-002` (ADR-0075): the pair bounds an `Ordered<_, R, _>` walk
/// takes, from the key bounds `dispatch` hands a `Uuid`-keyed adapter —
/// "key ≥ k" is `Included((k, nil))`, "key > k" is `Excluded((k, max))`,
/// "key ≤ k" is `Included((k, max))`, "key < k" is `Excluded((k, nil))`,
/// exact because every real id lies within `nil()..=max()`. The generic
/// layer's `RangeBy` takes pair bounds and knows no sentinel; this is
/// where the id type is known. `Malformed` for a bound whose value is
/// not `I64` — the kind both shipped range fields carry, and the one
/// `validate_predicate` already guaranteed through `dispatch`.
pub fn uuid_pair_bounds(
    lower: Bound<ScanValue>,
    upper: Bound<ScanValue>,
) -> Result<UuidPairBounds, ErrorCode> {
    let key = |value: ScanValue| match value {
        ScanValue::I64(k) => Ok(k),
        _ => Err(ErrorCode::Malformed),
    };
    let lower = match lower {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(v) => Bound::Included((key(v)?, RecordId::nil())),
        Bound::Excluded(v) => Bound::Excluded((key(v)?, RecordId::max())),
    };
    let upper = match upper {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(v) => Bound::Included((key(v)?, RecordId::max())),
        Bound::Excluded(v) => Bound::Excluded((key(v)?, RecordId::nil())),
    };
    Ok((lower, upper))
}

/// `validate_query` already refused an ordering comparator against a
/// `Str`/`Bool` field before this ever runs, so the `_ => false` arm
/// below is unreachable through `dispatch` — kept as a safe default
/// rather than a `match` that could panic if that invariant ever broke.
fn compare(actual: &ScanValue, op: CompareOp, expected: &ScanValue) -> bool {
    match op {
        CompareOp::Eq => actual == expected,
        CompareOp::Ne => actual != expected,
        CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => match (actual, expected) {
            (ScanValue::U32(a), ScanValue::U32(b)) => ordering_matches(a.cmp(b), op),
            (ScanValue::I64(a), ScanValue::I64(b)) => ordering_matches(a.cmp(b), op),
            _ => false,
        },
    }
}

fn ordering_matches(ordering: std::cmp::Ordering, op: CompareOp) -> bool {
    use std::cmp::Ordering::{Equal, Greater, Less};
    matches!(
        (op, ordering),
        (CompareOp::Lt, Less)
            | (CompareOp::Gt, Greater)
            | (CompareOp::Le, Less | Equal)
            | (CompareOp::Ge, Greater | Equal)
    )
}

fn select_fields(
    fields: Vec<(FieldRef, ScanValue)>,
    select: &Selection,
) -> Vec<(FieldRef, ScanValue)> {
    match select {
        Selection::All => fields,
        Selection::Fields(wanted) => fields
            .into_iter()
            .filter(|(field, _)| wanted.contains(field))
            .collect(),
    }
}

/// The conservative [`ConnectionStore::describe_relations`] default —
/// `JOIN-FR-002` (ADR-0044): `neighbors` and one entry per symmetric
/// label when the schema reports `relations.neighbors`; nothing for
/// `parent`/`children`, which an adapter must claim explicitly (see the
/// trait method's own doc for why). Every entry is `target_table: None`.
/// Public so an adapter's override can extend it rather than restate it.
pub fn default_relation_descriptors(
    schema: &DomainSchema,
    labels: Vec<String>,
) -> Vec<RelationDescriptor> {
    let mut out = Vec::new();
    if schema.relations.neighbors {
        out.push(RelationDescriptor {
            name: "neighbors".to_string(),
            kind: JoinRelation::Neighbors(None),
            target_table: None,
        });
        for label in labels {
            out.push(RelationDescriptor {
                kind: JoinRelation::Neighbors(Some(label.clone())),
                name: label,
                target_table: None,
            });
        }
    }
    out
}

/// `JOIN-FR-003` (ADR-0044): every rejection `Request::Join` can produce,
/// checked against `schema` and `relations` alone — before `scan_all`
/// ever runs. Both sides are validated exactly as `validate_query` does
/// (`UnknownField`/`Malformed`); `right_table: Some(_)` is `Malformed`
/// until ADR-0045 lands; a relation the adapter does not list is
/// `Malformed`; one it lists with a `target_table` (another table's
/// rows) is `Unsupported` while `right_table` is `None`. No new
/// `ErrorCode`.
fn validate_join(
    schema: &DomainSchema,
    relations: &[RelationDescriptor],
    right_schema: Option<&DomainSchema>,
    spec: &JoinSpec,
) -> Result<(), ErrorCode> {
    validate_query(schema, &spec.left, &spec.left_filter)?;
    let relation = relations
        .iter()
        .find(|r| r.kind == spec.relation)
        .ok_or(ErrorCode::Malformed)?;
    match (&spec.right_table, &relation.target_table) {
        // Within one table: the right rows are this table's.
        (None, None) => validate_query(schema, &spec.right, &spec.right_filter),
        // The relation's rows live elsewhere and the caller did not say
        // where — `Unsupported`, as since `JOIN-FR-003`.
        (None, Some(_)) => Err(ErrorCode::Unsupported),
        // `TBL-FR-004` (ADR-0050): a cross-table join must name exactly
        // the table the descriptor names, and that table must be one
        // this server registered (`right_schema` is `Some`).
        (Some(named), Some(target)) if named == target => {
            let right = right_schema.ok_or(ErrorCode::Malformed)?;
            validate_query(right, &spec.right, &spec.right_filter)
        }
        (Some(_), _) => Err(ErrorCode::Malformed),
    }
}

/// `JOIN-FR-003` (ADR-0044): the index nested loop — for each `scan_all`
/// row passing `left_filter`, the related ids through the adapter's own
/// relation method, one `get` per id, `right_filter`, both projections,
/// one [`JoinedRow`]. `limit` truncates the pair count *and* stops the
/// loop — the one place a join bounds work as well as response,
/// deliberate because the pair count is not knowable up front. A
/// symmetric relation yields both orientations of an edge (SQL
/// semantics, `JOIN-FR-004`). The `Err` arms of the relation calls are
/// unreachable through `dispatch` (`validate_join` refused any relation
/// the adapter does not list) and fall back to "no related rows" rather
/// than panicking — the `compare` precedent.
fn evaluate_join<L: ConnectionStore + ?Sized, R: ConnectionStore + ?Sized>(
    store: &L,
    right_store: &R,
    spec: &JoinSpec,
) -> Vec<JoinedRow> {
    let mut out = Vec::new();
    let limit = spec.limit.unwrap_or(usize::MAX);
    if limit == 0 {
        return out;
    }
    // `QPC-FR-004` (ADR-0074): the left side is `Query`'s own candidate
    // step — the left filter's declared equality index when it has one,
    // `scan_all` otherwise — planned against the *left* store's schema,
    // which is the one `left_filter` was validated against (a cross-table
    // join's right store is a different table). The re-check below stays:
    // it is what keeps a superset index (`Entity::label`) exact.
    for (left_id, left_fields) in indexed_candidates(store, &store.describe(), &spec.left_filter) {
        if !spec
            .left_filter
            .iter()
            .all(|p| predicate_matches(&left_fields, p))
        {
            continue;
        }
        let right_ids: Vec<RecordId> = match &spec.relation {
            JoinRelation::Neighbors(None) => store.neighbors(left_id).unwrap_or_default(),
            JoinRelation::Neighbors(Some(label)) => store
                .neighbors_by_relation(left_id, label)
                .unwrap_or_default(),
            JoinRelation::Parent => match store.parent(left_id) {
                Ok(ParentLookup::Parent(parent_id)) => vec![parent_id],
                _ => vec![],
            },
            JoinRelation::Children => store.children(left_id).unwrap_or_default(),
        };
        for right_id in right_ids {
            // `TBL-FR-004`: the right rows come from `right_store` — the
            // same adapter within one table, another table's across.
            let Some(right_fields) = right_store.get(right_id) else {
                continue;
            };
            if !spec
                .right_filter
                .iter()
                .all(|p| predicate_matches(&right_fields, p))
            {
                continue;
            }
            out.push(JoinedRow {
                left_id,
                left: select_fields(left_fields.clone(), &spec.left),
                right_id,
                right: select_fields(right_fields, &spec.right),
            });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// `AGG-FR-006` (ADR-0035): every rejection `Request::Aggregate` can
/// produce, checked against `schema` alone — before
/// `ConnectionStore::scan_all` ever runs. `filter` is validated exactly
/// like `Request::Query`'s own filter (`validate_query`'s identical
/// checks). An unknown tag in `group_by` or an `AggregateSpec::field` is
/// `ErrorCode::UnknownField`; a `StrList`-kinded tag in `group_by` is
/// `ErrorCode::Malformed` (`ENT4-FR-003`, ADR-0041 — a list is never a
/// group key, so `Response::Groups` never carries one); a `Count` whose `field` is `Some(_)` (only
/// `COUNT(*)` is supported — this schema has no `NULL` concept, so
/// `COUNT(field)` would be unconditionally identical), or a
/// `Sum`/`Avg`/`Min`/`Max` with no field or a non-`U32`/`I64` field (the
/// identical "orderable kind" rule `CompareOp::is_ordering` already
/// established), is `ErrorCode::Malformed`.
fn validate_aggregate(
    schema: &DomainSchema,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
) -> Result<(), ErrorCode> {
    let kind_of = |tag: FieldRef| {
        schema
            .fields
            .iter()
            .find(|f| f.tag == tag)
            .map(|f| f.value_kind)
    };
    for &tag in group_by {
        match kind_of(tag) {
            None => return Err(ErrorCode::UnknownField),
            // `ENT4-FR-003` (ADR-0041): a `StrList` field is not groupable
            // — the one way a `StrList` could otherwise reach
            // `Response::Groups`, which `downgrade_for_version` cannot
            // strip. `Malformed`, the code every other kind-shaped
            // aggregate rejection already uses.
            Some(protocol::ValueKind::StrList) => return Err(ErrorCode::Malformed),
            Some(_) => {}
        }
    }
    for predicate in filter {
        let kind = kind_of(predicate.field).ok_or(ErrorCode::UnknownField)?;
        if !value_matches_kind(kind, &predicate.value) {
            return Err(ErrorCode::Malformed);
        }
        let orderable_kind = matches!(kind, protocol::ValueKind::U32 | protocol::ValueKind::I64);
        if predicate.op.is_ordering() && !orderable_kind {
            return Err(ErrorCode::Malformed);
        }
    }
    for spec in aggregates {
        match (spec.func, spec.field) {
            (AggregateFn::Count, None) => {}
            (AggregateFn::Count, Some(_)) => return Err(ErrorCode::Malformed),
            (_, None) => return Err(ErrorCode::Malformed),
            (_, Some(tag)) => {
                let kind = kind_of(tag).ok_or(ErrorCode::UnknownField)?;
                if !matches!(kind, protocol::ValueKind::U32 | protocol::ValueKind::I64) {
                    return Err(ErrorCode::Malformed);
                }
            }
        }
    }
    Ok(())
}

/// `AGG-FR-007`/`AGG-FR-008` (ADR-0035): filter, bucket, and reduce
/// `rows` — the one place this logic is written, shared by every
/// domain's `Request::Aggregate` (`scan_all`'s result is unfiltered and
/// unbucketed). `filter` reuses `predicate_matches` unchanged; `group_by`
/// empty means exactly one implicit bucket over every filtered row (the
/// `SELECT COUNT(*) FROM t` case with no `GROUP BY`) — that bucket always
/// exists, even holding zero rows (`SELECT COUNT(*) FROM t WHERE false`-
/// equivalent still returns one group whose `Count` is `0`, acceptance
/// criterion 4); a *keyed* bucket (`group_by` non-empty) whose key
/// matches zero rows never appears, since there is no key to have not
/// matched. `limit` truncates the *group* count, applied after the full
/// reduction — the same "bounds the response, not the work" shape
/// `evaluate_query`'s own `limit` already has. `schema` resolves each
/// `Sum`/`Avg`/`Min`/`Max` field's `ValueKind` for the one case a real
/// observed value can't: the implicit whole-table bucket with zero rows.
/// Since `AGB-FR-001` (ADR-0085) each row finds its bucket through a
/// [`BucketKey`] hash rather than a linear search over the buckets so
/// far — the same buckets in the same first-seen order, linear in the
/// rows rather than quadratic in the groups.
fn evaluate_aggregate(
    rows: Vec<(RecordId, Vec<(FieldRef, ScanValue)>)>,
    group_by: &[FieldRef],
    filter: &[Predicate],
    aggregates: &[AggregateSpec],
    limit: Option<usize>,
    schema: &DomainSchema,
) -> Vec<AggregateGroup> {
    type RowFields = Vec<(FieldRef, ScanValue)>;
    let mut buckets: Vec<(RowFields, Vec<RowFields>)> = if group_by.is_empty() {
        vec![(Vec::new(), Vec::new())]
    } else {
        Vec::new()
    };
    let mut slots: HashMap<Vec<(FieldRef, BucketKey)>, usize> = HashMap::new();
    for (_, fields) in rows
        .into_iter()
        .filter(|(_, fields)| filter.iter().all(|p| predicate_matches(fields, p)))
    {
        if group_by.is_empty() {
            buckets[0].1.push(fields);
            continue;
        }
        let key: Vec<(FieldRef, ScanValue)> = group_by
            .iter()
            .filter_map(|&tag| fields.iter().find(|(f, _)| *f == tag).cloned())
            .collect();
        let hashed: Vec<(FieldRef, BucketKey)> =
            key.iter().map(|(f, v)| (*f, BucketKey::of(v))).collect();
        match slots.get(&hashed) {
            Some(&slot) => buckets[slot].1.push(fields),
            None => {
                slots.insert(hashed, buckets.len());
                buckets.push((key, vec![fields]));
            }
        }
    }
    let mut groups: Vec<AggregateGroup> = buckets
        .into_iter()
        .map(|(key, group_rows)| AggregateGroup {
            key,
            values: aggregates
                .iter()
                .map(|spec| reduce(spec, &group_rows, schema))
                .collect(),
        })
        .collect();
    if let Some(limit) = limit {
        groups.truncate(limit);
    }
    groups
}

/// `AGB-FR-001` (ADR-0085): a [`ScanValue`] as a hashable bucket key —
/// every variant carried as itself, `F64` by its bit pattern (no stored
/// field is `F64`, so no group key ever is; the bits keep the function
/// total without a panic or an `Eq` on `f64`). Two keys are equal
/// exactly when `ScanValue`'s own `==` would call them equal, for every
/// kind a group key can hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BucketKey {
    U32(u32),
    I64(i64),
    Bool(bool),
    Str(String),
    F64Bits(u64),
    StrList(Vec<String>),
}

impl BucketKey {
    fn of(value: &ScanValue) -> Self {
        match value {
            ScanValue::U32(v) => Self::U32(*v),
            ScanValue::I64(v) => Self::I64(*v),
            ScanValue::Bool(v) => Self::Bool(*v),
            ScanValue::Str(v) => Self::Str(v.clone()),
            ScanValue::F64(v) => Self::F64Bits(v.to_bits()),
            ScanValue::StrList(v) => Self::StrList(v.clone()),
        }
    }
}

/// One `AggregateSpec`'s result over one bucket's rows — `AGG-FR-008`.
/// `validate_aggregate` already guarantees `Sum`/`Avg`/`Min`/`Max` carry
/// a field of `U32`/`I64` kind, and `Count` carries none. `rows` is empty
/// only for the implicit whole-table bucket when no row matched `filter`
/// (`evaluate_aggregate`'s own doc comment) — `Sum` and `Avg` are already
/// well-defined there (an empty sum is `0`, an empty average is defined
/// as `0.0` by this schema's own choice, since it has no `NULL` to return
/// instead); `Min`/`Max` have no real observed value to pass through, so
/// they fall back to `schema`-typed zero rather than panicking.
fn reduce(
    spec: &AggregateSpec,
    rows: &[Vec<(FieldRef, ScanValue)>],
    schema: &DomainSchema,
) -> ScanValue {
    const FIELD_REQUIRED: &str = "validate_aggregate requires a field for Sum/Avg/Min/Max";
    match spec.func {
        AggregateFn::Count => ScanValue::I64(rows.len() as i64),
        AggregateFn::Sum => {
            let field = spec.field.expect(FIELD_REQUIRED);
            ScanValue::I64(field_values(field, rows).map(|v| numeric_i64(&v)).sum())
        }
        AggregateFn::Avg => {
            let field = spec.field.expect(FIELD_REQUIRED);
            let values: Vec<i64> = field_values(field, rows).map(|v| numeric_i64(&v)).collect();
            if values.is_empty() {
                ScanValue::F64(0.0)
            } else {
                let sum: i64 = values.iter().sum();
                ScanValue::F64(sum as f64 / values.len() as f64)
            }
        }
        AggregateFn::Min => reduce_extreme(spec.field.expect(FIELD_REQUIRED), rows, false, schema),
        AggregateFn::Max => reduce_extreme(spec.field.expect(FIELD_REQUIRED), rows, true, schema),
    }
}

fn field_values<'a>(
    field: FieldRef,
    rows: &'a [Vec<(FieldRef, ScanValue)>],
) -> impl Iterator<Item = ScanValue> + 'a {
    rows.iter()
        .filter_map(move |fields| fields.iter().find(|(f, _)| *f == field))
        .map(|(_, v)| v.clone())
}

fn numeric_i64(value: &ScanValue) -> i64 {
    match value {
        ScanValue::U32(n) => i64::from(*n),
        ScanValue::I64(n) => *n,
        other => unreachable!(
            "validate_aggregate only allows U32/I64 fields for Sum/Avg/Min/Max, got {other:?}"
        ),
    }
}

/// The actual observed `ScanValue` with the largest (`want_max`) or
/// smallest numeric value in `rows` for `field` — a passthrough of a
/// real stored value, never promoted or converted, unlike `Sum`/`Avg`.
/// `rows` is empty only for the implicit whole-table bucket with no
/// matching row (`reduce`'s own doc comment): there is no real value to
/// pass through, so this falls back to a `schema`-typed zero
/// (`ScanValue::U32(0)`/`ScanValue::I64(0)` matching `field`'s declared
/// kind) rather than panicking on an empty iterator.
fn reduce_extreme(
    field: FieldRef,
    rows: &[Vec<(FieldRef, ScanValue)>],
    want_max: bool,
    schema: &DomainSchema,
) -> ScanValue {
    let mut values = field_values(field, rows);
    let mut best = match values.next() {
        Some(first) => first,
        None => {
            let kind = schema
                .fields
                .iter()
                .find(|f| f.tag == field)
                .map(|f| f.value_kind)
                .expect("validate_aggregate already confirmed this field exists in schema");
            return match kind {
                protocol::ValueKind::U32 => ScanValue::U32(0),
                _ => ScanValue::I64(0),
            };
        }
    };
    for v in values {
        let replace = if want_max {
            numeric_i64(&v) > numeric_i64(&best)
        } else {
            numeric_i64(&v) < numeric_i64(&best)
        };
        if replace {
            best = v;
        }
    }
    best
}

/// Compatibility rule 3's "nearest older shape" (`protocol.rs`): a
/// response carrying a variant introduced after the connection's
/// negotiated version is rewritten before it is sent. Three cases exist.
/// Two remap an error code — `ErrorCode::Journal` (version 4, ADR-0025)
/// and `ErrorCode::Conflict` (version 7, ADR-0033), both inside
/// `TransactionFailed`, each seen as `Unsupported` by a connection below
/// the version that introduced it. The third rewrites *content* —
/// `ENT4-FR-003` (version 11, ADR-0041): `ScanValue::StrList` is the
/// first appended value variant that reaches an ungated response
/// (`GetById`/`Query`/`DescribeSchema` are protocol-1/8/1 requests a
/// silent client can send), so below 11 every `StrList` pair is stripped
/// from `Record` and from each `Rows` row, and every `StrList`
/// descriptor from `Schema` — the field did not exist for that client at
/// `FR-042` and does not now. `ScanValues`/`Groups` never carry one (the
/// adapter refuses `ScanField` on such a field; `validate_aggregate`
/// refuses it in `group_by`/aggregates), pinned by a debug assertion.
/// The session shapes (version 3) and `Request::Query`/`Response::Rows`
/// themselves (version 8) never need this: neither can arise on a
/// connection that could not send the gated request that produces it.
fn downgrade_for_version(resp: Response, negotiated: u32) -> Response {
    fn is_str_list(pair: &(FieldRef, ScanValue)) -> bool {
        matches!(pair.1, ScanValue::StrList(_))
    }
    match resp {
        Response::Record { id, fields } if negotiated < 11 => Response::Record {
            id,
            fields: fields.into_iter().filter(|p| !is_str_list(p)).collect(),
        },
        Response::Rows { rows } if negotiated < 11 => Response::Rows {
            rows: rows
                .into_iter()
                .map(|(id, fields)| (id, fields.into_iter().filter(|p| !is_str_list(p)).collect()))
                .collect(),
        },
        Response::Schema(mut schema) if negotiated < 11 => {
            schema
                .fields
                .retain(|f| f.value_kind != protocol::ValueKind::StrList);
            Response::Schema(schema)
        }
        Response::ScanValues { ref values } => {
            debug_assert!(
                !values.iter().any(|v| matches!(v, ScanValue::StrList(_))),
                "a StrList field is never scannable (ENT4-FR-003)"
            );
            resp
        }
        Response::Groups { ref groups } => {
            debug_assert!(
                !groups.iter().any(|g| g.key.iter().any(is_str_list)
                    || g.values.iter().any(|v| matches!(v, ScanValue::StrList(_)))),
                "a StrList field is never groupable or aggregatable (ENT4-FR-003)"
            );
            resp
        }
        Response::TransactionFailed {
            index,
            code: ErrorCode::Journal,
            ..
        } if negotiated < 4 => Response::TransactionFailed {
            index,
            code: ErrorCode::Unsupported,
            message: error_message(ErrorCode::Unsupported).to_string(),
        },
        Response::TransactionFailed {
            index,
            code: ErrorCode::Conflict,
            ..
        } if negotiated < 7 => Response::TransactionFailed {
            index,
            code: ErrorCode::Unsupported,
            message: error_message(ErrorCode::Unsupported).to_string(),
        },
        other => other,
    }
}

fn err_response(code: ErrorCode) -> Response {
    Response::Err {
        code,
        message: error_message(code).to_string(),
    }
}

/// `BAK-FR-004`–`006` (ADR-0065): resolve, confine, and execute one
/// `Request::Backup` — kept out of `dispatch` (unlike `Compact`) because
/// it needs [`ServeOptions::backup_root`], which a store-generic
/// `dispatch(store, req)` has no way to reach — the same reason
/// `Use`/`Delete`/`Join` bypass `dispatch` for `tables`-level routing,
/// just for `options` instead of `tables`.
///
/// The target directory is never observed in a partial state: copying
/// happens into a fresh, process-unique temporary directory under
/// `root` (any leftover from a prior crash at the same name is removed
/// first), and only a successful [`ConnectionStore::backup`] is
/// followed by one atomic [`std::fs::rename`] onto the real name. An
/// *existing* target — empty or not — is refused outright rather than
/// risked against `std::fs::rename`'s own platform-specific "destination
/// already exists" behavior (POSIX and Windows disagree on replacing a
/// directory); a caller who wants to retry a name removes the old
/// backup first.
fn handle_backup(store: &dyn ConnectionStore, options: &ServeOptions, name: &str) -> Response {
    let Some(root) = options.backup_root() else {
        return err_response(ErrorCode::Unsupported);
    };
    // `BAK-FR-004`: a single path component, checked before any I/O —
    // the server never honors a caller-supplied absolute path or a `..`
    // component.
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return err_response(ErrorCode::Malformed);
    }
    let target = root.join(name);
    if target.exists() {
        return err_response(ErrorCode::Storage);
    }
    let tmp = root.join(format!(".backup-tmp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let result = match store.backup(&tmp) {
        Ok(report) => std::fs::rename(&tmp, &target)
            .map(|()| Response::BackedUp {
                files: report.files,
                bytes: report.bytes,
            })
            .map_err(|_| ErrorCode::Storage),
        Err(code) => Err(code),
    };
    result.unwrap_or_else(|code| {
        let _ = std::fs::remove_dir_all(&tmp);
        err_response(code)
    })
}

/// `ACC-FR-001`: a dispatched request's outcome *shape*, for the access
/// log — exhaustive over every `Response` variant, never its content.
/// `NotFound`/`NoParent` are `Ok` (a normal outcome, per this crate's own
/// convention); `Err`/`TransactionFailed` are `Err(code)`, the code alone.
fn outcome_of(resp: &Response) -> access::Outcome {
    match resp {
        Response::Err { code, .. } | Response::TransactionFailed { code, .. } => {
            access::Outcome::Err(*code)
        }
        Response::Record { .. }
        | Response::RecordList { .. }
        | Response::ScanValues { .. }
        | Response::Id { .. }
        | Response::Schema(_)
        | Response::NotFound
        | Response::NoParent
        | Response::Ok
        | Response::Hello { .. }
        | Response::Staged { .. }
        | Response::Rows { .. }
        | Response::Groups { .. }
        | Response::RelationKinds { .. }
        | Response::JoinedRows { .. }
        | Response::Relations { .. }
        | Response::Tables { .. }
        | Response::Compacted { .. }
        | Response::Count { .. }
        | Response::BatchResults { .. }
        | Response::Metrics { .. }
        | Response::BackedUp { .. }
        | Response::Snapshot { .. } => access::Outcome::Ok,
    }
}

/// The two static permission classes a configured token can grant — see
/// `docs/design/SERVER-AUTH-DESIGN.md`, ADR-0012. Deliberately coarse:
/// `ReadOnly` is blocked only from [`Request::UpdateField`],
/// [`Request::Transaction`] (`TXN-FR-004` extends `AUTH-FR-003`'s rule to
/// the latter), and — since protocol 13/14 — [`Request::Insert`]
/// (`INS-FR-007`) and [`Request::Link`] (`LNK-FR-010`); both classes can
/// do everything else, including
/// `DescribeSchema` (`AUTH-FR-003`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    ReadOnly,
    ReadWrite,
    /// `RPL-FR-002` (ADR-0067): the one class [`Request::FetchSnapshot`]
    /// ever accepts — never granted by the no-tokens-configured
    /// `ReadWrite` bootstrap default (`AUTH-FR-007`), and never matched
    /// by a `read_only_token`/`read_write_token` even if an operator
    /// configures both. A server with no `replication_token` configured
    /// answers every `FetchSnapshot` `Unauthorized`, the same "opt-in,
    /// zero new surface by default" posture `SERVER_BACKUP_ROOT`
    /// established for local-disk backup — this is that same posture
    /// for network-transferable snapshots.
    Replication,
}

/// Which tokens (if any) this server instance accepts, and what
/// [`TokenClass`] each grants (`AUTH-FR-005`). Built once at server
/// startup and shared (`Arc`) across every connection thread [`serve`]
/// spawns.
///
/// Tokens are never logged or echoed back on any path, including error
/// messages (`AUTH-FR-005`).
#[derive(Default)]
pub struct ServeOptions {
    read_only_token: Option<String>,
    read_write_token: Option<String>,
    /// `AUD-FR-003` (ADR-0029): where admission, authentication, and
    /// authorization decisions are recorded; `None` is [`super::audit::NoAudit`].
    audit: Option<Arc<dyn audit::AuditSink>>,
    /// `CLS-FR-003` (ADR-0028): a presented leaf certificate's exact DER
    /// bytes mapped to the class it grants. Matched by `==` on byte
    /// slices — no parsing, no constant-time requirement (certificates
    /// are public material).
    certificate_classes: Vec<(Vec<u8>, TokenClass)>,
    /// `RL-FR-002`/`RL-FR-003` (ADR-0030): the opt-in per-peer failed-
    /// `Authenticate` budget; `None` is no budget. Shared across every
    /// connection thread through the `Arc`, same lifecycle as `audit`.
    rate_limit: Option<Arc<FailureTable>>,
    /// `ACC-FR-003` (ADR-0031): where per-request access events are
    /// recorded, independent of `audit`; `None` is [`super::access::NoAccessLog`].
    access_log: Option<Arc<dyn access::AccessSink>>,
    /// `SRV-FR-001` (ADR-0032): native TLS, folded in from the former
    /// second `serve` parameter — `None` is plaintext, exactly [`serve`]'s
    /// behavior before this field existed (`TLS-FR-008`).
    tls: Option<TlsConfig>,
    /// `MET-FR-002` (ADR-0064): always-on process-wide counters — not an
    /// `Option`, unlike every sink above, since rendering them costs
    /// nothing and needs no operator opt-in.
    metrics: ServerMetrics,
    /// `MHTTP-FR-001` (ADR-0069): an already-bound, opt-in HTTP scrape
    /// listener. Taken by `serve_tables` before sharing the options;
    /// `None` opens no second listener and preserves the wire-only default.
    metrics_http: Option<TcpListener>,
    /// `BAK-FR-003` (ADR-0065): the confinement boundary every
    /// `Request::Backup` target must resolve under. `None` (the
    /// default) answers every `Backup` request `Unsupported` —
    /// zero new filesystem-write surface unless an operator opts in.
    backup_root: Option<PathBuf>,
    /// `RPL-FR-002` (ADR-0067): the one credential
    /// [`TokenClass::Replication`] is ever granted for — distinct from
    /// `read_only_token`/`read_write_token`, so a `ReadWrite` client
    /// never automatically holds it. `None` (the default) answers every
    /// `Request::FetchSnapshot` `Unauthorized` server-wide.
    replication_token: Option<String>,
}

impl std::fmt::Debug for ServeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let read_only_certs = self
            .certificate_classes
            .iter()
            .filter(|(_, class)| *class == TokenClass::ReadOnly)
            .count();
        let read_write_certs = self.certificate_classes.len() - read_only_certs;
        f.debug_struct("ServeOptions")
            .field("read_only_token", &self.read_only_token)
            .field("read_write_token", &self.read_write_token)
            // `CLS-FR-006`: counts only, never a configured leaf's bytes.
            .field("read_only_certificates", &read_only_certs)
            .field("read_write_certificates", &read_write_certs)
            .field(
                "audit",
                &if self.audit.is_some() {
                    "configured"
                } else {
                    "none"
                },
            )
            // `RL-FR-002`: the budget's numbers, never the tracked peers.
            .field("rate_limit", &self.rate_limit())
            .field(
                "access_log",
                &if self.access_log.is_some() {
                    "configured"
                } else {
                    "none"
                },
            )
            .field(
                "tls",
                &if self.tls.is_some() {
                    "configured"
                } else {
                    "none"
                },
            )
            .field(
                "backup_root",
                &if self.backup_root.is_some() {
                    "configured"
                } else {
                    "none"
                },
            )
            .field(
                "replication_token",
                &if self.replication_token.is_some() {
                    "configured"
                } else {
                    "none"
                },
            )
            .finish()
    }
}

static NO_AUDIT: audit::NoAudit = audit::NoAudit;
static NO_ACCESS_LOG: access::NoAccessLog = access::NoAccessLog;

impl ServeOptions {
    /// Build directly from already-known tokens. Use this from tests and
    /// from any caller that already has its tokens from its own config
    /// source — see [`ServeOptions::from_env`]'s own doc comment for why
    /// tests specifically must not use real environment variables.
    pub fn new(read_only_token: Option<String>, read_write_token: Option<String>) -> Self {
        Self {
            read_only_token,
            read_write_token,
            audit: None,
            certificate_classes: Vec::new(),
            rate_limit: None,
            access_log: None,
            tls: None,
            metrics: ServerMetrics::new(),
            metrics_http: None,
            backup_root: None,
            replication_token: None,
        }
    }

    /// `CLS-FR-003` (ADR-0028): a client presenting a certificate whose
    /// leaf's DER bytes exactly equal `leaf_der` starts the connection at
    /// `class`, with no `Authenticate` needed (`CLS-FR-004`) — see
    /// `handle_connection`'s TLS arm. Repeatable; builds the map one
    /// certificate at a time. Only takes effect when [`with_tls`][Self::with_tls]
    /// configures client auth (`SERVER_TLS_CLIENT_CA_PATH`) — this crate
    /// does not refuse the combination on its own; see `dog_server`'s
    /// startup check (`CLS-FR-005`).
    pub fn with_certificate_class(mut self, leaf_der: Vec<u8>, class: TokenClass) -> Self {
        self.certificate_classes.push((leaf_der, class));
        self
    }

    /// [`ServeOptions::with_certificate_class`] for every `CERTIFICATE`
    /// block in a PEM file — a leaf per class-holding certificate,
    /// classed identically (`CLS-FR-003`).
    pub fn with_certificate_class_pem_file(
        mut self,
        path: impl AsRef<Path>,
        class: TokenClass,
    ) -> Result<Self, TlsConfigError> {
        let pem_text = std::fs::read_to_string(path).map_err(TlsConfigError::Io)?;
        let leaves = pem::decode_blocks(&pem_text).map_err(TlsConfigError::Pem)?;
        for leaf_der in leaves {
            self.certificate_classes.push((leaf_der, class));
        }
        Ok(self)
    }

    /// `CLS-FR-003`: the class a presented leaf's DER bytes match, by
    /// exact byte equality against every configured certificate — `None`
    /// if `leaf_der` matches none of them.
    pub(crate) fn class_for_certificate(&self, leaf_der: &[u8]) -> Option<TokenClass> {
        self.certificate_classes
            .iter()
            .find(|(configured, _)| configured.as_slice() == leaf_der)
            .map(|(_, class)| *class)
    }

    /// `AUD-FR-003` (ADR-0029): record every admission, authentication,
    /// and authorization decision on `sink` — see [`audit`]. Off unless
    /// called; the sink is shared by every connection thread and is
    /// called after each decision, before the response, outside every
    /// lock. A sink that fails never fails a connection (`AUD-FR-006`).
    pub fn with_audit(mut self, sink: Arc<dyn audit::AuditSink>) -> Self {
        self.audit = Some(sink);
        self
    }

    /// The configured sink, or [`super::audit::NoAudit`].
    pub fn audit(&self) -> &dyn audit::AuditSink {
        match &self.audit {
            Some(sink) => sink.as_ref(),
            None => &NO_AUDIT,
        }
    }

    /// Build from `SERVER_AUTH_READ_ONLY_TOKEN`/`SERVER_AUTH_READ_WRITE_TOKEN`
    /// at process startup (`AUTH-FR-005`). Reserved for a real server
    /// binary's own one-time startup — `cargo test` runs many tests in
    /// parallel within one process, so reading real process-wide
    /// environment variables from a test would race with every other test
    /// doing the same; tests use [`ServeOptions::new`] instead.
    pub fn from_env() -> Self {
        Self {
            read_only_token: std::env::var("SERVER_AUTH_READ_ONLY_TOKEN").ok(),
            read_write_token: std::env::var("SERVER_AUTH_READ_WRITE_TOKEN").ok(),
            audit: None,
            certificate_classes: Vec::new(),
            rate_limit: None,
            access_log: None,
            tls: None,
            metrics: ServerMetrics::new(),
            metrics_http: None,
            backup_root: None,
            replication_token: std::env::var("SERVER_AUTH_REPLICATION_TOKEN").ok(),
        }
    }

    /// `SRV-FR-001`/`SRV-FR-003` (ADR-0032): native TLS — the former
    /// second `serve` parameter, now one more opt-in field. Kept a
    /// separate, still-fallible construction step deliberately
    /// (`TlsConfig::new`/`from_env` can fail; nothing about `ServeOptions`
    /// itself can) — a caller builds a `TlsConfig` and handles its
    /// `Result` first, then folds it in here, exactly the two-step
    /// pattern every other conditionally-configured piece already uses
    /// (`with_audit`, `with_rate_limit`, `with_access_log`).
    pub fn with_tls(mut self, tls: TlsConfig) -> Self {
        self.tls = Some(tls);
        self
    }

    /// The configured `TlsConfig`, or `None` for plaintext.
    pub fn tls(&self) -> Option<&TlsConfig> {
        self.tls.as_ref()
    }

    /// `MET-FR-002` (ADR-0064): the process-wide counters this
    /// `ServeOptions`' `Arc` shares across every connection thread —
    /// always present, never an `Option`.
    pub fn metrics(&self) -> &ServerMetrics {
        &self.metrics
    }

    /// `MHTTP-FR-001` (ADR-0069): serve `GET /metrics` on this separate,
    /// already-bound listener when `serve`/`serve_tables` starts. The
    /// caller handles bind failures, just as for the wire listener.
    /// Opt-in: unset, no second listener is opened. HTTP reads the same
    /// counters without incrementing them and has no TLS or authentication
    /// gate (`MHTTP-FR-002`/`005`, accepted option (a)).
    pub fn with_metrics_http(mut self, listener: TcpListener) -> Self {
        self.metrics_http = Some(listener);
        self
    }

    /// `BAK-FR-003` (ADR-0065): every `Request::Backup` target must
    /// resolve to a descendant of `root` — opt-in; unset, every `Backup`
    /// request answers `Unsupported` server-wide, the identical
    /// "closed unless configured" posture `SERVER_TXN_JOURNAL_PATH`
    /// already established for the transaction journal.
    pub fn with_backup_root(mut self, root: PathBuf) -> Self {
        self.backup_root = Some(root);
        self
    }

    /// The configured backup root, or `None`.
    pub(crate) fn backup_root(&self) -> Option<&Path> {
        self.backup_root.as_deref()
    }

    /// `RPL-FR-002` (ADR-0067): the one credential
    /// [`TokenClass::Replication`] is ever granted for — distinct from
    /// `read_only_token`/`read_write_token`. Unset, every
    /// `Request::FetchSnapshot` answers `Unauthorized` server-wide, no
    /// new surface at all — the identical "opt-in, zero new surface by
    /// default" posture [`ServeOptions::with_backup_root`] already
    /// established for local-disk backup.
    pub fn with_replication_token(mut self, token: String) -> Self {
        self.replication_token = Some(token);
        self
    }

    /// No tokens *and* no certificate classes configured — `AUTH-FR-007`:
    /// every connection behaves exactly as it did before this feature
    /// existed, and `Authenticate` becomes a no-op success. `replication_token`
    /// counts as a configured token too (`RPL-FR-002`): a server with
    /// only it set still requires every connection to authenticate
    /// before anything, the same posture a `read_only_token`-only
    /// server already has. Since `CLS-FR-003` (ADR-0028) a
    /// certificates-only deployment (classes, no tokens) is also
    /// "configured": an admitted certificate not in the map starts
    /// unauthenticated rather than falling back to `ReadWrite` — the
    /// safe direction, see `SERVER-MTLS-CLASS-DESIGN.md`.
    pub fn is_configured(&self) -> bool {
        self.read_only_token.is_some()
            || self.read_write_token.is_some()
            || self.replication_token.is_some()
            || !self.certificate_classes.is_empty()
    }

    /// Check `token` against every configured token in constant time
    /// (`AUTH-FR-006`, via the `subtle` crate rather than a hand-rolled
    /// comparison — see this crate's `Cargo.toml` for why). Both slots are
    /// always checked, never short-circuited on the first match, so
    /// neither which slot (if either) matched nor how many slots are
    /// configured is observable from timing.
    fn check(&self, token: &str) -> Option<TokenClass> {
        use subtle::ConstantTimeEq;
        let mut result: Option<TokenClass> = None;
        if let Some(read_write) = &self.read_write_token {
            if bool::from(read_write.as_bytes().ct_eq(token.as_bytes())) {
                result = Some(TokenClass::ReadWrite);
            }
        }
        if let Some(read_only) = &self.read_only_token {
            if bool::from(read_only.as_bytes().ct_eq(token.as_bytes())) {
                result = Some(TokenClass::ReadOnly);
            }
        }
        if let Some(replication) = &self.replication_token {
            if bool::from(replication.as_bytes().ct_eq(token.as_bytes())) {
                result = Some(TokenClass::Replication);
            }
        }
        result
    }

    /// `RL-FR-002` (ADR-0030): opt-in — count failed `Authenticate`s per
    /// peer IP over `limit.window`; once a peer is at or over
    /// `limit.failures` in its current window, every further
    /// `Authenticate` from that address is refused before any comparison,
    /// audited as `Throttled` — see `handle_connection`. Off unless
    /// called; bounded by `MAX_TRACKED_PEERS` regardless of how many
    /// addresses fail (`RL-FR-003`).
    pub fn with_rate_limit(mut self, limit: RateLimit) -> Self {
        self.rate_limit = Some(Arc::new(FailureTable::new(limit)));
        self
    }

    /// The configured budget, if any.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.rate_limit.as_ref().map(|table| table.limit)
    }

    /// `RL-FR-002`: whether `peer` is currently over its configured
    /// budget — `false` with no budget configured or no peer address (a
    /// `peer_addr` failure never throttles; it still locks out per
    /// connection).
    fn is_throttled(&self, peer: Option<IpAddr>) -> bool {
        match (&self.rate_limit, peer) {
            (Some(table), Some(peer)) => table.is_throttled(peer),
            _ => false,
        }
    }

    /// `RL-FR-002`: record one failed `Authenticate` from `peer` against
    /// the configured budget — a no-op with no budget configured or no
    /// peer address. Returns whether `peer` is now over budget.
    fn note_failure(&self, peer: Option<IpAddr>) -> bool {
        match (&self.rate_limit, peer) {
            (Some(table), Some(peer)) => table.note_failure(peer),
            _ => false,
        }
    }

    /// `ACC-FR-003` (ADR-0031): record one [`super::access::AccessEvent`] per
    /// dispatched request on `sink` — see [`access`]. Off unless called,
    /// independent of `with_audit`: an operator's choice to turn on one
    /// never implies the other's cost. Called after the response is
    /// decided, outside every lock, in `handle_connection`.
    pub fn with_access_log(mut self, sink: Arc<dyn access::AccessSink>) -> Self {
        self.access_log = Some(sink);
        self
    }

    /// The configured sink, or [`super::access::NoAccessLog`].
    pub fn access_log(&self) -> &dyn access::AccessSink {
        match &self.access_log {
            Some(sink) => sink.as_ref(),
            None => &NO_ACCESS_LOG,
        }
    }
}

/// `RL-FR-001` (ADR-0030): a connection's fifth failed `Authenticate` is
/// answered `Unauthenticated` as any wrong token is, then the server
/// records `LockedOut` and closes the connection. On by default and not
/// configurable — the `MAX_STAGED_OPS` precedent: a knob nobody has asked
/// for yet, not one designed in speculatively.
pub const MAX_AUTH_FAILURES: u32 = 5;

/// `RL-FR-003` (ADR-0030): the per-peer failure table never grows past
/// this many tracked addresses — bounded memory under an address flood.
/// On insert, expired entries are dropped first, then the oldest window
/// start if the table is still full: the budget degrades toward "no
/// budget" under a flood, never toward "no service."
pub const MAX_TRACKED_PEERS: usize = 4096;

/// `RL-FR-002` (ADR-0030): an opt-in per-peer failed-`Authenticate`
/// budget — at most `failures` failures per `window`, per peer IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub failures: u32,
    pub window: Duration,
}

/// `RateLimit::parse` failed — `SERVER_AUTH_RATE_LIMIT` was set but not
/// `"<failures>/<seconds>"`, or one half was zero.
#[derive(Debug)]
pub struct RateLimitParseError(String);

impl std::fmt::Display for RateLimitParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid rate limit {:?}: expected \"<failures>/<seconds>\", both nonzero",
            self.0
        )
    }
}

impl std::error::Error for RateLimitParseError {}

impl RateLimit {
    /// `RL-FR-006`: parse `"<failures>/<seconds>"` (e.g. `"10/60"`) —
    /// both halves nonzero integers, or an error naming the whole input.
    pub fn parse(s: &str) -> Result<Self, RateLimitParseError> {
        let invalid = || RateLimitParseError(s.to_string());
        let (failures, seconds) = s.split_once('/').ok_or_else(invalid)?;
        let failures: u32 = failures.parse().map_err(|_| invalid())?;
        let seconds: u64 = seconds.parse().map_err(|_| invalid())?;
        if failures == 0 || seconds == 0 {
            return Err(invalid());
        }
        Ok(Self {
            failures,
            window: Duration::from_secs(seconds),
        })
    }
}

/// One peer's current window: when it started (monotonic — never
/// wall-clock, so a clock step cannot shorten or lengthen it) and how
/// many failures have landed in it.
#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    failures: u32,
}

/// `RL-FR-002`/`RL-FR-003`: the shared per-peer failure table backing
/// [`ServeOptions::with_rate_limit`]. One mutex, touched only on the
/// `Authenticate` path of a server with a budget configured, for one
/// lookup or insert (`RL-FR-007`).
#[derive(Debug)]
struct FailureTable {
    limit: RateLimit,
    peers: Mutex<HashMap<IpAddr, Window>>,
}

impl FailureTable {
    fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// A poisoned mutex fails open — "not throttled" — trading a
    /// vanishingly unlikely availability gap (another thread panicked
    /// mid-update) for never wedging every connection's `Authenticate`
    /// path; the per-connection lockout still holds regardless.
    fn is_throttled(&self, peer: IpAddr) -> bool {
        let Ok(peers) = self.peers.lock() else {
            return false;
        };
        match peers.get(&peer) {
            Some(window) => {
                Instant::now().duration_since(window.started) < self.limit.window
                    && window.failures >= self.limit.failures
            }
            None => false,
        }
    }

    /// Record one failure for `peer`: a fresh window if none is tracked
    /// or the current one expired, otherwise one more failure in it. If
    /// tracking a new peer would exceed `MAX_TRACKED_PEERS`, expired
    /// entries are purged first, then the oldest window start is evicted
    /// if the table is still full (`RL-FR-003`). Returns whether `peer`
    /// is now over budget.
    fn note_failure(&self, peer: IpAddr) -> bool {
        let Ok(mut peers) = self.peers.lock() else {
            return false;
        };
        let now = Instant::now();
        if let Some(window) = peers.get_mut(&peer) {
            if now.duration_since(window.started) >= self.limit.window {
                *window = Window {
                    started: now,
                    failures: 1,
                };
            } else {
                window.failures += 1;
            }
        } else {
            if peers.len() >= MAX_TRACKED_PEERS {
                let window = self.limit.window;
                peers.retain(|_, w| now.duration_since(w.started) < window);
                if peers.len() >= MAX_TRACKED_PEERS {
                    if let Some(oldest) = peers
                        .iter()
                        .min_by_key(|(_, w)| w.started)
                        .map(|(addr, _)| *addr)
                    {
                        peers.remove(&oldest);
                    }
                }
            }
            peers.insert(
                peer,
                Window {
                    started: now,
                    failures: 1,
                },
            );
        }
        peers
            .get(&peer)
            .is_some_and(|w| w.failures >= self.limit.failures)
    }
}

/// Native TLS configuration for [`serve`] (ADR-0014,
/// `docs/design/SERVER-TLS-DESIGN.md`). Wraps a `rusty_tls::TlsAcceptor`
/// — this owner's own ecosystem-wide `rustls` wrapper, not a direct
/// `rustls` dependency, see that design's own "Ecosystem check" for why —
/// built once at server startup and shared across every connection
/// thread [`serve`] spawns, the same lifecycle [`ServeOptions`] already
/// uses for its own configured tokens. `serve` with `tls: None` behaves
/// exactly as it did before this feature existed (`TLS-FR-008`) — this is
/// purely opt-in.
pub struct TlsConfig {
    acceptor: rusty_tls::TlsAcceptor,
    /// `MTLS-FR-001` (ADR-0023): whether `acceptor` was built with client
    /// CA roots, so every connection must present a certificate chaining
    /// to one of them or fail the handshake.
    requires_client_certificate: bool,
}

impl TlsConfig {
    /// Build directly from DER-encoded certificate chain + private key —
    /// `cert_chain_der` is the leaf certificate followed by any
    /// intermediates, each DER-encoded, leaf first; `private_key_der` is
    /// the leaf's private key, DER-encoded (PKCS#8, PKCS#1, or SEC1,
    /// auto-detected — see `rusty_tls::TlsAcceptor::new`'s own doc
    /// comment). Use this directly when the caller already has DER bytes;
    /// see [`TlsConfig::from_pem_files`] for the common PEM-file case.
    pub fn new(
        cert_chain_der: Vec<Vec<u8>>,
        private_key_der: Vec<u8>,
    ) -> Result<Self, TlsConfigError> {
        let acceptor = rusty_tls::TlsAcceptor::new(cert_chain_der, private_key_der)
            .map_err(TlsConfigError::Tls)?;
        Ok(Self {
            acceptor,
            requires_client_certificate: false,
        })
    }

    /// [`TlsConfig::new`] plus the DER-encoded CA certificates a client
    /// certificate must chain to (`MTLS-FR-001`, ADR-0023,
    /// `docs/design/SERVER-MTLS-DESIGN.md`) — mutual TLS as an *admission*
    /// gate: a connection that presents no certificate, or one that does
    /// not chain to any of `client_ca_roots_der`, fails the TLS handshake
    /// and is dropped before any framed message (`Authenticate` included)
    /// is read, on the same `TLS-FR-003` path as any other handshake
    /// failure. Admission is all the certificate decides: an admitted
    /// connection still starts exactly where [`ServeOptions`] says it does
    /// (`MTLS-FR-002`), and nothing in this crate ever reads the admitted
    /// certificate's contents (`MTLS-FR-005`). An empty root set is
    /// `TlsConfigError::Tls` (`rusty_tls::Error::InvalidClientCaRoots`) —
    /// a server never starts with mTLS silently off. `handle_connection`
    /// has no mTLS branch: the acceptor carries the policy.
    pub fn new_with_client_auth(
        cert_chain_der: Vec<Vec<u8>>,
        private_key_der: Vec<u8>,
        client_ca_roots_der: Vec<Vec<u8>>,
    ) -> Result<Self, TlsConfigError> {
        let acceptor = rusty_tls::TlsAcceptor::new_with_client_auth(
            cert_chain_der,
            private_key_der,
            client_ca_roots_der,
        )
        .map_err(TlsConfigError::Tls)?;
        Ok(Self {
            acceptor,
            requires_client_certificate: true,
        })
    }

    /// Whether every connection must present a client certificate —
    /// `true` only for a config built by [`TlsConfig::new_with_client_auth`]
    /// or its PEM/environment equivalents.
    pub fn requires_client_certificate(&self) -> bool {
        self.requires_client_certificate
    }

    /// Build from PEM-encoded certificate chain + private key files
    /// (`TLS-FR-006`) — the common operator-facing format (`openssl`,
    /// `mkcert`, a CA). `rusty_tls::TlsAcceptor::new` itself takes DER
    /// bytes directly (`rusty_tls` deliberately keeps its own public seam
    /// narrow and doesn't re-expose a PEM parser — see
    /// `docs/design/SERVER-TLS-DESIGN.md`'s "Ecosystem check"), so this
    /// decodes PEM into DER first (see the `pem` module — a small,
    /// hand-written decoder, not a new dependency). `cert_chain_path` may
    /// contain more than one `-----BEGIN CERTIFICATE-----` block (the
    /// leaf followed by any intermediates, leaf first); `private_key_path`
    /// must contain exactly one block.
    pub fn from_pem_files(
        cert_chain_path: impl AsRef<Path>,
        private_key_path: impl AsRef<Path>,
    ) -> Result<Self, TlsConfigError> {
        let cert_chain_pem =
            std::fs::read_to_string(cert_chain_path).map_err(TlsConfigError::Io)?;
        let private_key_pem =
            std::fs::read_to_string(private_key_path).map_err(TlsConfigError::Io)?;
        let cert_chain_der = pem::decode_blocks(&cert_chain_pem).map_err(TlsConfigError::Pem)?;
        let private_key_blocks =
            pem::decode_blocks(&private_key_pem).map_err(TlsConfigError::Pem)?;
        let [private_key_der] = <[Vec<u8>; 1]>::try_from(private_key_blocks)
            .map_err(|_| TlsConfigError::Pem(pem::PemError::UnterminatedBlock))?;
        Self::new(cert_chain_der, private_key_der)
    }

    /// [`TlsConfig::from_pem_files`] plus a PEM file holding one or more
    /// `CERTIFICATE` blocks — the client CA roots for
    /// [`TlsConfig::new_with_client_auth`] (`MTLS-FR-004`).
    pub fn from_pem_files_with_client_ca(
        cert_chain_path: impl AsRef<Path>,
        private_key_path: impl AsRef<Path>,
        client_ca_path: impl AsRef<Path>,
    ) -> Result<Self, TlsConfigError> {
        let (cert_chain_der, private_key_der) =
            Self::read_pem_chain_and_key(cert_chain_path, private_key_path)?;
        let client_ca_pem = std::fs::read_to_string(client_ca_path).map_err(TlsConfigError::Io)?;
        let client_ca_roots_der =
            pem::decode_blocks(&client_ca_pem).map_err(TlsConfigError::Pem)?;
        Self::new_with_client_auth(cert_chain_der, private_key_der, client_ca_roots_der)
    }

    /// The shared PEM→DER step of both `from_pem_files*` constructors.
    fn read_pem_chain_and_key(
        cert_chain_path: impl AsRef<Path>,
        private_key_path: impl AsRef<Path>,
    ) -> Result<(Vec<Vec<u8>>, Vec<u8>), TlsConfigError> {
        let cert_chain_pem =
            std::fs::read_to_string(cert_chain_path).map_err(TlsConfigError::Io)?;
        let private_key_pem =
            std::fs::read_to_string(private_key_path).map_err(TlsConfigError::Io)?;
        let cert_chain_der = pem::decode_blocks(&cert_chain_pem).map_err(TlsConfigError::Pem)?;
        let private_key_blocks =
            pem::decode_blocks(&private_key_pem).map_err(TlsConfigError::Pem)?;
        let [private_key_der] = <[Vec<u8>; 1]>::try_from(private_key_blocks)
            .map_err(|_| TlsConfigError::Pem(pem::PemError::UnterminatedBlock))?;
        Ok((cert_chain_der, private_key_der))
    }

    /// Build from `SERVER_TLS_CERT_CHAIN_PATH`/`SERVER_TLS_PRIVATE_KEY_PATH`
    /// at process startup, mirroring [`ServeOptions::from_env`]'s own
    /// pattern — `None` (rather than an error) if either variable is
    /// unset, so a caller can treat "TLS not configured" and "TLS
    /// misconfigured" differently: the former is `serve(..., None)`'s
    /// ordinary opt-out, the latter is a real startup error a caller
    /// should surface (`Some(Err(..))`). Since v0.13.0 (`MTLS-FR-004`)
    /// an optional third variable, `SERVER_TLS_CLIENT_CA_PATH`, selects
    /// [`TlsConfig::from_pem_files_with_client_ca`]; set while the chain/key
    /// pair is not, it is `Some(Err(TlsConfigError::Io(NotFound, ..)))`
    /// naming the missing variables — never a silent plaintext or
    /// no-mTLS server. (The chain/key pair itself is all-or-nothing as it
    /// always was: one of the two set alone is still `None`.)
    pub fn from_env() -> Option<Result<Self, TlsConfigError>> {
        Self::from_env_values(
            std::env::var("SERVER_TLS_CERT_CHAIN_PATH").ok(),
            std::env::var("SERVER_TLS_PRIVATE_KEY_PATH").ok(),
            std::env::var("SERVER_TLS_CLIENT_CA_PATH").ok(),
        )
    }

    /// [`TlsConfig::from_env`]'s decision table, factored so a test can
    /// drive it without touching the real process environment (the same
    /// constraint [`ServeOptions::from_env`]'s docs impose).
    fn from_env_values(
        cert_chain_path: Option<String>,
        private_key_path: Option<String>,
        client_ca_path: Option<String>,
    ) -> Option<Result<Self, TlsConfigError>> {
        match (cert_chain_path, private_key_path, client_ca_path) {
            (Some(chain), Some(key), None) => Some(Self::from_pem_files(chain, key)),
            (Some(chain), Some(key), Some(ca)) => {
                Some(Self::from_pem_files_with_client_ca(chain, key, ca))
            }
            (_, _, Some(_)) => Some(Err(TlsConfigError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "SERVER_TLS_CLIENT_CA_PATH is set but SERVER_TLS_CERT_CHAIN_PATH and \
                 SERVER_TLS_PRIVATE_KEY_PATH are not both set",
            )))),
            _ => None,
        }
    }
}

/// Which raw stream a connection is speaking — a plain, unencrypted
/// `TcpStream`, or one wrapped in TLS (ADR-0014,
/// `docs/design/SERVER-TLS-DESIGN.md`). `dispatch`/`ConnectionStore`
/// never see this distinction; it's resolved once per connection in
/// [`handle_connection`]. `framing::read_message`/`write_message` work
/// unchanged either way, since both are already generic over
/// `Read`/`Write`.
///
/// Read and write are split into two owned halves the same way the plain
/// path already did (`TcpStream::try_clone`, two independent socket
/// handles) — but a `rusty_tls::TlsServerStream` can't be split that way:
/// its `rustls::ServerConnection` state is shared, single-owner data that
/// both a read and a write need to reach through the same object.
/// `Rc<RefCell<_>>` gives the TLS path the same two-owned-halves shape.
/// Single-threaded is enough — each connection is served by exactly one
/// OS thread (see [`serve`]), so a `RefCell`'s runtime borrow check is
/// sufficient; no `Mutex`/`Arc` needed for this, unlike `ServeOptions`/
/// `TlsConfig` themselves, which really are shared *across* connection
/// threads.
enum ReadHalf {
    Plain(TcpStream),
    Tls(Rc<RefCell<rusty_tls::TlsServerStream<TcpStream>>>),
}

enum WriteHalf {
    Plain(TcpStream),
    Tls(Rc<RefCell<rusty_tls::TlsServerStream<TcpStream>>>),
}

impl Read for ReadHalf {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ReadHalf::Plain(s) => s.read(buf),
            ReadHalf::Tls(s) => s.borrow_mut().read(buf),
        }
    }
}

impl Write for WriteHalf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            WriteHalf::Plain(s) => s.write(buf),
            WriteHalf::Tls(s) => s.borrow_mut().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            WriteHalf::Plain(s) => s.flush(),
            WriteHalf::Tls(s) => s.borrow_mut().flush(),
        }
    }
}

/// Translate one [`Request`] into a [`Response`] against `store` — the
/// entire request-handling logic, independent of framing or sockets, kept
/// separate so it can be tested (see this module's tests) without a real
/// TCP connection.
/// `QPM-FR-001` (ADR-0086): the path `dispatch` will take for a planned
/// read — `None` for every request that is not one (a write, a point
/// read, `Page`, `Metrics`, …). A pure function of the request and the
/// adapter's declarations (`describe`, `range_field`), computed from the
/// same predicates the arms themselves consult — [`plan_query`] for the
/// candidate step, [`bounded_walk_applies`] for the page walk,
/// [`counted_walk_applies`]/[`keyed_walk_applies`] for the walk-only
/// aggregates (a grouped walk with no bound and a `limit` decodes,
/// `QKG-FR-003`, so it classifies as its candidate step) — so the
/// classification never diverges from the path without a code change
/// to both. Two named approximations: a request `dispatch` then refuses
/// at validation is classified anyway (the caller records only an ok
/// response, so it is never counted), and a walk-only aggregate whose
/// adapter refuses the walk (`range_count`/`range_keys` `Unsupported` —
/// no shipped adapter) is counted as the walk it asked for.
pub fn plan_of<S: ConnectionStore + ?Sized>(store: &S, req: &Request) -> Option<PlanKind> {
    let candidate_step = |schema: &DomainSchema, filter: &[Predicate]| match plan_query(
        schema,
        store.range_field(),
        filter,
    ) {
        QueryPlan::FullScan => PlanKind::FullScan,
        QueryPlan::IndexEq(_) => PlanKind::IndexEq,
        QueryPlan::IndexRange { .. } => PlanKind::IndexRange,
        QueryPlan::IndexIntersect { .. } => PlanKind::IndexIntersect,
    };
    match req {
        Request::Query { filter, .. } => Some(candidate_step(&store.describe(), filter)),
        Request::Aggregate {
            group_by,
            filter,
            aggregates,
            limit,
        } => {
            let range_field = store.range_field();
            let walks = keyed_walk_applies(range_field, group_by, filter, aggregates)
                && !(!group_by.is_empty() && filter.is_empty() && limit.is_some());
            if !walks {
                return Some(candidate_step(&store.describe(), filter));
            }
            let count_only = group_by.is_empty()
                && counted_walk_applies(range_field, group_by, filter, aggregates);
            Some(if count_only {
                PlanKind::CountedWalk
            } else {
                PlanKind::KeyedWalk
            })
        }
        Request::FilteredPage {
            order_by, filter, ..
        } => {
            let schema = store.describe();
            Some(
                if bounded_walk_applies(*order_by, store.range_field(), &schema, filter) {
                    PlanKind::BoundedWalk
                } else {
                    candidate_step(&schema, filter)
                },
            )
        }
        Request::Join(spec) => Some(candidate_step(&store.describe(), &spec.left_filter)),
        _ => None,
    }
}

pub fn dispatch<S: ConnectionStore + ?Sized>(store: &S, req: Request) -> Response {
    match req {
        Request::GetById { id } => match store.get(id) {
            Some(fields) => Response::Record { id, fields },
            None => Response::NotFound,
        },
        Request::FilterEq { field, value } => match store.filter_eq(field, &value) {
            Ok(records) => Response::RecordList { records },
            Err(code) => err_response(code),
        },
        Request::ScanField { field } => match store.scan_field(field) {
            Ok(values) => Response::ScanValues { values },
            Err(code) => err_response(code),
        },
        // `SQL-FR-004`/`SQL-FR-007` (ADR-0034): validated against the
        // schema alone before `scan_all` ever runs; a read, gated like
        // `GetById`/`FilterEq`/`ScanField` above, never overlaid or
        // read-set-tracked by a session (`SQL-FR-009`) — the intercepts
        // for those live in `handle_connection`, keyed on `GetById`
        // alone, so `Query` reaching `dispatch` at all already means
        // neither applies.
        Request::Query {
            select,
            filter,
            limit,
        } => {
            let schema = store.describe();
            match validate_query(&schema, &select, &filter) {
                // `QPL-FR-001`–`004` (ADR-0073): one declared equality
                // index's bucket when the filter has an `Eq` on an
                // indexed field, the whole table otherwise — and
                // `evaluate_query` re-checks every predicate either way,
                // so the result set is the full scan's by construction.
                Ok(()) => Response::Rows {
                    rows: evaluate_query(
                        indexed_candidates(store, &schema, &filter),
                        &select,
                        &filter,
                        limit,
                    ),
                },
                Err(code) => err_response(code),
            }
        }
        // `AGG-FR-006`/`AGG-FR-009` (ADR-0035): the same validate-then-scan
        // shape `Query` above uses, and the same "never overlaid, never
        // read-set-tracked" posture — `Aggregate` never reaches the
        // `GetById`-keyed session intercepts in `handle_connection` either.
        // `QPC-FR-002` (ADR-0074): the same candidate step too —
        // `evaluate_aggregate` re-filters every row before bucketing, so
        // the groups are the full scan's by construction.
        Request::Aggregate {
            group_by,
            filter,
            aggregates,
            limit,
        } => {
            let schema = store.describe();
            match validate_aggregate(&schema, &group_by, &filter, &aggregates) {
                // `QCW-FR-004` (ADR-0081) / `QKW-FR-004` (ADR-0082) /
                // `QKG-FR-002` (ADR-0084): a count of a range, an
                // aggregate over the walked key itself, or a `GROUP BY`
                // that key, the sorted index answers on its own — no
                // record read at all.
                Ok(()) => {
                    match keyed_walk(store, &schema, &group_by, &filter, &aggregates, limit) {
                        Some(groups) => Response::Groups { groups },
                        None => Response::Groups {
                            groups: evaluate_aggregate(
                                indexed_candidates(store, &schema, &filter),
                                &group_by,
                                &filter,
                                &aggregates,
                                limit,
                                &schema,
                            ),
                        },
                    }
                }
                Err(code) => err_response(code),
            }
        }
        Request::UpdateField { id, field, value } => match store.update_field(id, field, value) {
            Ok(true) => Response::Ok,
            Ok(false) => Response::NotFound,
            Err(code) => err_response(code),
        },
        Request::Parent { id } => match store.parent(id) {
            Ok(ParentLookup::Parent(parent_id)) => Response::Id { id: parent_id },
            Ok(ParentLookup::NoParent) => Response::NoParent,
            Ok(ParentLookup::ChildNotFound) => Response::NotFound,
            Err(code) => err_response(code),
        },
        Request::Children { id } => match store.children(id) {
            Ok(records) => Response::RecordList { records },
            Err(code) => err_response(code),
        },
        Request::Neighbors { id } => match store.neighbors(id) {
            Ok(records) => Response::RecordList { records },
            Err(code) => err_response(code),
        },
        // `ENT2-FR-004`/`005` (ADR-0039): gated entirely client-side,
        // the same posture `Query`/`Aggregate` above already have — no
        // negotiated-version check here.
        Request::NeighborsByRelation { id, relation } => {
            match store.neighbors_by_relation(id, &relation) {
                Ok(records) => Response::RecordList { records },
                Err(code) => err_response(code),
            }
        }
        Request::ListRelationKinds => Response::RelationKinds {
            kinds: store.list_relation_kinds(),
        },
        // `CNT-FR-003` (ADR-0057): one read, `NeighborsByRelation`'s
        // unknown-label rule. Gated in `handle_connection` like `Page`.
        Request::CountEdges { relation } => match store.count_edges(&relation) {
            Ok(count) => Response::Count { count },
            Err(code) => err_response(code),
        },
        // `WBT-FR-002`/`003` (ADR-0060): the batch is applied by the
        // adapter (pipelined or atomic per `atomic`); an atomic
        // precondition abort is `TransactionFailed`, naming the first
        // failing op. Gated in `handle_connection` (write, session, 22,
        // `MAX_BATCH_OPS`).
        Request::WriteBatch { ops, atomic } => match store.write_batch(&ops, atomic) {
            Ok(results) => Response::BatchResults { results },
            Err((index, code)) => Response::TransactionFailed {
                index,
                code,
                message: error_message(code).to_string(),
            },
        },
        // `JOIN-FR-001`–`003` (ADR-0044): validate against the schema and
        // the adapter's own relation list, then the index nested loop.
        // Gated server-side in `handle_connection` (`Malformed` below 12)
        // — unlike `Query`/`Aggregate`, per the accepted design text.
        Request::Join(spec) => {
            match validate_join(&store.describe(), &store.describe_relations(), None, &spec) {
                Ok(()) => Response::JoinedRows {
                    rows: evaluate_join(store, store, &spec),
                },
                Err(code) => err_response(code),
            }
        }
        // `TBL-FR-002`/`003` (ADR-0050): `dispatch` knows exactly one
        // table, so `Use` succeeds only for that table's own name and
        // `ListTables` lists it alone; `handle_connection` answers both
        // itself for a `serve_tables` server before ever reaching here.
        Request::Use { table } if table == store.table_name() => Response::Ok,
        Request::Use { .. } => err_response(ErrorCode::Malformed),
        Request::ListTables => Response::Tables {
            names: vec![store.table_name().to_string()],
            primary: store.table_name().to_string(),
        },
        Request::DescribeRelations => Response::Relations {
            relations: store.describe_relations(),
        },
        // `INS-FR-006`/`007` (ADR-0046): the adapter validates the whole
        // field list before writing; a duplicate id is a normal outcome
        // carried as the one new error code. Gated server-side in
        // `handle_connection` (`Malformed` below 13, `Unauthorized` for
        // `ReadOnly`, `SessionOpen` inside a session).
        Request::Insert { id, fields } => match store.insert_record(id, fields) {
            Ok(InsertOutcome::Inserted) => Response::Ok,
            Ok(InsertOutcome::Duplicate) => err_response(ErrorCode::Duplicate),
            Err(code) => err_response(code),
        },
        // `LNK-FR-009`/`010` (ADR-0047): both outcomes are `Ok` —
        // insert-or-ignore. Gated in `handle_connection` like `Insert`.
        Request::Link {
            left,
            right,
            relation,
        } => match store.link_records(left, right, &relation) {
            Ok(LinkOutcome::Linked | LinkOutcome::AlreadyLinked) => Response::Ok,
            Err(code) => err_response(code),
        },
        // `REP-FR-005`/`006` (ADR-0049): `Insert`'s validation, `UpdateField`'s
        // not-found shape. Gated in `handle_connection` like `Insert`.
        Request::Replace { id, fields } => match store.replace_record(id, fields) {
            Ok(ReplaceOutcome::Replaced) => Response::Ok,
            Ok(ReplaceOutcome::NotFound) => Response::NotFound,
            Err(code) => err_response(code),
        },
        // `DEL-FR-006` (ADR-0051): `UpdateField`'s not-found shape. Gated in
        // `handle_connection` like `Insert`; the cross-table cascade
        // (`DEL-FR-007`) is `delete_across`'s, since it needs every table.
        Request::Delete { id } => match store.delete_record(id) {
            Ok(DeleteOutcome::Deleted) => Response::Ok,
            Ok(DeleteOutcome::NotFound) => Response::NotFound,
            Err(code) => err_response(code),
        },
        // `CMP-FR-006` (ADR-0052): the counts, straight from the stack.
        // Gated in `handle_connection` like `Insert`.
        // `GRD-FR-004`/`005` (ADR-0054): the guard validated as a query
        // predicate first (nothing evaluated on refusal), then one guarded
        // write under the adapter's own lock. Gated like `Replace`.
        Request::ReplaceIf { id, fields, guard } => {
            if let Err(code) = validate_predicate(&store.describe(), &guard) {
                return err_response(code);
            }
            match store.replace_record_if(id, fields, &guard) {
                Ok(ReplaceIfOutcome::Replaced) => Response::Ok,
                Ok(ReplaceIfOutcome::NotFound) => Response::NotFound,
                Ok(ReplaceIfOutcome::GuardFailed) => err_response(ErrorCode::GuardFailed),
                Err(code) => err_response(code),
            }
        }
        // `PAG-FR-003`/`004` (ADR-0055): validate, then one ordered page —
        // `Query`'s own validate-then-scan shape and posture.
        Request::Page {
            order_by,
            after,
            limit,
        } => match validate_page(&store.describe(), order_by, after.as_ref(), limit) {
            Ok(()) => match store.page(order_by, after, limit as usize) {
                Ok(rows) => Response::Rows { rows },
                Err(code) => err_response(code),
            },
            Err(code) => err_response(code),
        },
        Request::Compact => match store.compact() {
            Ok(report) => Response::Compacted {
                records: report.records as u64,
                slots_reclaimed: report.slots_reclaimed as u64,
                log_entries_folded: report.log_entries_folded as u64,
                edge_logs_folded: report.edge_logs_folded as u64,
            },
            Err(code) => err_response(code),
        },
        Request::DescribeSchema => Response::Schema(store.describe()),
        // `Authenticate` is intercepted directly by `handle_connection`,
        // which has the per-connection state (and `ServeOptions`) this
        // function has no way to reach — a store has no notion of "this
        // connection". Reaching this arm at all means `handle_connection`
        // let an `Authenticate` request fall through, which never happens
        // in the real dispatch loop; kept exhaustive rather than `_ =>`
        // so a future `Request` variant can't silently skip this decision.
        Request::Authenticate { .. } => err_response(ErrorCode::Unsupported),
        // Same story as `Authenticate`: `Hello` is a per-connection
        // negotiation `handle_connection` answers itself (`PROTO-FR-003`),
        // and a store has nothing to say about it.
        Request::Hello { .. } => err_response(ErrorCode::Unsupported),
        // Protocol 3, `SESS-FR-006`: a session is per-connection state
        // `handle_connection` owns; a store has nothing to say about it.
        Request::Begin | Request::BeginWith { .. } | Request::Commit | Request::Rollback => {
            err_response(ErrorCode::Unsupported)
        }
        // A one-shot `Request::Transaction` has no session, so no
        // snapshot-isolation read set to re-check — `ISO-FR-001` is
        // exclusively a session (`BeginWith`) feature.
        Request::Transaction { updates } => match store.apply_transaction(&updates, &[]) {
            Ok(()) => Response::Ok,
            Err((index, code)) => Response::TransactionFailed {
                index,
                code,
                message: error_message(code).to_string(),
            },
        },
        // `MET-FR-005` (ADR-0064): process-wide state `handle_connection`
        // owns (it needs `ServeOptions`, which a store-generic `dispatch`
        // has no way to reach) — the `Hello`/`Authenticate` precedent.
        // Reaching this arm at all never happens in the real dispatch
        // loop.
        Request::Metrics => err_response(ErrorCode::Unsupported),
        // `BAK-FR-001` (ADR-0065): same story — `handle_backup` needs
        // `ServeOptions::backup_root`, which `dispatch` cannot reach.
        Request::Backup { .. } => err_response(ErrorCode::Unsupported),
        // `RPL-FR-003` (ADR-0067): unlike `Backup`, this needs no
        // `ServeOptions` access beyond the `TokenClass::Replication`
        // gate `handle_connection` already checked before dispatching —
        // so, unlike `Backup`, it goes through the generic loop.
        Request::FetchSnapshot => match store.fetch_snapshot() {
            Ok(files) => Response::Snapshot { files },
            Err(code) => err_response(code),
        },
        // `FPG-FR-002`/`FPG-FR-003` (ADR-0068): `Page`'s own
        // validate-then-scan shape, composed with `Query`'s filter
        // validation. Gated in `handle_connection` like `Page`.
        Request::FilteredPage {
            order_by,
            after,
            limit,
            filter,
        } => match validate_filtered_page(
            &store.describe(),
            order_by,
            after.as_ref(),
            limit,
            &filter,
        ) {
            Ok(()) => match store.filtered_page(order_by, after, limit as usize, &filter) {
                Ok(rows) => Response::Rows { rows },
                Err(code) => err_response(code),
            },
            Err(code) => err_response(code),
        },
    }
}

/// Write `resp` and flush, reporting whether the connection is still
/// usable — folds the write-then-flush-then-check-both boilerplate every
/// response path in [`handle_connection`] needs (there are now several,
/// since auth gating adds early-return response paths that don't go
/// through [`dispatch`]).
fn send_response(writer: &mut BufWriter<WriteHalf>, resp: &Response) -> bool {
    if framing::write_message(writer, resp).is_err() {
        return false;
    }
    writer.flush().is_ok()
}

/// Serve one already-accepted connection until the client disconnects or a
/// framing error occurs. Never panics on a bad client: a malformed or
/// oversized frame ends the connection after (when possible) one
/// [`Response::Err`], never the process — `SERVER-FR-004`.
///
/// # Transport encryption (`TLS-FR-002`/`TLS-FR-003`), ADR-0014
///
/// When `tls` is configured, the raw `stream` is wrapped in a TLS server
/// connection (`rusty_tls::TlsAcceptor::accept`) before anything else
/// happens — `dispatch`/`ConnectionStore` never see this. `accept` itself
/// performs no I/O; the handshake runs lazily, driven by the very first
/// `framing::read_message` call below, so a connection that fails the
/// handshake surfaces there as an ordinary I/O error and ends the
/// connection cleanly — the same "return on the first framing error, no
/// panic" path a malformed plaintext frame already takes, satisfying
/// `TLS-FR-003` with no special-casing needed.
///
/// # Authentication gating (`AUTH-FR-001`/`AUTH-FR-002`/`AUTH-FR-003`/`AUTH-FR-007`)
///
/// `auth` is checked once per connection, not per request, to decide the
/// starting state: if no tokens are configured at all, the connection
/// starts already authenticated at [`TokenClass::ReadWrite`] and every
/// request is allowed exactly as before this feature existed
/// (`AUTH-FR-007`) — `Request::Authenticate` still round-trips
/// successfully in that case, but as a no-op. Otherwise the connection
/// starts unauthenticated: every request except `Authenticate` is
/// rejected with `ErrorCode::Unauthenticated` (including `DescribeSchema`
/// — `AUTH-FR-002`) until a recognized token is presented, after which its
/// [`TokenClass`] gates `Request::UpdateField` and `Request::Transaction`
/// (`AUTH-FR-003`, `TXN-FR-004`). With `tls` also configured,
/// `Authenticate`'s token now travels over the encrypted channel rather
/// than plaintext (`TLS-FR-007`) — the handshake above always completes
/// before this loop ever reads a frame, so there's no ordering hazard.
///
/// # Protocol version negotiation (`PROTO-FR-003`/`PROTO-FR-004`), ADR-0022
///
/// `Request::Hello` is intercepted ahead of even the `Authenticate`
/// intercept — a client learns the server's protocol version before it
/// presents a token, and an unauthenticated connection can say `Hello`
/// and nothing else. Only the first frame may be a `Hello`, and its
/// version must be at least 1: the reply is `Response::Hello` carrying
/// `min(client, PROTOCOL_VERSION)`; otherwise `ErrorCode::Malformed`,
/// with the connection left open. A connection whose first frame is not
/// a `Hello` is served at version 1 with no other change (`PROTO-FR-002`).
///
/// # Transaction sessions (`SESS-FR-002`–`SESS-FR-006`), ADR-0024
///
/// Protocol 3 adds the first per-connection state after `authenticated`:
/// the *negotiated version* (kept at last — `ADR-0022` deferred it until
/// a gated variant existed) and an optional *session*, a buffer of
/// staged `TransactionOp`s. `Begin` opens it; while it is open every
/// `UpdateField` the gates admit is pushed and answered `Staged { index }`
/// — nothing applied, no lock taken, no validation (commit validates);
/// `Commit` hands the buffer to `ConnectionStore::apply_transaction`
/// exactly as a `Request::Transaction` would and closes the session
/// either way; `Rollback` or a disconnect discards it. No lock is ever
/// held between round trips — the only lock a session takes is
/// `apply_transaction`'s own, at `Commit`, for the same interval a
/// `Transaction` of the same batch holds it (`SESS-FR-003`). The three
/// requests sit *after* the auth and `ReadOnly` gates (`Commit` joins
/// `UpdateField`/`Transaction` in the latter) and are `Malformed` on a
/// connection negotiated below 3 — a silent client included — so no
/// version-3 response shape is ever sent on an older connection
/// (compatibility rule 3). Misuse (`NoSession`, `SessionOpen`,
/// `SessionFull`) is a typed error with the connection open.
///
/// # Audit (`AUD-FR-004`–`006`), ADR-0029
///
/// Every decision the gates below take is recorded on
/// [`ServeOptions::audit`] after the decision and before the response —
/// `Admitted` (after an *eager* TLS handshake, so a refused admission is
/// `HandshakeFailed` with the TLS error's text; `classed_by_certificate`
/// records whether `initial_class` came from a matched certificate —
/// `ADR-0029`'s fourth revisit trigger, taken once `ADR-0028` landed),
/// `Authenticated` / `AuthenticationFailed`, `Refused` at the
/// unauthenticated and `ReadOnly` gates, and exactly one `Disconnected`
/// on the way out. No successful request is recorded; no token,
/// certificate, id, or value ever is. With the default
/// [`super::audit::NoAudit`] every call is a no-op.
///
/// # Stage-time validation (`STV-FR-001`–`003`), `ADR-0024`'s second trigger
///
/// Protocol 6 adds a second `BeginWith` bit, `SESSION_VALIDATE_ON_STAGE`:
/// each `UpdateField` is passed to `ConnectionStore::validate_op` as it
/// is staged and refused — nothing staged — with the code `Commit`
/// would have reported, so a client learns about a bad write at the
/// round trip that sent it rather than by index at `Commit`. Commit
/// still validates the whole batch (the store's rule, not this one's).
/// Below version 6 the bit is unknown and `BeginWith` is `Malformed`.
///
/// # Read-your-writes sessions (`RYW-FR-001`–`005`), ADR-0027
///
/// Protocol 5 adds `BeginWith { flags }`: with `SESSION_READ_YOUR_WRITES`
/// the session also remembers the schema's updatable field tags, and this
/// connection's own `GetById` — and only that read — is served as usual
/// and then passed through [`overlay_staged`] before it is sent. Set
/// reads, plain `Begin` sessions, and every other connection see
/// committed state exactly as before; a connection with no session takes
/// no new branch. Unknown flag bits are `Malformed`; below version 5 the
/// request is `Malformed` (rule 3), like `Begin` below 3.
/// Records `Disconnected` when a connection's `handle_connection` frame
/// returns by any path (`AUD-FR-004`) — created only after `Admitted`.
struct DisconnectAudit<'a> {
    sink: &'a dyn audit::AuditSink,
    peer: Option<std::net::SocketAddr>,
}

impl Drop for DisconnectAudit<'_> {
    fn drop(&mut self) {
        self.sink.record(&audit::AuditEvent::now(
            self.peer,
            audit::AuditKind::Disconnected,
        ));
    }
}

/// `MVCC2-FR-005` (`ADR-0072`): releases an open real-MVCC snapshot's
/// registration from its table's open-snapshot set on every return path
/// out of `handle_connection` — including an ungraceful disconnect, not
/// only `Commit`/`Rollback`'s own explicit release. Without this, a
/// dropped connection would leave a stale registration pinning
/// `Compact`'s GC boundary forever, growing that table's MVCC history
/// unbounded.
struct MvccReleaseGuard<'a> {
    tables: &'a [(String, Arc<dyn ConnectionStore>)],
    snapshot: &'a Cell<Option<(usize, u64)>>,
}

impl Drop for MvccReleaseGuard<'_> {
    fn drop(&mut self) {
        if let Some((table, snapshot_txn)) = self.snapshot.get() {
            self.tables[table].1.mvcc_release(snapshot_txn);
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    tables: &[(String, Arc<dyn ConnectionStore>)],
    primary: usize,
    options: &ServeOptions,
) {
    // `SRV-FR-004` (ADR-0032): `tls` was `serve`'s own second parameter;
    // now it is read off the one consolidated `options` value.
    let tls = options.tls();
    // This is a synchronous request/response protocol: each side writes a
    // small frame, then blocks reading the other side's small frame back.
    // Left at its default, Nagle's algorithm delays a small write hoping
    // to coalesce it with more data, which collides with the peer's own
    // delayed-ACK timer — the textbook interaction that turns every
    // round trip into a ~40ms stall. Disabling it is the correct fix for
    // this protocol shape, not just a benchmark convenience: confirmed
    // directly (a concurrent-client integration test went from ~36s to
    // well under a second after this one call).
    let _ = stream.set_nodelay(true);
    // `AUD-FR-001`: the one identifying datum an audit event carries.
    let peer = stream.peer_addr().ok();
    let sink = options.audit();

    let transport = match tls {
        None => audit::Transport::Plain,
        Some(tls) if tls.requires_client_certificate() => audit::Transport::MutualTls,
        Some(_) => audit::Transport::Tls,
    };
    // `CLS-FR-004`: the class a presented, configured certificate grants,
    // if the TLS arm below finds one — `None` on a plain connection or an
    // admitted leaf that matches no configured class.
    let mut certificate_class: Option<TokenClass> = None;
    let (mut reader, mut writer): (BufReader<ReadHalf>, BufWriter<WriteHalf>) = match tls {
        None => {
            let peer_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return,
            };
            (
                BufReader::new(ReadHalf::Plain(stream)),
                BufWriter::new(WriteHalf::Plain(peer_stream)),
            )
        }
        Some(tls) => {
            let mut tls_stream = match tls.acceptor.accept(stream) {
                Ok(s) => s,
                Err(_) => return, // config/setup error building the connection object — drop cleanly, no panic
            };
            // `AUD-FR-005` (and `CLS-FR-002`): complete the handshake before
            // the frame loop, so a refused admission has a typed reason to
            // record. Client-visible behavior is unchanged — the connection
            // ends with no response either way (`TLS-FR-003`).
            if let Err(e) = tls_stream.complete_handshake() {
                sink.record(&audit::AuditEvent::now(
                    peer,
                    audit::AuditKind::HandshakeFailed {
                        reason: e.to_string(),
                    },
                ));
                return;
            }
            // `CLS-FR-003`/`CLS-FR-004`: the class a presented leaf grants,
            // if its exact DER bytes are configured — `None` on a plain
            // acceptor (no client auth) or a leaf not in the map.
            certificate_class = tls_stream
                .peer_certificate_der()
                .and_then(|der| options.class_for_certificate(der));
            let shared = Rc::new(RefCell::new(tls_stream));
            (
                BufReader::new(ReadHalf::Tls(Rc::clone(&shared))),
                BufWriter::new(WriteHalf::Tls(shared)),
            )
        }
    };

    // `CLS-FR-004`: a certificate-classed connection starts at that class
    // with no `Authenticate` needed; a later `Authenticate` with a valid
    // token still replaces it (unchanged below). Otherwise exactly
    // today's rule: unauthenticated if anything is configured, `ReadWrite`
    // if nothing is.
    let mut authenticated: Option<TokenClass> = match (certificate_class, options.is_configured()) {
        (Some(class), _) => Some(class),
        (None, false) => Some(TokenClass::ReadWrite),
        (None, true) => None,
    };
    // `AUD-FR-004`: admitted — and exactly one `Disconnected` when this
    // function returns by any path.
    sink.record(&audit::AuditEvent::now(
        peer,
        audit::AuditKind::Admitted {
            transport,
            initial_class: authenticated,
            classed_by_certificate: certificate_class.is_some(),
        },
    ));
    let _disconnect = DisconnectAudit { sink, peer };
    // `MET-FR-002` (ADR-0064): the matching close runs on every return
    // path via `Drop`, the identical guard shape `_disconnect` above
    // already uses.
    options.metrics().record_connection_opened();
    let _metrics_guard = ConnectionMetricsGuard(options.metrics());
    // `MVCC2-FR-005` (`ADR-0072`): the `(table index, snapshot_txn)` of an
    // open real-MVCC session, if any — a `Cell` so [`MvccReleaseGuard`]'s
    // `Drop` (which fires on *every* return path, including an ungraceful
    // disconnect) can read it without fighting the loop body's own
    // ordinary reads/writes for a borrow. `Use` is refused while any
    // session is open (`TBL-FR-002`'s own rule), so the table index
    // recorded here never goes stale while a snapshot is registered
    // against it.
    let mvcc_snapshot: Cell<Option<(usize, u64)>> = Cell::new(None);
    let _mvcc_release_guard = MvccReleaseGuard {
        tables,
        snapshot: &mvcc_snapshot,
    };

    // `PROTO-FR-004`: only the very first frame may be a `Hello`. Since
    // protocol 3 the negotiated version is kept too (`SESS-FR-006`): the
    // session requests consult it, exactly the moment `ADR-0022` said the
    // state would appear. A silent client is version 1.
    let mut first_frame = true;
    let mut negotiated: u32 = 1;
    // `TBL-FR-002` (ADR-0050): which of `tables` this connection's
    // table-less requests are served from — the primary until a `Use`.
    let mut table: usize = primary;
    // `SESS-FR-002`: the staged writes of an open session, if any.
    let mut session: Option<Vec<TransactionOp>> = None;
    // `RYW-FR-001`: `Some(updatable tags)` while a read-your-writes
    // session is open; cleared with the session.
    let mut read_your_writes: Option<Vec<FieldRef>> = None;
    // `STV-FR-001`: whether the open session validates each write as it
    // is staged; cleared with the session.
    let mut validate_on_stage = false;
    // `ISO-FR-002` (ADR-0033): `Some(read set)` while a snapshot-isolated
    // session is open — every `GetById`'s raw, pre-overlay result is
    // recorded here, keyed by `(id, field)`; cleared with the session.
    let mut snapshot_reads: Option<HashMap<(RecordId, FieldRef), ScanValue>> = None;
    // `RL-FR-001`: this connection's own failed-`Authenticate` count,
    // never reset by a success — the fifth failure locks it out
    // regardless of how many succeeded around it.
    let mut failures: u32 = 0;
    let peer_ip = peer.map(|addr| addr.ip());

    loop {
        let store: &dyn ConnectionStore = tables[table].1.as_ref();
        let req: Request = match framing::read_message(&mut reader) {
            Ok(req) => req,
            Err(_) => return, // client disconnected, or a framing/decode error — end the connection
        };

        // `RLH-FR-003` (ADR-0088): the request's arrival — its frame fully
        // read — from which a dispatched request's latency is measured.
        let arrived = Instant::now();
        if let Request::Hello { protocol_version } = &req {
            let resp = if !first_frame || *protocol_version == 0 {
                err_response(ErrorCode::Malformed)
            } else {
                negotiated = (*protocol_version).min(PROTOCOL_VERSION);
                Response::Hello {
                    protocol_version: negotiated,
                }
            };
            first_frame = false;
            if !send_response(&mut writer, &resp) {
                return;
            }
            continue;
        }
        first_frame = false;

        if let Request::Authenticate { token } = &req {
            let resp = if !options.is_configured() {
                sink.record(&audit::AuditEvent::now(
                    peer,
                    audit::AuditKind::Authenticated {
                        class: TokenClass::ReadWrite,
                    },
                ));
                Response::Ok
            } else if options.is_throttled(peer_ip) {
                // `RL-FR-002`: over budget — refused before any
                // comparison, and it still counts toward the
                // per-connection lockout below.
                failures += 1;
                sink.record(&audit::AuditEvent::now(
                    peer,
                    audit::AuditKind::Throttled { failures },
                ));
                err_response(ErrorCode::Unauthenticated)
            } else {
                match options.check(token) {
                    Some(class) => {
                        authenticated = Some(class);
                        sink.record(&audit::AuditEvent::now(
                            peer,
                            audit::AuditKind::Authenticated { class },
                        ));
                        Response::Ok
                    }
                    None => {
                        failures += 1;
                        options.note_failure(peer_ip);
                        sink.record(&audit::AuditEvent::now(
                            peer,
                            audit::AuditKind::AuthenticationFailed,
                        ));
                        err_response(ErrorCode::Unauthenticated)
                    }
                }
            };
            if !send_response(&mut writer, &resp) {
                return;
            }
            // `RL-FR-001`: the response above is sent either way; only
            // *when the connection closes* changes.
            if failures >= MAX_AUTH_FAILURES {
                sink.record(&audit::AuditEvent::now(
                    peer,
                    audit::AuditKind::LockedOut { failures },
                ));
                return;
            }
            continue;
        }

        let class = match authenticated {
            Some(class) => class,
            None => {
                sink.record(&audit::AuditEvent::now(
                    peer,
                    audit::AuditKind::Refused {
                        class: None,
                        request: audit::RequestKind::of(&req),
                        code: ErrorCode::Unauthenticated,
                    },
                ));
                if !send_response(&mut writer, &err_response(ErrorCode::Unauthenticated)) {
                    return;
                }
                continue;
            }
        };

        if class == TokenClass::ReadOnly
            && matches!(
                req,
                Request::UpdateField { .. }
                    | Request::Transaction { .. }
                    | Request::Commit
                    | Request::Insert { .. }
                    | Request::Link { .. }
                    | Request::Replace { .. }
                    | Request::Delete { .. }
                    | Request::Compact
                    | Request::ReplaceIf { .. }
                    | Request::WriteBatch { .. }
                    | Request::Backup { .. }
            )
        {
            sink.record(&audit::AuditEvent::now(
                peer,
                audit::AuditKind::Refused {
                    class: Some(class),
                    request: audit::RequestKind::of(&req),
                    code: ErrorCode::Unauthorized,
                },
            ));
            if !send_response(&mut writer, &err_response(ErrorCode::Unauthorized)) {
                return;
            }
            continue;
        }

        // `RPL-FR-002` (ADR-0067): `FetchSnapshot` is the one request
        // that requires `TokenClass::Replication` specifically — refused
        // for `ReadOnly` *and* `ReadWrite` both, the opposite shape from
        // the `ReadOnly`-blocks-writes gate just above (that one blocks
        // one class from many requests; this one blocks many classes
        // from one request). This is why `TokenClass::Replication`
        // cannot be folded into a boolean flag on `ReadWrite` — the
        // match above already proves at compile time that no future
        // `TokenClass` variant is silently exempted here.
        if matches!(req, Request::FetchSnapshot) && class != TokenClass::Replication {
            sink.record(&audit::AuditEvent::now(
                peer,
                audit::AuditKind::Refused {
                    class: Some(class),
                    request: audit::RequestKind::of(&req),
                    code: ErrorCode::Unauthorized,
                },
            ));
            if !send_response(&mut writer, &err_response(ErrorCode::Unauthorized)) {
                return;
            }
            continue;
        }

        // `ACC-FR-004`: everything from here on is a dispatched request —
        // past `Hello`/`Authenticate` (handled above) and the
        // unauthenticated/`ReadOnly` gates (also above, each its own
        // `continue`) — so `request_kind` is captured now, before the
        // match below consumes `req`.
        let request_kind = audit::RequestKind::of(&req);
        // `QPM-FR-003` (ADR-0086): the plan a planned read will take,
        // classified before the match consumes `req`; recorded below
        // only when the response carries no error.
        let plan = plan_of(store, &req);

        // `SESS-FR-002`/`SESS-FR-004`/`SESS-FR-006`: the session intercepts.
        let resp = match req {
            Request::Begin | Request::Commit | Request::Rollback if negotiated < 3 => {
                err_response(ErrorCode::Malformed)
            }
            Request::Begin => {
                if session.is_some() {
                    err_response(ErrorCode::SessionOpen)
                } else {
                    session = Some(Vec::new());
                    read_your_writes = None;
                    validate_on_stage = false;
                    snapshot_reads = None;
                    mvcc_snapshot.set(None);
                    Response::Ok
                }
            }
            Request::BeginWith { .. } if negotiated < 5 => err_response(ErrorCode::Malformed),
            Request::BeginWith { flags } => {
                // A flag bit is introduced at a version like a variant
                // (`STV-FR-003`): below 6 the validate bit is unknown,
                // below 7 the snapshot-isolation bit is unknown
                // (`ISO-FR-001`), below 27 the real-MVCC bit is unknown
                // (`MVCC2-FR-004`).
                let known = SESSION_READ_YOUR_WRITES
                    | if negotiated >= 6 {
                        SESSION_VALIDATE_ON_STAGE
                    } else {
                        0
                    }
                    | if negotiated >= 7 {
                        SESSION_SNAPSHOT_ISOLATION
                    } else {
                        0
                    }
                    | if negotiated >= 27 {
                        SESSION_MVCC_ISOLATION
                    } else {
                        0
                    };
                if flags & !known != 0 {
                    err_response(ErrorCode::Malformed)
                } else if session.is_some() {
                    err_response(ErrorCode::SessionOpen)
                } else if flags & SESSION_MVCC_ISOLATION != 0 && !store.mvcc_supported() {
                    // `MVCC2-FR-012`: a domain that doesn't implement real
                    // MVCC (`Dog`/`Order`/`Employee`) refuses the bit —
                    // `Unsupported`, the same precedent `Insert`/`Compact`
                    // already established for a domain-scoped capability.
                    // Nothing is staged; no session opens.
                    err_response(ErrorCode::Unsupported)
                } else {
                    session = Some(Vec::new());
                    read_your_writes = (flags & SESSION_READ_YOUR_WRITES != 0).then(|| {
                        store
                            .describe()
                            .fields
                            .iter()
                            .filter(|f| f.capabilities.update)
                            .map(|f| f.tag)
                            .collect()
                    });
                    validate_on_stage = flags & SESSION_VALIDATE_ON_STAGE != 0;
                    snapshot_reads = (flags & SESSION_SNAPSHOT_ISOLATION != 0).then(HashMap::new);
                    mvcc_snapshot.set(
                        (flags & SESSION_MVCC_ISOLATION != 0).then(|| (table, store.mvcc_begin())),
                    );
                    Response::Ok
                }
            }
            Request::Rollback => {
                read_your_writes = None;
                validate_on_stage = false;
                snapshot_reads = None;
                // `MVCC2-FR-005`: release before clearing — `Commit`'s own
                // arm below follows the identical order.
                if let Some((table, snapshot_txn)) = mvcc_snapshot.take() {
                    tables[table].1.mvcc_release(snapshot_txn);
                }
                if session.take().is_some() {
                    Response::Ok
                } else {
                    err_response(ErrorCode::NoSession)
                }
            }
            Request::GetById { id }
                if (read_your_writes.is_some()
                    || snapshot_reads.is_some()
                    || mvcc_snapshot.get().is_some())
                    && session.is_some() =>
            {
                // `MVCC2-FR-006`: a real-MVCC session answers from the
                // version index as of its own snapshot, not the store's
                // current committed state — a genuinely different read
                // path from `dispatch`'s ordinary `GetById`, not an
                // overlay on top of it. Read-your-writes still composes
                // afterward, exactly as it does with the other two bits.
                let record = match mvcc_snapshot.get() {
                    Some((_, snapshot_txn)) => match store.mvcc_get(id, snapshot_txn) {
                        Ok(Some(fields)) => Some(Response::Record { id, fields }),
                        Ok(None) => None,
                        Err(code) => Some(err_response(code)),
                    },
                    None => match dispatch(store, Request::GetById { id }) {
                        found @ Response::Record { .. } => Some(found),
                        _ => None,
                    },
                };
                match record {
                    Some(Response::Record { id, mut fields }) => {
                        // `ISO-FR-002`/`ISO-FR-005`: record the raw,
                        // committed values — before any read-your-writes
                        // overlay — into the read set; only a found
                        // record is tracked at all. Not recorded for a
                        // real-MVCC session: its own conflict check at
                        // `Commit` supersedes this read-set replay check.
                        if let Some(reads) = snapshot_reads.as_mut() {
                            record_read_set(reads, id, &fields);
                        }
                        if let (Some(staged), Some(updatable)) =
                            (session.as_ref(), read_your_writes.as_ref())
                        {
                            overlay_staged(id, &mut fields, staged, updatable);
                        }
                        Response::Record { id, fields }
                    }
                    Some(other) => other,
                    None => Response::NotFound,
                }
            }
            Request::Commit => {
                read_your_writes = None;
                validate_on_stage = false;
                // `ISO-FR-003`: whatever this session tracked is handed
                // to `apply_transaction` alongside the staged batch, to
                // be re-checked atomically with the apply — empty when
                // snapshot isolation was never turned on.
                let read_set: Vec<(RecordId, FieldRef, ScanValue)> = snapshot_reads
                    .take()
                    .map(|reads| reads.into_iter().map(|((id, f), v)| (id, f, v)).collect())
                    .unwrap_or_default();
                // `MVCC2-FR-005`: release after the apply, regardless of
                // outcome — a rejected commit still closes its snapshot.
                let mvcc = mvcc_snapshot.take();
                let outcome = session.take().map(|batch| match mvcc {
                    Some((_, snapshot_txn)) => {
                        store.apply_transaction_mvcc(&batch, &read_set, snapshot_txn)
                    }
                    None => store.apply_transaction(&batch, &read_set),
                });
                if let Some((table, snapshot_txn)) = mvcc {
                    tables[table].1.mvcc_release(snapshot_txn);
                }
                match outcome {
                    None => err_response(ErrorCode::NoSession),
                    Some(Ok(())) => Response::Ok,
                    Some(Err((index, code))) => Response::TransactionFailed {
                        index,
                        code,
                        message: error_message(code).to_string(),
                    },
                }
            }
            Request::UpdateField { id, field, value } if session.is_some() => {
                let op = TransactionOp { id, field, value };
                // `STV-FR-001`: a validating session refuses a bad write
                // now, with the code `Commit` would have given; nothing
                // is staged.
                let refused = if validate_on_stage {
                    store.validate_op(&op).err()
                } else {
                    None
                };
                match (refused, session.as_mut()) {
                    (Some(code), _) => err_response(code),
                    (None, Some(staged)) if staged.len() < MAX_STAGED_OPS => {
                        staged.push(op);
                        Response::Staged {
                            index: (staged.len() - 1) as u32,
                        }
                    }
                    _ => err_response(ErrorCode::SessionFull),
                }
            }
            Request::Transaction { .. } if session.is_some() => {
                err_response(ErrorCode::SessionOpen)
            }
            // `INS-FR-007` (ADR-0046): an insert is never staged — the
            // `Transaction`-inside-a-session rule — and, as a write, is
            // gated server-side below 13 like the session requests.
            Request::Insert { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Insert { .. } if negotiated < 13 => err_response(ErrorCode::Malformed),
            // `LNK-FR-010` (ADR-0047): the same two gates, at 14.
            Request::Link { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Link { .. } if negotiated < 14 => err_response(ErrorCode::Malformed),
            // `REP-FR-006` (ADR-0049): the same two gates, at 15.
            Request::Replace { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Replace { .. } if negotiated < 15 => err_response(ErrorCode::Malformed),
            // `DEL-FR-006` (ADR-0051): the same two gates, at 17.
            Request::Delete { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Delete { .. } if negotiated < 17 => err_response(ErrorCode::Malformed),
            // `CMP-FR-006` (ADR-0052): the same two gates, at 18.
            Request::Compact if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Compact if negotiated < 18 => err_response(ErrorCode::Malformed),
            // `GRD-FR-005` (ADR-0054): the same two gates, at 19.
            Request::ReplaceIf { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::ReplaceIf { .. } if negotiated < 19 => err_response(ErrorCode::Malformed),
            // `JOIN-FR-001`/`002` (ADR-0044), compatibility rule 3: the two
            // protocol-12 requests are unknown to a connection negotiated
            // below 12 — the session precedent, not `Query`'s client-only
            // gate, per the accepted design text.
            Request::Join(_) | Request::DescribeRelations if negotiated < 12 => {
                err_response(ErrorCode::Malformed)
            }
            // `PAG-FR-004` (ADR-0055), rule 3: a read, gated like `Join`.
            Request::Page { .. } if negotiated < 20 => err_response(ErrorCode::Malformed),
            // `CNT-FR-003` (ADR-0057), rule 3: a read, gated like `Page`.
            Request::CountEdges { .. } if negotiated < 21 => err_response(ErrorCode::Malformed),
            // `WBT-FR-003` (ADR-0060): a write, gated like the other
            // runtime writes — `SessionOpen` inside a session, `Malformed`
            // below 22, and `Malformed` for a batch over `MAX_BATCH_OPS`
            // (nothing applied).
            Request::WriteBatch { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::WriteBatch { .. } if negotiated < 22 => err_response(ErrorCode::Malformed),
            Request::WriteBatch { ref ops, .. } if ops.len() > MAX_BATCH_OPS => {
                err_response(ErrorCode::Malformed)
            }
            // `MET-FR-005` (ADR-0064): a read — no `SessionOpen` gate,
            // unlike every write above (harmless mid-session; never
            // reaches `ConnectionStore` at all).
            Request::Metrics if negotiated < 23 => err_response(ErrorCode::Malformed),
            Request::Metrics => Response::Metrics {
                text: options.metrics().render(),
            },
            // `BAK-FR-001` (ADR-0065): the same two gates every other
            // operator write uses (`Compact`'s precedent), at 24; the
            // real work is `handle_backup`'s; it needs `options`, which
            // `dispatch` cannot reach.
            Request::Backup { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Backup { .. } if negotiated < 24 => err_response(ErrorCode::Malformed),
            Request::Backup { name } => handle_backup(store, options, &name),
            // `RPL-FR-002` (ADR-0067): a read, not a write — no
            // `SessionOpen` gate (`Metrics`'s precedent) — but still
            // `Malformed` below the protocol version that introduced it.
            // The `TokenClass::Replication` gate ran earlier, above; the
            // real work is `dispatch`'s own `Request::FetchSnapshot` arm
            // (reached via the `other => dispatch(store, other)` fallback
            // below), since — unlike `Backup` — it needs no `options`
            // access this match arm would otherwise have to thread through.
            Request::FetchSnapshot if negotiated < 25 => err_response(ErrorCode::Malformed),
            // `FPG-FR-007` (ADR-0068): a read, `Page`'s own gate shape —
            // no `SessionOpen` gate, `Malformed` below the protocol
            // version that introduced it. Falls through to `dispatch`'s
            // own `Request::FilteredPage` arm via the `other =>` fallback.
            Request::FilteredPage { .. } if negotiated < 26 => err_response(ErrorCode::Malformed),
            // `TBL-FR-002`/`003` (ADR-0050): both protocol-16 requests are
            // gated like the session requests (rule 3); `Use` inside a
            // session is `SessionOpen`, since a session's staged writes
            // belong to one table; an unknown name is `Malformed`.
            Request::Use { .. } | Request::ListTables if negotiated < 16 => {
                err_response(ErrorCode::Malformed)
            }
            Request::Use { .. } if session.is_some() => err_response(ErrorCode::SessionOpen),
            Request::Use { table: name } => match tables.iter().position(|(n, _)| *n == name) {
                Some(i) => {
                    table = i;
                    Response::Ok
                }
                None => err_response(ErrorCode::Malformed),
            },
            Request::ListTables => Response::Tables {
                names: tables.iter().map(|(n, _)| n.clone()).collect(),
                primary: tables[primary].0.clone(),
            },
            // `TBL-FR-004` (ADR-0050): a cross-table join — the right rows
            // from the named table's adapter, the relation the left's.
            Request::Join(spec) if spec.right_table.is_some() => join_across(tables, store, &spec),
            // `TBL-FR-007` (ADR-0050): a link under a relation whose rows
            // live in another table has its far endpoint checked there.
            Request::Link {
                left,
                right,
                relation,
            } => link_across(tables, store, left, right, relation),
            // `DEL-FR-007` (ADR-0051): a delete here detaches the id from
            // every other table's relation that targets this one.
            Request::Delete { id } => delete_across(tables, store, id),
            other => dispatch(store, other),
        };
        let resp = downgrade_for_version(resp, negotiated);
        // `MET-FR-003` (ADR-0064): the identical call site `AccessEvent`
        // is recorded at — one increment per dispatched request.
        let ok = matches!(outcome_of(&resp), access::Outcome::Ok);
        options.metrics().record_request(ok);
        if let (true, Some(kind)) = (ok, plan) {
            options.metrics().record_plan(kind);
        }
        options.metrics().record_latency(arrived.elapsed());
        // `ACC-FR-004`: after the audit log's own recording for this path
        // (if any — the gates above already returned), one access event
        // per dispatched request, before the response is sent.
        options.access_log().record(&access::AccessEvent::now(
            peer,
            Some(class),
            request_kind,
            outcome_of(&resp),
        ));
        if !send_response(&mut writer, &resp) {
            return;
        }
    }
}

/// Accept connections on `listener` and serve each one on its own OS
/// thread against the same shared `store` — the thread-per-connection
/// model ADR-0010 chose over an async runtime. Every connection thread
/// takes only `&S`; all coordination is whatever locking `store` already
/// does internally (see this module's own doc comment). `options` is
/// shared (`Arc`) across every connection thread the same way `store`
/// is — see this module's own `handle_connection` for the gating/
/// handshake it performs. `options.tls()`'s `None` reproduces plaintext
/// behavior exactly (`TLS-FR-008`); configured, it requires every
/// connection to complete a TLS handshake before any request is served.
/// Runs until `listener` itself errors (e.g. the socket is closed) or
/// forever otherwise — a real deployment's shutdown/drain story is an
/// explicit non-goal of the accepted design, not solved here.
///
/// `options` consolidates every cross-cutting server concern —
/// tokens, certificate classes, the audit/access-log sinks, the
/// rate-limit budget, and (since `ADR-0032`) native TLS — into `serve`'s
/// one configuration parameter (`SRV-FR-001`/`SRV-FR-004`); before this
/// it was `ServeOptions` (then named `AuthConfig`) plus a separate
/// `Option<TlsConfig>` parameter.
pub fn serve<S: ConnectionStore + 'static>(
    listener: TcpListener,
    store: Arc<S>,
    options: ServeOptions,
) {
    let name = store.table_name().to_string();
    serve_tables(listener, vec![(name, store)], 0, options);
}

/// `TBL-FR-001` (ADR-0045, implemented by ADR-0050): [`serve`] for more
/// than one table. Every adapter in `tables` is served on this one
/// listener under its name; a connection starts on `tables[primary]`
/// and moves with [`Request::Use`]. [`serve`] is exactly this with one
/// table named by `ConnectionStore::table_name`, so a one-table server
/// is unchanged. One `options` — tokens, TLS, logs, rate limit — is
/// shared by every table; per-table authorization is a named non-goal.
///
/// # Panics
///
/// Panics if `tables` is empty or `primary` is out of range — a
/// misconfiguration at startup, not a runtime condition.
pub fn serve_tables(
    listener: TcpListener,
    tables: Vec<(String, Arc<dyn ConnectionStore>)>,
    primary: usize,
    mut options: ServeOptions,
) {
    let metrics_listener = options.metrics_http.take();
    assert!(
        primary < tables.len(),
        "serve_tables: primary {primary} is not one of the {} tables",
        tables.len()
    );
    let tables = Arc::new(tables);
    let options = Arc::new(options);
    if let Some(listener) = metrics_listener {
        let options = Arc::clone(&options);
        thread::spawn(move || super::metrics_http::serve_metrics_http(listener, options));
    }
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(s) => s,
            Err(_) => continue, // one bad accept doesn't take down the server
        };
        let tables = Arc::clone(&tables);
        let options = Arc::clone(&options);
        thread::spawn(move || handle_connection(stream, &tables, primary, options.as_ref()));
    }
}

/// `TBL-FR-004` (ADR-0050): [`Request::Join`] with `right_table: Some`
/// — validated with the right table's own schema, evaluated with the
/// left adapter's relation and the right adapter's `get`. A right table
/// this server did not register, or one the relation's descriptor does
/// not name, is `Malformed` (`validate_join`).
fn join_across(
    tables: &[(String, Arc<dyn ConnectionStore>)],
    left: &dyn ConnectionStore,
    spec: &JoinSpec,
) -> Response {
    let right = spec
        .right_table
        .as_deref()
        .and_then(|name| tables.iter().find(|(n, _)| n == name))
        .map(|(_, store)| store.as_ref());
    let right_schema = right.map(|r| r.describe());
    match validate_join(
        &left.describe(),
        &left.describe_relations(),
        right_schema.as_ref(),
        spec,
    ) {
        Ok(()) => Response::JoinedRows {
            rows: evaluate_join(
                left,
                right.expect("validate_join accepted, so the right table resolved"),
                spec,
            ),
        },
        Err(code) => err_response(code),
    }
}

/// `DEL-FR-007` (ADR-0051): [`Request::Delete`] on a `serve_tables`
/// server — the table's own `delete_record` (which drops the record's
/// edges within that table), then, only on `Ok`, every *other* table's
/// `detach_record` for each of its relations whose `target_table` is
/// this one: the consumer's `DELETE FROM memory_entities WHERE
/// entity_id = ?`. An adapter answering `Unsupported`/`Malformed` for
/// the detach has nothing to drop and is skipped; a `Storage` failure
/// there is reported in the delete's place, the record itself already
/// gone (the one partial state, named in the design). A crash between
/// the two steps leaves edges to a record no table holds, which every
/// read already skips (`evaluate_join`'s `get` miss).
fn delete_across(
    tables: &[(String, Arc<dyn ConnectionStore>)],
    store: &dyn ConnectionStore,
    id: RecordId,
) -> Response {
    let resp = dispatch(store, Request::Delete { id });
    if resp != Response::Ok {
        return resp;
    }
    let this = store.table_name();
    for (name, other) in tables {
        if name == this {
            continue;
        }
        for relation in other.describe_relations() {
            if relation.target_table.as_deref() != Some(this) {
                continue;
            }
            match other.detach_record(&relation.name, id) {
                Ok(_) | Err(ErrorCode::Unsupported) | Err(ErrorCode::Malformed) => {}
                Err(code) => return err_response(code),
            }
        }
    }
    Response::Ok
}

/// `TBL-FR-007` (ADR-0050): [`Request::Link`] under a relation whose
/// descriptor names a `target_table` — the far endpoint must exist in
/// *that* table (`RecordNotFound` otherwise; `Unsupported` when the
/// server registered no such table), which the left adapter cannot
/// check itself. A relation with no `target_table` goes straight to
/// `dispatch`, exactly as before this round.
fn link_across(
    tables: &[(String, Arc<dyn ConnectionStore>)],
    store: &dyn ConnectionStore,
    left: RecordId,
    right: RecordId,
    relation: String,
) -> Response {
    let foreign = store
        .describe_relations()
        .into_iter()
        .find(|r| r.name == relation)
        .and_then(|r| r.target_table);
    if let Some(target) = foreign {
        match tables.iter().find(|(n, _)| *n == target) {
            None => return err_response(ErrorCode::Unsupported),
            Some((_, other)) if other.get(right).is_none() => {
                return err_response(ErrorCode::RecordNotFound)
            }
            Some(_) => {}
        }
    }
    dispatch(
        store,
        Request::Link {
            left,
            right,
            relation,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MHTTP-FR-001`: neither default constructor opens an HTTP port;
    /// only the builder supplies a caller-owned listener.
    #[test]
    fn metrics_http_is_absent_until_a_listener_is_supplied() {
        assert!(ServeOptions::default().metrics_http.is_none());
        assert!(ServeOptions::new(None, None).metrics_http.is_none());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let options = ServeOptions::default().with_metrics_http(listener);
        assert_eq!(options.metrics_http.unwrap().local_addr().unwrap(), addr);
    }

    /// `MTLS-FR-004`: `TlsConfig::from_env`'s decision table, driven
    /// through the factored `from_env_values` so no real environment
    /// variable is read (the constraint `ServeOptions::from_env`'s docs
    /// impose on tests). Chain/key unset → `None`; the pair set → the
    /// PEM path (here an `Io` error, since the files do not exist);
    /// client CA set without the pair → `Some(Err(Io(NotFound)))`, never
    /// `None`.
    #[test]
    fn tls_from_env_values_treats_a_client_ca_without_a_chain_and_key_as_an_error() {
        let missing = |name: &str| Some(format!("/nonexistent/{name}"));
        assert!(TlsConfig::from_env_values(None, None, None).is_none());
        assert!(TlsConfig::from_env_values(missing("chain"), None, None).is_none());
        assert!(matches!(
            TlsConfig::from_env_values(missing("chain"), missing("key"), None),
            Some(Err(TlsConfigError::Io(_)))
        ));
        assert!(matches!(
            TlsConfig::from_env_values(missing("chain"), missing("key"), missing("ca")),
            Some(Err(TlsConfigError::Io(_)))
        ));
        match TlsConfig::from_env_values(None, None, missing("ca")).map(|r| r.map(|_| ())) {
            Some(Err(TlsConfigError::Io(e))) => {
                assert_eq!(e.kind(), io::ErrorKind::NotFound);
                assert!(e.to_string().contains("SERVER_TLS_CLIENT_CA_PATH"));
            }
            other => panic!("expected a NotFound Io error naming the variable, got {other:?}"),
        }
        match TlsConfig::from_env_values(missing("chain"), None, missing("ca"))
            .map(|r| r.map(|_| ()))
        {
            Some(Err(TlsConfigError::Io(e))) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
            other => panic!("expected a NotFound Io error, got {other:?}"),
        }
    }

    /// A minimal in-memory `ConnectionStore` fixture, independent of any
    /// real domain adapter — exercises `dispatch`'s own logic (response
    /// shape per request kind, error-code mapping) without needing
    /// `ProductionStore`/`GenericProductionStore` at all.
    struct FixtureStore;

    const FIELD_A: FieldRef = 0;

    impl ConnectionStore for FixtureStore {
        fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
            if id == RecordId::from_u128(1) {
                Some(vec![(FIELD_A, ScanValue::U32(7))])
            } else {
                None
            }
        }
        fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
            vec![(RecordId::from_u128(1), vec![(FIELD_A, ScanValue::U32(7))])]
        }
        fn filter_eq(
            &self,
            field: FieldRef,
            _value: &ScanValue,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            if field == FIELD_A {
                Ok(vec![RecordId::from_u128(1)])
            } else {
                Err(ErrorCode::UnknownField)
            }
        }
        fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
            if field == FIELD_A {
                Ok(vec![ScanValue::U32(7)])
            } else {
                Err(ErrorCode::UnknownField)
            }
        }
        fn update_field(
            &self,
            id: RecordId,
            field: FieldRef,
            value: ScanValue,
        ) -> Result<bool, ErrorCode> {
            match (field, &value) {
                (FIELD_A, ScanValue::U32(_)) => Ok(id == RecordId::from_u128(1)),
                (FIELD_A, _) => Err(ErrorCode::Malformed),
                _ => Err(ErrorCode::UnknownField),
            }
        }
        fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
            if id == RecordId::from_u128(1) {
                Ok(ParentLookup::Parent(RecordId::from_u128(100)))
            } else if id == RecordId::from_u128(2) {
                Ok(ParentLookup::NoParent)
            } else {
                Ok(ParentLookup::ChildNotFound)
            }
        }
        fn children(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Ok(vec![RecordId::from_u128(1)])
        }
        fn neighbors(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn neighbors_by_relation(
            &self,
            _id: RecordId,
            _relation: &str,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn list_relation_kinds(&self) -> Vec<String> {
            Vec::new()
        }
        fn validate_op(&self, _op: &TransactionOp) -> Result<(), ErrorCode> {
            Ok(())
        }
        fn describe(&self) -> DomainSchema {
            use protocol::{FieldCapabilities, FieldDescriptor, RelationCapabilities, ValueKind};
            DomainSchema {
                fields: vec![FieldDescriptor {
                    tag: FIELD_A,
                    name: "a".into(),
                    value_kind: ValueKind::U32,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: true,
                        update: true,
                    },
                }],
                relations: RelationCapabilities {
                    parent_children: true,
                    neighbors: false,
                },
            }
        }
        fn apply_transaction(
            &self,
            updates: &[TransactionOp],
            read_set: &[(RecordId, FieldRef, ScanValue)],
        ) -> Result<(), (usize, ErrorCode)> {
            // Same validate-then-apply shape a real adapter uses, against
            // this fixture's own single-record, non-mutating "store" —
            // exercises dispatch's Request::Transaction arm without
            // needing a real domain adapter.
            for (i, op) in updates.iter().enumerate() {
                match (op.field, &op.value) {
                    (FIELD_A, ScanValue::U32(_)) => {
                        if op.id != RecordId::from_u128(1) {
                            return Err((i, ErrorCode::RecordNotFound));
                        }
                    }
                    (FIELD_A, _) => return Err((i, ErrorCode::Malformed)),
                    _ => return Err((i, ErrorCode::UnknownField)),
                }
            }
            // `ISO-FR-002`/`ISO-FR-006`: re-check every tracked read
            // against this fixture's own fixed state (id 1, FIELD_A = 7)
            // the same way a real adapter re-checks against its store.
            for (id, field, value) in read_set {
                let current = if *id == RecordId::from_u128(1) && *field == FIELD_A {
                    Some(ScanValue::U32(7))
                } else {
                    None
                };
                if current.as_ref() != Some(value) {
                    return Err((0, ErrorCode::Conflict));
                }
            }
            Ok(())
        }
    }

    #[test]
    fn get_by_id_found_and_not_found() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::GetById {
                    id: RecordId::from_u128(1)
                }
            ),
            Response::Record {
                id: RecordId::from_u128(1),
                fields: vec![(FIELD_A, ScanValue::U32(7))],
            }
        );
        assert_eq!(
            dispatch(
                &store,
                Request::GetById {
                    id: RecordId::from_u128(99)
                }
            ),
            Response::NotFound
        );
    }

    #[test]
    fn update_field_maps_found_missing_and_malformed() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::UpdateField {
                    id: RecordId::from_u128(1),
                    field: FIELD_A,
                    value: ScanValue::U32(9)
                }
            ),
            Response::Ok
        );
        assert_eq!(
            dispatch(
                &store,
                Request::UpdateField {
                    id: RecordId::from_u128(99),
                    field: FIELD_A,
                    value: ScanValue::U32(9)
                }
            ),
            Response::NotFound
        );
        assert_eq!(
            dispatch(
                &store,
                Request::UpdateField {
                    id: RecordId::from_u128(1),
                    field: FIELD_A,
                    value: ScanValue::Bool(true)
                }
            ),
            err_response(ErrorCode::Malformed)
        );
    }

    #[test]
    fn parent_preserves_the_not_found_versus_no_parent_distinction() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Parent {
                    id: RecordId::from_u128(1)
                }
            ),
            Response::Id {
                id: RecordId::from_u128(100)
            }
        );
        assert_eq!(
            dispatch(
                &store,
                Request::Parent {
                    id: RecordId::from_u128(2)
                }
            ),
            Response::NoParent
        );
        assert_eq!(
            dispatch(
                &store,
                Request::Parent {
                    id: RecordId::from_u128(3)
                }
            ),
            Response::NotFound
        );
    }

    #[test]
    fn describe_schema_returns_the_fixture_store_own_shape() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(&store, Request::DescribeSchema),
            Response::Schema(store.describe())
        );
    }

    /// `SQL-FR-004` end to end through `dispatch`: a valid `Query`
    /// against `FixtureStore` answers `Response::Rows`; an unknown field
    /// answers the typed error, never a panic.
    #[test]
    fn dispatch_answers_query_with_rows_or_a_typed_error() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Query {
                    select: Selection::All,
                    filter: vec![Predicate {
                        field: FIELD_A,
                        op: CompareOp::Eq,
                        value: ScanValue::U32(7),
                    }],
                    limit: None,
                }
            ),
            Response::Rows {
                rows: vec![(RecordId::from_u128(1), vec![(FIELD_A, ScanValue::U32(7))])]
            }
        );
        assert_eq!(
            dispatch(
                &store,
                Request::Query {
                    select: Selection::Fields(vec![99]),
                    filter: vec![],
                    limit: None,
                }
            ),
            err_response(ErrorCode::UnknownField)
        );
    }

    #[test]
    fn unsupported_operation_reports_a_typed_error_not_a_panic() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Neighbors {
                    id: RecordId::from_u128(1)
                }
            ),
            err_response(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn transaction_all_pass_reports_ok() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Transaction {
                    updates: vec![
                        TransactionOp {
                            id: RecordId::from_u128(1),
                            field: FIELD_A,
                            value: ScanValue::U32(9),
                        },
                        TransactionOp {
                            id: RecordId::from_u128(1),
                            field: FIELD_A,
                            value: ScanValue::U32(10),
                        },
                    ]
                }
            ),
            Response::Ok
        );
    }

    #[test]
    fn transaction_reports_the_first_failing_operations_index_and_code() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Transaction {
                    updates: vec![
                        TransactionOp {
                            id: RecordId::from_u128(1),
                            field: FIELD_A,
                            value: ScanValue::U32(9),
                        },
                        TransactionOp {
                            id: RecordId::from_u128(99),
                            field: FIELD_A,
                            value: ScanValue::U32(9),
                        },
                        TransactionOp {
                            id: RecordId::from_u128(1),
                            field: FIELD_A,
                            value: ScanValue::U32(11),
                        },
                    ]
                }
            ),
            Response::TransactionFailed {
                index: 1,
                code: ErrorCode::RecordNotFound,
                message: error_message(ErrorCode::RecordNotFound).to_string(),
            }
        );
    }

    #[test]
    fn dispatch_never_routes_authenticate_to_a_store() {
        // handle_connection intercepts Authenticate before dispatch is ever
        // called with it — this only documents that dispatch itself stays
        // exhaustive and safe if that invariant were ever violated.
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Authenticate {
                    token: "irrelevant".into()
                }
            ),
            err_response(ErrorCode::Unsupported)
        );
    }

    /// `PROTO-FR-003`'s dispatch half (design criterion 5): like
    /// `Authenticate`, `Hello` is answered by `handle_connection`, never
    /// by a store.
    #[test]
    fn dispatch_never_routes_hello_to_a_store() {
        let store = FixtureStore;
        assert_eq!(
            dispatch(
                &store,
                Request::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            ),
            err_response(ErrorCode::Unsupported)
        );
    }

    /// `JRN-FR-008` / compatibility rule 3: `ErrorCode::Journal` is
    /// version 4, so a connection negotiated below 4 sees `Unsupported`
    /// in its place; nothing else is rewritten.
    #[test]
    fn journal_error_code_is_downgraded_below_version_4() {
        let failed = Response::TransactionFailed {
            index: 0,
            code: ErrorCode::Journal,
            message: error_message(ErrorCode::Journal).to_string(),
        };
        for older in [1, 2, 3] {
            assert_eq!(
                downgrade_for_version(failed.clone(), older),
                Response::TransactionFailed {
                    index: 0,
                    code: ErrorCode::Unsupported,
                    message: error_message(ErrorCode::Unsupported).to_string(),
                }
            );
        }
        assert_eq!(downgrade_for_version(failed.clone(), 4), failed);
        let untouched = err_response(ErrorCode::SessionFull);
        assert_eq!(downgrade_for_version(untouched.clone(), 1), untouched);
    }

    /// `ENT4-FR-003`/`ENT4-FR-006` (ADR-0041), acceptance criterion 5:
    /// the protocol's first content-rewriting downgrade. Below 11 every
    /// `StrList` pair is stripped from `Record` and from each `Rows` row
    /// and every `StrList` descriptor from `Schema`, other fields kept in
    /// order; at 11 all three are untouched; a `Record` with no `StrList`
    /// is untouched at 1 (every other domain's regression); the two
    /// `ErrorCode` cases are unchanged.
    #[test]
    fn str_list_content_is_stripped_below_version_11() {
        let id = uuid::Uuid::from_u128(1);
        let aliases = || (3u16, ScanValue::StrList(vec!["Ada".into()]));
        let label = || (0u16, ScanValue::Str("Ada Lovelace".into()));
        let count = || (2u16, ScanValue::I64(3));

        let record = Response::Record {
            id,
            fields: vec![label(), aliases(), count()],
        };
        let stripped_record = Response::Record {
            id,
            fields: vec![label(), count()],
        };
        let rows = Response::Rows {
            rows: vec![(id, vec![aliases(), label()]), (id, vec![aliases()])],
        };
        let stripped_rows = Response::Rows {
            rows: vec![(id, vec![label()]), (id, vec![])],
        };
        let descriptor = |name: &str, value_kind: protocol::ValueKind| protocol::FieldDescriptor {
            tag: 0,
            name: name.into(),
            value_kind,
            capabilities: protocol::FieldCapabilities {
                filter_eq: false,
                scan: false,
                update: false,
            },
        };
        let schema = Response::Schema(DomainSchema {
            fields: vec![
                descriptor("label", protocol::ValueKind::Str),
                descriptor("aliases", protocol::ValueKind::StrList),
            ],
            relations: protocol::RelationCapabilities {
                parent_children: false,
                neighbors: true,
            },
        });
        let stripped_schema = Response::Schema(DomainSchema {
            fields: vec![descriptor("label", protocol::ValueKind::Str)],
            relations: protocol::RelationCapabilities {
                parent_children: false,
                neighbors: true,
            },
        });

        for older in [1, 10] {
            assert_eq!(
                downgrade_for_version(record.clone(), older),
                stripped_record
            );
            assert_eq!(downgrade_for_version(rows.clone(), older), stripped_rows);
            assert_eq!(
                downgrade_for_version(schema.clone(), older),
                stripped_schema
            );
        }
        for current in [11, PROTOCOL_VERSION] {
            assert_eq!(downgrade_for_version(record.clone(), current), record);
            assert_eq!(downgrade_for_version(rows.clone(), current), rows);
            assert_eq!(downgrade_for_version(schema.clone(), current), schema);
        }
        // Every other domain: no `StrList` anywhere, nothing rewritten.
        assert_eq!(
            downgrade_for_version(stripped_record.clone(), 1),
            stripped_record
        );
        assert_eq!(
            downgrade_for_version(stripped_schema.clone(), 1),
            stripped_schema
        );
        // The two pre-existing cases hold, above and below their versions.
        let conflict = Response::TransactionFailed {
            index: 0,
            code: ErrorCode::Conflict,
            message: error_message(ErrorCode::Conflict).to_string(),
        };
        assert_eq!(downgrade_for_version(conflict.clone(), 11), conflict);
        assert_eq!(
            downgrade_for_version(conflict, 6),
            Response::TransactionFailed {
                index: 0,
                code: ErrorCode::Unsupported,
                message: error_message(ErrorCode::Unsupported).to_string(),
            }
        );
    }

    /// `SESS-FR-006`'s dispatch half: the session requests are
    /// per-connection, like `Authenticate` and `Hello`.
    #[test]
    fn dispatch_never_routes_session_requests_to_a_store() {
        let store = FixtureStore;
        for req in [
            Request::Begin,
            Request::Commit,
            Request::Rollback,
            Request::BeginWith { flags: 1 },
        ] {
            assert_eq!(dispatch(&store, req), err_response(ErrorCode::Unsupported));
        }
    }

    /// `RYW-FR-002`: the overlay is exact where it applies and inert
    /// everywhere else — last staged write per field wins; another id,
    /// an absent field, a kind mismatch, and a read-only field are each
    /// ignored.
    #[test]
    fn overlay_staged_replaces_only_matching_updatable_fields() {
        let id = RecordId::from_u128(1);
        let other = RecordId::from_u128(2);
        let staged = vec![
            TransactionOp {
                id,
                field: 1,
                value: ScanValue::U32(10),
            },
            TransactionOp {
                id: other,
                field: 1,
                value: ScanValue::U32(99),
            },
            TransactionOp {
                id,
                field: 1,
                value: ScanValue::U32(11),
            },
            TransactionOp {
                id,
                field: 2,
                value: ScanValue::U32(5),
            },
            TransactionOp {
                id,
                field: 0,
                value: ScanValue::Str("poodle".into()),
            },
            TransactionOp {
                id,
                field: 3,
                value: ScanValue::I64(7),
            },
        ];
        let mut fields = vec![
            (0, ScanValue::Str("labrador".into())),
            (1, ScanValue::U32(3)),
            (2, ScanValue::I64(4)),
        ];
        overlay_staged(id, &mut fields, &staged, &[1, 2]);
        assert_eq!(
            fields,
            vec![
                (0, ScanValue::Str("labrador".into())), // read-only: untouched
                (1, ScanValue::U32(11)),                // last staged write wins
                (2, ScanValue::I64(4)),                 // kind mismatch: untouched
            ]
        );
        overlay_staged(other, &mut fields, &staged, &[1, 2]);
        assert_eq!(fields[1], (1, ScanValue::U32(99)));
    }

    /// `ISO-FR-002`: a second read of the same `(id, field)` replaces the
    /// earlier entry — the read set always holds the most recently seen
    /// value, not a stale first-read snapshot.
    #[test]
    fn record_read_set_replaces_a_repeated_key_with_the_latest_value() {
        let id = RecordId::from_u128(1);
        let mut reads = HashMap::new();
        record_read_set(&mut reads, id, &[(1, ScanValue::U32(3))]);
        assert_eq!(reads.get(&(id, 1)), Some(&ScanValue::U32(3)));
        record_read_set(&mut reads, id, &[(1, ScanValue::U32(9))]);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads.get(&(id, 1)), Some(&ScanValue::U32(9)));
    }

    /// `ISO-FR-002`: distinct fields on the same id, and the same field
    /// across distinct ids, are independent keys.
    #[test]
    fn record_read_set_keys_by_both_id_and_field() {
        let id = RecordId::from_u128(1);
        let other = RecordId::from_u128(2);
        let mut reads = HashMap::new();
        record_read_set(
            &mut reads,
            id,
            &[
                (0, ScanValue::Str("labrador".into())),
                (1, ScanValue::U32(3)),
            ],
        );
        record_read_set(&mut reads, other, &[(1, ScanValue::U32(5))]);
        assert_eq!(reads.len(), 3);
        assert_eq!(
            reads.get(&(id, 0)),
            Some(&ScanValue::Str("labrador".into()))
        );
        assert_eq!(reads.get(&(id, 1)), Some(&ScanValue::U32(3)));
        assert_eq!(reads.get(&(other, 1)), Some(&ScanValue::U32(5)));
    }

    /// `ISO-FR-004`: past `MAX_TRACKED_READS` distinct keys, a *new* key
    /// is simply not added — the call never panics or truncates existing
    /// entries, and an already-tracked key keeps updating even once the
    /// map is at the cap.
    #[test]
    fn record_read_set_stops_adding_new_keys_past_the_cap_but_keeps_updating_old_ones() {
        let mut reads = HashMap::new();
        for i in 0..MAX_TRACKED_READS {
            record_read_set(
                &mut reads,
                RecordId::from_u128(i as u128),
                &[(0, ScanValue::U32(0))],
            );
        }
        assert_eq!(reads.len(), MAX_TRACKED_READS);

        // A new key past the cap is not added.
        record_read_set(
            &mut reads,
            RecordId::from_u128(MAX_TRACKED_READS as u128),
            &[(0, ScanValue::U32(0))],
        );
        assert_eq!(reads.len(), MAX_TRACKED_READS);
        assert!(!reads.contains_key(&(RecordId::from_u128(MAX_TRACKED_READS as u128), 0)));

        // An already-tracked key still updates at the cap.
        record_read_set(
            &mut reads,
            RecordId::from_u128(0),
            &[(0, ScanValue::U32(7))],
        );
        assert_eq!(reads.len(), MAX_TRACKED_READS);
        assert_eq!(
            reads.get(&(RecordId::from_u128(0), 0)),
            Some(&ScanValue::U32(7))
        );
    }

    /// `SQL-FR-007` (ADR-0034): a two-field schema — one numeric
    /// (`age`, `U32`), one string (`breed`, `Str`), the latter with
    /// every capability flag `false` — for `validate_query`/
    /// `evaluate_query` tests independent of any real domain adapter.
    fn sql_test_schema() -> DomainSchema {
        use protocol::{FieldCapabilities, FieldDescriptor, RelationCapabilities, ValueKind};
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: 1,
                    name: "age".into(),
                    value_kind: ValueKind::U32,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                FieldDescriptor {
                    tag: 2,
                    name: "breed".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: false,
                        update: false,
                    },
                },
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
            },
        }
    }

    /// `SQL-FR-007`: an unknown field in `select` or `filter` is
    /// `UnknownField`; an ordering comparator against `breed` (`Str`) is
    /// `Malformed`; a kind-mismatched literal against `age` (`U32`) is
    /// `Malformed`; a fully valid query is `Ok`, including one that
    /// selects/filters `breed` — every capability flag `false` there
    /// doesn't stop `Query` from reaching it (`SQL-FR-008`).
    #[test]
    fn validate_query_reports_unknown_fields_and_kind_mismatches() {
        let schema = sql_test_schema();
        assert_eq!(
            validate_query(&schema, &Selection::Fields(vec![99]), &[]),
            Err(ErrorCode::UnknownField)
        );
        assert_eq!(
            validate_query(
                &schema,
                &Selection::All,
                &[Predicate {
                    field: 99,
                    op: CompareOp::Eq,
                    value: ScanValue::U32(1)
                }]
            ),
            Err(ErrorCode::UnknownField)
        );
        assert_eq!(
            validate_query(
                &schema,
                &Selection::All,
                &[Predicate {
                    field: 2,
                    op: CompareOp::Gt,
                    value: ScanValue::Str("a".into())
                }]
            ),
            Err(ErrorCode::Malformed),
            "an ordering comparator against a Str field is Malformed"
        );
        assert_eq!(
            validate_query(
                &schema,
                &Selection::All,
                &[Predicate {
                    field: 1,
                    op: CompareOp::Eq,
                    value: ScanValue::Str("3".into())
                }]
            ),
            Err(ErrorCode::Malformed),
            "a Str literal against a U32 field is Malformed"
        );
        assert_eq!(
            validate_query(
                &schema,
                &Selection::Fields(vec![2]),
                &[Predicate {
                    field: 2,
                    op: CompareOp::Eq,
                    value: ScanValue::Str("labrador".into())
                }]
            ),
            Ok(()),
            "breed has every capability flag false but is still queryable"
        );
    }

    fn sql_test_rows() -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        vec![
            (
                RecordId::from_u128(1),
                vec![
                    (1, ScanValue::U32(3)),
                    (2, ScanValue::Str("labrador".into())),
                ],
            ),
            (
                RecordId::from_u128(2),
                vec![(1, ScanValue::U32(5)), (2, ScanValue::Str("poodle".into()))],
            ),
            (
                RecordId::from_u128(3),
                vec![
                    (1, ScanValue::U32(9)),
                    (2, ScanValue::Str("labrador".into())),
                ],
            ),
        ]
    }

    /// A three-field schema for the planner tests: `1` unindexed `U32`,
    /// `2` indexed `Str`, `3` indexed `U32`.
    fn planner_schema() -> DomainSchema {
        use protocol::{FieldCapabilities, FieldDescriptor, RelationCapabilities, ValueKind};
        let field = |tag: FieldRef, filter_eq: bool, value_kind: ValueKind| FieldDescriptor {
            tag,
            name: format!("f{tag}"),
            value_kind,
            capabilities: FieldCapabilities {
                filter_eq,
                scan: true,
                update: true,
            },
        };
        DomainSchema {
            fields: vec![
                field(1, false, ValueKind::U32),
                field(2, true, ValueKind::Str),
                field(3, true, ValueKind::U32),
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
            },
        }
    }

    fn eq(field: FieldRef, value: ScanValue) -> Predicate {
        Predicate {
            field,
            op: CompareOp::Eq,
            value,
        }
    }

    /// `QPL-FR-001` (ADR-0073), acceptance criterion 1: the plan is the
    /// first `Eq` on a `filter_eq: true` field in wire order, else a full
    /// scan — an empty filter, an `Eq` on an unindexed field, every
    /// non-`Eq` comparator on an indexed field, and a tag the schema
    /// lacks all plan `FullScan`; an ineligible predicate ahead of an
    /// eligible one yields the eligible one's own index.
    #[test]
    fn plan_query_picks_the_first_indexed_equality_or_a_full_scan() {
        let schema = planner_schema();
        let str_x = || ScanValue::Str("x".into());
        assert_eq!(plan_query(&schema, None, &[]), QueryPlan::FullScan);
        assert_eq!(
            plan_query(&schema, None, &[eq(2, str_x())]),
            QueryPlan::IndexEq(0)
        );
        assert_eq!(
            plan_query(&schema, None, &[eq(3, ScanValue::U32(1)), eq(2, str_x())]),
            QueryPlan::IndexEq(0),
            "two eligible predicates: the first in wire order"
        );
        assert_eq!(
            plan_query(&schema, None, &[eq(1, ScanValue::U32(1))]),
            QueryPlan::FullScan,
            "Eq on a filter_eq: false field"
        );
        for op in [
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            assert_eq!(
                plan_query(
                    &schema,
                    None,
                    &[Predicate {
                        field: 3,
                        op,
                        value: ScanValue::U32(1),
                    }]
                ),
                QueryPlan::FullScan,
                "{op:?} on an indexed field never uses the equality index"
            );
        }
        assert_eq!(
            plan_query(
                &schema,
                None,
                &[
                    Predicate {
                        field: 3,
                        op: CompareOp::Gt,
                        value: ScanValue::U32(1),
                    },
                    eq(2, str_x()),
                ]
            ),
            QueryPlan::IndexEq(1),
            "an ineligible predicate ahead of an eligible one"
        );
        assert_eq!(
            plan_query(&schema, None, &[eq(99, ScanValue::U32(1))]),
            QueryPlan::FullScan,
            "a tag the schema lacks (validate_query already rejected it)"
        );
    }

    /// `QPL-FR-002`–`004` (ADR-0073): a `ConnectionStore` whose equality
    /// index answers whatever the test says — exact, a superset, or a
    /// refusal — and which counts every `get` and `scan_all`, so a test
    /// asserts what the planner actually read rather than inferring it.
    /// Schema is [`planner_schema`]; rows are [`sql_test_rows`].
    type FixtureRow = (RecordId, Vec<(FieldRef, ScanValue)>);

    struct PlannerFixture {
        index: Result<Vec<RecordId>, ErrorCode>,
        /// `QKG-FR-004`: the fixture's rows when not `sql_test_rows()` —
        /// a table with repeated keys for the grouped walk.
        rows: Option<Vec<FixtureRow>>,
        /// `QPR-FR-002` (ADR-0075): the field the fixture's sorted index
        /// is over, if any, and whether `range_ids` refuses despite
        /// declaring it (the contract-mismatch fallback case).
        range_field: Option<FieldRef>,
        range_refuses: bool,
        /// `QPB-FR-003`: answer every budgeted walk "over budget" — the
        /// three-row fixture can never exceed a real budget on its own.
        range_over_budget: bool,
        gets: std::sync::atomic::AtomicUsize,
        scans: std::sync::atomic::AtomicUsize,
        limited_walks: std::sync::atomic::AtomicUsize,
        range_counts: std::sync::atomic::AtomicUsize,
        /// `QRF-FR-004`: the keys materialized (`range_keys`) and the
        /// folds (`range_stats`) apart, beside `range_counts`, which
        /// counts every walk-only call.
        key_walks: std::sync::atomic::AtomicUsize,
        stat_folds: std::sync::atomic::AtomicUsize,
    }

    impl PlannerFixture {
        fn with_index(index: Result<Vec<RecordId>, ErrorCode>) -> Self {
            Self {
                index,
                rows: None,
                range_field: None,
                range_refuses: false,
                range_over_budget: false,
                gets: Default::default(),
                scans: Default::default(),
                limited_walks: Default::default(),
                range_counts: Default::default(),
                key_walks: Default::default(),
                stat_folds: Default::default(),
            }
        }
        fn range_counts(&self) -> usize {
            self.range_counts.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn key_walks(&self) -> usize {
            self.key_walks.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn stat_folds(&self) -> usize {
            self.stat_folds.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn limited_walks(&self) -> usize {
            self.limited_walks
                .load(std::sync::atomic::Ordering::Relaxed)
        }
        /// An equality index that refuses and an exact sorted index over
        /// `field` (`Ordered`'s shape: every row's `(value, id)`, walked
        /// between the bounds).
        fn with_range(field: FieldRef) -> Self {
            Self {
                range_field: Some(field),
                ..Self::with_index(Err(ErrorCode::Unsupported))
            }
        }
        /// `with_range` over `rows` instead of `sql_test_rows()`.
        fn with_range_over(field: FieldRef, rows: Vec<FixtureRow>) -> Self {
            Self {
                rows: Some(rows),
                ..Self::with_range(field)
            }
        }
        fn rows(&self) -> Vec<FixtureRow> {
            self.rows.clone().unwrap_or_else(sql_test_rows)
        }
        /// Declares `field` range-indexed but refuses every walk.
        fn with_refusing_range(field: FieldRef) -> Self {
            Self {
                range_refuses: true,
                ..Self::with_range(field)
            }
        }
        fn gets(&self) -> usize {
            self.gets.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn scans(&self) -> usize {
            self.scans.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl ConnectionStore for PlannerFixture {
        fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.rows()
                .into_iter()
                .find(|(row_id, _)| *row_id == id)
                .map(|(_, fields)| fields)
        }
        fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
            self.scans
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.rows()
        }
        fn filter_eq(
            &self,
            _field: FieldRef,
            _value: &ScanValue,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            self.index.clone()
        }
        fn range_field(&self) -> Option<FieldRef> {
            self.range_field
        }
        /// The rows whose `U32` value of `field` lies within the bounds,
        /// ascending by `(value, id)` — what `Ordered::range_by` answers
        /// for a real domain.
        fn range_ids(
            &self,
            field: FieldRef,
            lower: Bound<ScanValue>,
            upper: Bound<ScanValue>,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            use std::ops::RangeBounds;
            if self.range_refuses || Some(field) != self.range_field {
                return Err(ErrorCode::Unsupported);
            }
            let key = |b: Bound<ScanValue>| match b {
                Bound::Unbounded => Bound::Unbounded,
                Bound::Included(ScanValue::U32(v)) => Bound::Included(v),
                Bound::Excluded(ScanValue::U32(v)) => Bound::Excluded(v),
                other => panic!("the fixture's range field is U32, got {other:?}"),
            };
            let range = (key(lower), key(upper));
            let mut keyed: Vec<(u32, RecordId)> = self
                .rows()
                .into_iter()
                .filter_map(
                    |(id, fields)| match fields.iter().find(|(f, _)| *f == field)?.1 {
                        ScanValue::U32(v) if range.contains(&v) => Some((v, id)),
                        _ => None,
                    },
                )
                .collect();
            keyed.sort();
            Ok(keyed.into_iter().map(|(_, id)| id).collect())
        }
        /// `QKW-FR-002`: the walked keys (field 1 is `U32`), as `i64`;
        /// counted with `range_counts`, and the `get`s it borrows to read
        /// the fixture's key are given back so a test can still assert
        /// "no record decoded".
        fn range_keys(
            &self,
            field: FieldRef,
            lower: Bound<ScanValue>,
            upper: Bound<ScanValue>,
        ) -> Result<Vec<i64>, ErrorCode> {
            self.range_counts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.key_walks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let ids = self.range_ids(field, lower, upper)?;
            Ok(ids
                .into_iter()
                .filter_map(|id| {
                    let fields = self.get(id)?;
                    self.gets.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    match fields.iter().find(|(f, _)| *f == field)?.1 {
                        ScanValue::U32(v) => Some(i64::from(v)),
                        _ => None,
                    }
                })
                .collect())
        }
        /// `QRF-FR-004`: the fold — over the same keys `range_keys` would
        /// give, counted apart so a test can assert which one ran.
        fn range_stats(
            &self,
            field: FieldRef,
            lower: Bound<ScanValue>,
            upper: Bound<ScanValue>,
        ) -> Result<KeyStats, ErrorCode> {
            let keys = self.range_keys(field, lower, upper)?;
            self.key_walks
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            self.stat_folds
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(keys.iter().fold(KeyStats::default(), KeyStats::with))
        }
        /// `QCW-FR-002`: the walk's length; counted, so a test can assert
        /// the count came from the index and not from decoded rows.
        fn range_count(
            &self,
            field: FieldRef,
            lower: Bound<ScanValue>,
            upper: Bound<ScanValue>,
        ) -> Result<u64, ErrorCode> {
            self.range_counts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(self.range_ids(field, lower, upper)?.len() as u64)
        }
        /// `QPB-FR-002`: the same walk, `None` past `limit` ids; counted,
        /// so a test can assert the budget was (or was not) consulted.
        fn range_ids_limited(
            &self,
            field: FieldRef,
            lower: Bound<ScanValue>,
            upper: Bound<ScanValue>,
            limit: usize,
        ) -> Result<Option<Vec<RecordId>>, ErrorCode> {
            self.limited_walks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let ids = self.range_ids(field, lower, upper)?;
            Ok((!self.range_over_budget && ids.len() <= limit).then_some(ids))
        }
        fn scan_field(&self, _field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn update_field(
            &self,
            _id: RecordId,
            _field: FieldRef,
            _value: ScanValue,
        ) -> Result<bool, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn parent(&self, _id: RecordId) -> Result<ParentLookup, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn children(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        /// A fixed symmetric edge 1 — 3 (both labradors), so a `Join`
        /// over `Neighbors(None)` has pairs to find in either orientation.
        fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Ok(match id {
                id if id == RecordId::from_u128(1) => vec![RecordId::from_u128(3)],
                id if id == RecordId::from_u128(3) => vec![RecordId::from_u128(1)],
                _ => vec![],
            })
        }
        fn neighbors_by_relation(
            &self,
            _id: RecordId,
            _relation: &str,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn list_relation_kinds(&self) -> Vec<String> {
            Vec::new()
        }
        fn validate_op(&self, _op: &TransactionOp) -> Result<(), ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn describe(&self) -> DomainSchema {
            planner_schema()
        }
        fn apply_transaction(
            &self,
            _updates: &[TransactionOp],
            _read_set: &[(RecordId, FieldRef, ScanValue)],
        ) -> Result<(), (usize, ErrorCode)> {
            Err((0, ErrorCode::Unsupported))
        }
    }

    fn labrador() -> Predicate {
        eq(2, ScanValue::Str("labrador".into()))
    }

    fn ids_of(rows: &[(RecordId, Vec<(FieldRef, ScanValue)>)]) -> Vec<RecordId> {
        let mut ids: Vec<_> = rows.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids
    }

    /// `QPL-FR-002`, acceptance criterion 2: the index path reads exactly
    /// the bucket's ids through `get` and never touches `scan_all`; an id
    /// whose record is gone is dropped silently, as `scan_all`'s own
    /// per-id `get` already drops it.
    #[test]
    fn query_candidates_index_path_reads_only_the_bucket_and_drops_a_vanished_id() {
        let store = PlannerFixture::with_index(Ok(vec![
            RecordId::from_u128(1),
            RecordId::from_u128(42),
            RecordId::from_u128(3),
        ]));
        let filter = [labrador()];
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(plan, QueryPlan::IndexEq(0));
        let candidates = query_candidates(&store, plan, &filter);
        assert_eq!(
            store.gets(),
            3,
            "one get per index id, including the vanished one"
        );
        assert_eq!(store.scans(), 0, "the index path never scans");
        assert_eq!(
            ids_of(&candidates),
            vec![RecordId::from_u128(1), RecordId::from_u128(3)],
            "id 42 has no record and is dropped, not an error"
        );
    }

    /// `QPL-FR-003`/`QPL-FR-005`, acceptance criterion 2: an index that
    /// returns a *superset* of the exact matches (`Entity::label`'s real
    /// shape) is corrected by `evaluate_query`'s re-check — the final rows
    /// equal the full scan's exactly.
    #[test]
    fn query_candidates_superset_index_is_corrected_by_the_re_check() {
        let store = PlannerFixture::with_index(Ok(vec![
            RecordId::from_u128(1),
            RecordId::from_u128(2),
            RecordId::from_u128(3),
        ]));
        let filter = [labrador()];
        let via_index = evaluate_query(
            query_candidates(&store, QueryPlan::IndexEq(0), &filter),
            &Selection::All,
            &filter,
            None,
        );
        let via_scan = evaluate_query(
            query_candidates(&store, QueryPlan::FullScan, &filter),
            &Selection::All,
            &filter,
            None,
        );
        assert_eq!(
            ids_of(&via_index),
            vec![RecordId::from_u128(1), RecordId::from_u128(3)]
        );
        assert_eq!(ids_of(&via_index), ids_of(&via_scan));
    }

    /// `QPL-FR-004`, acceptance criterion 2: a `filter_eq` refusal on a
    /// field the schema claimed indexed falls back to one full scan with
    /// the identical result — and, through `dispatch`, a `Response::Rows`,
    /// never an error.
    #[test]
    fn query_candidates_index_refusal_falls_back_to_a_full_scan_without_an_error() {
        let store = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let filter = [labrador()];
        let candidates = query_candidates(&store, QueryPlan::IndexEq(0), &filter);
        assert_eq!(store.scans(), 1);
        assert_eq!(store.gets(), 0);
        assert_eq!(ids_of(&candidates), ids_of(&sql_test_rows()));

        match dispatch(
            &store,
            Request::Query {
                select: Selection::All,
                filter: vec![labrador()],
                limit: None,
            },
        ) {
            Response::Rows { rows } => assert_eq!(
                ids_of(&rows),
                vec![RecordId::from_u128(1), RecordId::from_u128(3)]
            ),
            other => panic!("expected Rows, got {other:?}"),
        }
    }

    /// `QPL-FR-005`/`QPL-FR-006`: end to end through `dispatch`, an
    /// indexed `Eq` and the same filter with the index unavailable return
    /// the same row set, projected the same way; `limit` still truncates.
    #[test]
    fn dispatch_query_returns_the_same_rows_on_either_plan() {
        let indexed =
            PlannerFixture::with_index(Ok(vec![RecordId::from_u128(3), RecordId::from_u128(1)]));
        let unindexed = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let request = || Request::Query {
            select: Selection::Fields(vec![1]),
            filter: vec![labrador()],
            limit: None,
        };
        let rows_of = |response| match response {
            Response::Rows { mut rows } => {
                rows.sort_by_key(|(id, _)| *id);
                rows
            }
            other => panic!("expected Rows, got {other:?}"),
        };
        let a = rows_of(dispatch(&indexed, request()));
        let b = rows_of(dispatch(&unindexed, request()));
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].1, vec![(1, ScanValue::U32(3))], "projection applied");
        assert_eq!(indexed.scans(), 0);
        assert_eq!(unindexed.scans(), 1);

        match dispatch(
            &indexed,
            Request::Query {
                select: Selection::All,
                filter: vec![labrador()],
                limit: Some(1),
            },
        ) {
            Response::Rows { rows } => assert_eq!(rows.len(), 1),
            other => panic!("expected Rows, got {other:?}"),
        }
    }

    /// `QPC-FR-002` (ADR-0074), acceptance criterion 1: `Aggregate`
    /// through `dispatch` with an indexed `Eq` reads only the bucket and
    /// returns the same groups (compared as a set) as with the index
    /// refusing; a superset bucket is corrected by `evaluate_aggregate`'s
    /// own re-filter.
    #[test]
    fn dispatch_aggregate_returns_the_same_groups_on_either_plan() {
        let request = || Request::Aggregate {
            group_by: vec![2],
            filter: vec![labrador()],
            aggregates: vec![
                AggregateSpec {
                    func: AggregateFn::Count,
                    field: None,
                },
                AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(1),
                },
            ],
            limit: None,
        };
        let groups_of = |response| match response {
            Response::Groups { mut groups } => {
                groups.sort_by(|a, b| format!("{:?}", a.key).cmp(&format!("{:?}", b.key)));
                groups
            }
            other => panic!("expected Groups, got {other:?}"),
        };
        // Exact bucket.
        let indexed =
            PlannerFixture::with_index(Ok(vec![RecordId::from_u128(3), RecordId::from_u128(1)]));
        // Superset bucket (the `Entity::label` shape): the poodle is in the
        // bucket but must not be counted.
        let superset = PlannerFixture::with_index(Ok(vec![
            RecordId::from_u128(1),
            RecordId::from_u128(2),
            RecordId::from_u128(3),
        ]));
        let refusing = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let a = groups_of(dispatch(&indexed, request()));
        let b = groups_of(dispatch(&superset, request()));
        let c = groups_of(dispatch(&refusing, request()));
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a.len(), 1, "one group: labrador");
        assert_eq!(
            a[0].values[0],
            ScanValue::I64(2),
            "COUNT(*) of the two labradors"
        );
        assert_eq!(a[0].values[1], ScanValue::I64(12), "SUM(field 1) = 3 + 9");
        assert_eq!(indexed.gets(), 2);
        assert_eq!(indexed.scans(), 0, "the index path never scans");
        assert_eq!(superset.gets(), 3);
        assert_eq!(refusing.scans(), 1);
        assert_eq!(refusing.gets(), 0);
    }

    /// `QPC-FR-003` (ADR-0074), acceptance criterion 1: the
    /// `filtered_page` default reads only the bucket and returns the
    /// **identical sequence** either way — `page_rows` orders the
    /// filtered set by `(key, id)`, so the plan is invisible; a cursor
    /// works the same on both.
    #[test]
    fn filtered_page_default_returns_the_identical_sequence_on_either_plan() {
        let indexed =
            PlannerFixture::with_index(Ok(vec![RecordId::from_u128(3), RecordId::from_u128(1)]));
        let refusing = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let filter = [labrador()];
        let a = indexed.filtered_page(1, None, 10, &filter).unwrap();
        let b = refusing.filtered_page(1, None, 10, &filter).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            ids_of(&a),
            vec![RecordId::from_u128(1), RecordId::from_u128(3)]
        );
        assert_eq!(a[0].0, RecordId::from_u128(1), "field 1 = 3 sorts before 9");
        assert_eq!(indexed.scans(), 0);
        assert_eq!(indexed.gets(), 2);
        assert_eq!(refusing.scans(), 1);

        // A cursored second page: strictly after (3, id 1) → only id 3.
        let after = Some((ScanValue::U32(3), RecordId::from_u128(1)));
        let a2 = indexed
            .filtered_page(1, after.clone(), 10, &filter)
            .unwrap();
        let b2 = refusing.filtered_page(1, after, 10, &filter).unwrap();
        assert_eq!(a2, b2);
        assert_eq!(ids_of(&a2), vec![RecordId::from_u128(3)]);
    }

    /// `QPC-FR-004` (ADR-0074), acceptance criterion 1: `evaluate_join`
    /// narrows the *left* side through the index and returns the same
    /// pair set either way; the right side is fetched by id regardless.
    #[test]
    fn evaluate_join_returns_the_same_pairs_on_either_plan() {
        let spec = JoinSpec {
            relation: JoinRelation::Neighbors(None),
            right_table: None,
            left: Selection::Fields(vec![2]),
            right: Selection::Fields(vec![1]),
            left_filter: vec![labrador()],
            right_filter: vec![],
            limit: None,
        };
        let pair_ids = |rows: &[JoinedRow]| {
            let mut ids: Vec<(RecordId, RecordId)> =
                rows.iter().map(|r| (r.left_id, r.right_id)).collect();
            ids.sort();
            ids
        };
        let indexed =
            PlannerFixture::with_index(Ok(vec![RecordId::from_u128(1), RecordId::from_u128(3)]));
        let superset = PlannerFixture::with_index(Ok(vec![
            RecordId::from_u128(1),
            RecordId::from_u128(2),
            RecordId::from_u128(3),
        ]));
        let refusing = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let a = evaluate_join(&indexed, &indexed, &spec);
        let b = evaluate_join(&superset, &superset, &spec);
        let c = evaluate_join(&refusing, &refusing, &spec);
        assert_eq!(pair_ids(&a), pair_ids(&b));
        assert_eq!(pair_ids(&a), pair_ids(&c));
        assert_eq!(
            pair_ids(&a),
            vec![
                (RecordId::from_u128(1), RecordId::from_u128(3)),
                (RecordId::from_u128(3), RecordId::from_u128(1)),
            ],
            "the symmetric 1 — 3 edge in both orientations; the poodle (2) has no edge"
        );
        assert_eq!(
            indexed.scans(),
            0,
            "the left side never scans on the index path"
        );
        assert_eq!(refusing.scans(), 1);
        // Left rows are projected to field 2, right rows to field 1.
        assert_eq!(a[0].left, vec![(2, ScanValue::Str("labrador".into()))]);
        assert_eq!(a[0].right.len(), 1);
        assert_eq!(a[0].right[0].0, 1);
    }

    fn u32_pred(field: FieldRef, op: CompareOp, value: u32) -> Predicate {
        Predicate {
            field,
            op,
            value: ScanValue::U32(value),
        }
    }

    /// `QPR-FR-003` (ADR-0075), acceptance criterion 2: with no range
    /// field every ordering predicate still plans a full scan; with one,
    /// each comparator supplies its side, an `Eq` both, `Ne` neither, a
    /// second lower bound is ignored (no tightening), an eligible
    /// equality wins over any range (compatibility), and an ineligible
    /// predicate ahead of the bound leaves the bound's own position.
    #[test]
    fn plan_query_walks_a_range_only_after_the_equality_rule() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let schema = planner_schema();
        let range = |lower, upper| QueryPlan::IndexRange { lower, upper };
        for op in [Lt, Le, Gt, Ge] {
            assert_eq!(
                plan_query(&schema, None, &[u32_pred(1, op, 1)]),
                QueryPlan::FullScan,
                "{op:?} with no range field stays a full scan"
            );
        }
        let f = Some(1);
        assert_eq!(plan_query(&schema, f, &[]), QueryPlan::FullScan);
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Gt, 1)]),
            range(Some(0), None)
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Ge, 1)]),
            range(Some(0), None)
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Lt, 1)]),
            range(None, Some(0))
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Le, 1)]),
            range(None, Some(0))
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Ge, 1), u32_pred(1, Le, 9)]),
            range(Some(0), Some(1))
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Eq, 5)]),
            range(Some(0), Some(0)),
            "an Eq on the (not filter_eq) range field is a one-key walk"
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Ne, 5)]),
            QueryPlan::FullScan
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(3, Gt, 1)]),
            QueryPlan::FullScan,
            "an ordering predicate on a field that is not the range field"
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Gt, 1), u32_pred(1, Gt, 5)]),
            range(Some(1), None),
            "two lower bounds: the tightest (ADR-0083)"
        );
        assert_eq!(
            plan_query(
                &schema,
                f,
                &[u32_pred(1, Gt, 1), eq(2, ScanValue::Str("x".into()))]
            ),
            QueryPlan::IndexIntersect {
                eq: 1,
                lower: Some(0),
                upper: None
            },
            "an eligible equality anywhere in the filter, with a range: both (ADR-0078)"
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(3, Ne, 1), u32_pred(1, Lt, 9)]),
            range(None, Some(1)),
            "an ineligible predicate ahead of the bound"
        );
    }

    /// `QPI-FR-001` (ADR-0078), acceptance criterion 1: an eligible
    /// equality beside a bound on the range field plans the
    /// intersection — one side, both sides, `Eq` on the range field as
    /// both — in either wire order; the equality alone (no range field,
    /// no bound, an ordering on a field that is not the range field)
    /// still plans `IndexEq`, and a bound alone still `IndexRange`.
    #[test]
    fn plan_query_intersects_an_indexed_equality_with_a_range() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt};
        let schema = planner_schema();
        let x = || eq(2, ScanValue::Str("x".into()));
        let f = Some(1);
        let both = |eq, lower, upper| QueryPlan::IndexIntersect { eq, lower, upper };
        assert_eq!(
            plan_query(&schema, f, &[x(), u32_pred(1, Gt, 1)]),
            both(0, Some(1), None)
        );
        assert_eq!(
            plan_query(&schema, f, &[x(), u32_pred(1, Le, 9)]),
            both(0, None, Some(1))
        );
        assert_eq!(
            plan_query(&schema, f, &[u32_pred(1, Ge, 1), x(), u32_pred(1, Lt, 9)]),
            both(1, Some(0), Some(2)),
            "the equality's position and each bound's, in wire order"
        );
        assert_eq!(
            plan_query(&schema, f, &[x(), u32_pred(1, Eq, 5)]),
            both(0, Some(1), Some(1)),
            "Eq on the range field supplies both sides"
        );
        assert_eq!(
            plan_query(
                &schema,
                f,
                &[eq(3, ScanValue::U32(7)), x(), u32_pred(1, Gt, 1)]
            ),
            both(0, Some(2), None),
            "two eligible equalities: the first in wire order"
        );
        assert_eq!(
            plan_query(&schema, None, &[x(), u32_pred(1, Gt, 1)]),
            QueryPlan::IndexEq(0),
            "no range field: the bucket alone"
        );
        assert_eq!(
            plan_query(&schema, f, &[x(), u32_pred(3, Gt, 1)]),
            QueryPlan::IndexEq(0),
            "an ordering on a field that is not the range field"
        );
        assert_eq!(
            plan_query(&schema, f, &[x(), u32_pred(1, CompareOp::Ne, 1)]),
            QueryPlan::IndexEq(0),
            "Ne on the range field is not a bound"
        );
        assert_eq!(
            plan_query(&schema, f, &[eq(1, ScanValue::U32(5))]),
            QueryPlan::IndexRange {
                lower: Some(0),
                upper: Some(0)
            },
            "Eq on the (not filter_eq) range field alone is still the one-key walk"
        );
    }

    /// `QPI-FR-002` (ADR-0078): the ids in both lists, in the larger
    /// list's order; either side empty is empty; duplicates in the
    /// larger list survive as the larger list carries them.
    #[test]
    fn intersect_ids_keeps_the_larger_list_s_order() {
        let id = RecordId::from_u128;
        assert_eq!(
            intersect_ids(vec![id(3), id(1)], vec![id(1), id(2), id(3), id(4)]),
            vec![id(1), id(3)]
        );
        assert_eq!(
            intersect_ids(vec![id(4), id(3), id(2), id(1)], vec![id(3), id(9)]),
            vec![id(3)],
            "the smaller list is hashed whichever side it is on"
        );
        assert_eq!(intersect_ids(vec![], vec![id(1)]), Vec::<RecordId>::new());
        assert_eq!(intersect_ids(vec![id(1)], vec![]), Vec::<RecordId>::new());
        assert_eq!(
            intersect_ids(vec![id(1)], vec![id(1), id(1)]),
            vec![id(1), id(1)]
        );
    }

    /// `QPI-FR-002`/`003` (ADR-0078), acceptance criterion 2: the
    /// intersection reads only the ids in both the bucket and the walk —
    /// one `get` each, no scan; a walk refusal reads the bucket alone
    /// (what `IndexEq` read before this round); a bucket refusal scans,
    /// as `IndexEq`'s always did; and `dispatch` answers `Rows` on every
    /// path with the identical set.
    #[test]
    fn query_candidates_intersection_reads_only_the_ids_in_both() {
        use CompareOp::Gt;
        let id = RecordId::from_u128;
        let labradors = || Ok(vec![id(1), id(3)]);
        let filter = [labrador(), u32_pred(1, Gt, 3)];
        let expect_plan = QueryPlan::IndexIntersect {
            eq: 0,
            lower: Some(1),
            upper: None,
        };

        let store = PlannerFixture {
            range_field: Some(1),
            ..PlannerFixture::with_index(labradors())
        };
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(plan, expect_plan);
        let candidates = query_candidates(&store, plan, &filter);
        assert_eq!(
            ids_of(&candidates),
            vec![id(3)],
            "labradors are 1 and 3; field 1 > 3 admits 2 and 3; both: 3"
        );
        assert_eq!(
            (store.gets(), store.scans()),
            (1, 0),
            "one get, for the one id in both"
        );

        let store = PlannerFixture {
            range_refuses: true,
            range_field: Some(1),
            ..PlannerFixture::with_index(labradors())
        };
        let candidates = query_candidates(&store, expect_plan, &filter);
        assert_eq!(ids_of(&candidates), vec![id(1), id(3)], "the bucket alone");
        assert_eq!((store.gets(), store.scans()), (2, 0));

        let store = PlannerFixture {
            range_field: Some(1),
            ..PlannerFixture::with_index(Err(ErrorCode::Unsupported))
        };
        // Its own plan is `IndexRange` (nothing declares field 2 refusing);
        // hand it the intersection plan to exercise the bucket refusal.
        let candidates = query_candidates(&store, expect_plan, &filter);
        assert_eq!(ids_of(&candidates), ids_of(&sql_test_rows()));
        assert_eq!((store.gets(), store.scans()), (0, 1));

        for store in [
            PlannerFixture {
                range_field: Some(1),
                ..PlannerFixture::with_index(labradors())
            },
            PlannerFixture {
                range_refuses: true,
                range_field: Some(1),
                ..PlannerFixture::with_index(labradors())
            },
            PlannerFixture::with_index(Err(ErrorCode::Unsupported)),
        ] {
            match dispatch(
                &store,
                Request::Query {
                    select: Selection::All,
                    filter: filter.to_vec(),
                    limit: None,
                },
            ) {
                Response::Rows { rows } => assert_eq!(ids_of(&rows), vec![id(3)]),
                other => panic!("expected Rows, got {other:?}"),
            }
        }
    }

    /// `QPB-FR-003` (ADR-0079), acceptance criterion 2: the intersection
    /// walks with a budget of `INTERSECT_WALK_BUDGET` ids per bucket id
    /// — within it, the intersection; past it, the bucket alone with no
    /// id materialized; an empty bucket walks nothing at all; and
    /// `dispatch` answers the identical rows either way.
    #[test]
    fn query_candidates_intersection_yields_to_the_bucket_past_the_walk_budget() {
        use CompareOp::Gt;
        let id = RecordId::from_u128;
        let filter = [labrador(), u32_pred(1, Gt, 3)];
        let plan = QueryPlan::IndexIntersect {
            eq: 0,
            lower: Some(1),
            upper: None,
        };

        // Within budget: two bucket ids allow twenty walked; three exist.
        let store = PlannerFixture {
            range_field: Some(1),
            ..PlannerFixture::with_index(Ok(vec![id(1), id(3)]))
        };
        assert_eq!(
            ids_of(&query_candidates(&store, plan, &filter)),
            vec![id(3)]
        );
        assert_eq!((store.gets(), store.limited_walks()), (1, 1));

        // Past it: the bucket alone, every bucket id read.
        let store = PlannerFixture {
            range_field: Some(1),
            range_over_budget: true,
            ..PlannerFixture::with_index(Ok(vec![id(1), id(3)]))
        };
        assert_eq!(
            ids_of(&query_candidates(&store, plan, &filter)),
            vec![id(1), id(3)]
        );
        assert_eq!(
            (store.gets(), store.scans(), store.limited_walks()),
            (2, 0, 1)
        );

        // An empty bucket: nothing walked, nothing read.
        let store = PlannerFixture {
            range_field: Some(1),
            ..PlannerFixture::with_index(Ok(vec![]))
        };
        assert!(query_candidates(&store, plan, &filter).is_empty());
        assert_eq!(
            (store.gets(), store.scans(), store.limited_walks()),
            (0, 0, 0)
        );

        for over_budget in [false, true] {
            let store = PlannerFixture {
                range_field: Some(1),
                range_over_budget: over_budget,
                ..PlannerFixture::with_index(Ok(vec![id(1), id(3)]))
            };
            match dispatch(
                &store,
                Request::Query {
                    select: Selection::All,
                    filter: filter.to_vec(),
                    limit: None,
                },
            ) {
                Response::Rows { rows } => assert_eq!(ids_of(&rows), vec![id(3)]),
                other => panic!("expected Rows, got {other:?}"),
            }
        }
    }

    /// `QCW-FR-003` (ADR-0081), acceptance criterion 1: eligible for an
    /// empty filter, one bound, two bounds one per side, and `Eq`; not
    /// for a `group_by`, a non-`Count` aggregate, a `COUNT(field)`, no
    /// aggregate, no range field, a predicate on another field, `Ne`,
    /// or two bounds on one side.
    #[test]
    fn counted_walk_applies_only_to_a_pure_range_count() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let count = || AggregateSpec {
            func: AggregateFn::Count,
            field: None,
        };
        let applies = |group_by: &[FieldRef], filter: &[Predicate], aggs: &[AggregateSpec]| {
            counted_walk_applies(Some(1), group_by, filter, aggs)
        };
        assert!(applies(&[], &[], &[count()]));
        assert!(applies(&[], &[u32_pred(1, Ge, 1)], &[count()]));
        assert!(applies(
            &[],
            &[u32_pred(1, Ge, 1), u32_pred(1, Lt, 9)],
            &[count()]
        ));
        assert!(applies(&[], &[u32_pred(1, Eq, 5)], &[count()]));
        assert!(
            applies(&[], &[u32_pred(1, Gt, 1)], &[count(), count()]),
            "two COUNT(*)"
        );
        assert!(
            !applies(&[2], &[u32_pred(1, Ge, 1)], &[count()]),
            "group_by"
        );
        assert!(
            !applies(
                &[],
                &[u32_pred(1, Ge, 1)],
                &[AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(1)
                }]
            ),
            "SUM"
        );
        assert!(
            !applies(
                &[],
                &[u32_pred(1, Ge, 1)],
                &[
                    count(),
                    AggregateSpec {
                        func: AggregateFn::Max,
                        field: Some(1)
                    }
                ]
            ),
            "COUNT(*) beside MAX"
        );
        assert!(!applies(&[], &[u32_pred(1, Ge, 1)], &[]), "no aggregate");
        assert!(
            !counted_walk_applies(None, &[], &[u32_pred(1, Ge, 1)], &[count()]),
            "no range field"
        );
        assert!(
            !applies(&[], &[u32_pred(1, Ge, 1), u32_pred(3, Gt, 0)], &[count()]),
            "another field"
        );
        assert!(!applies(&[], &[u32_pred(1, Ne, 1)], &[count()]), "Ne");
        assert!(
            applies(&[], &[u32_pred(1, Ge, 1), u32_pred(1, Gt, 5)], &[count()]),
            "two lower bounds: the tightest implies the other (ADR-0083)"
        );
        assert!(
            applies(&[], &[u32_pred(1, Eq, 5), u32_pred(1, Le, 9)], &[count()]),
            "Eq beside an upper bound (ADR-0083)"
        );
    }

    /// `QBT-FR-001` (ADR-0083), acceptance criterion 1: the tightest
    /// lower bound is the greatest literal, `Gt` over `Ge` at a tie; the
    /// tightest upper the least, `Lt` over `Le` at a tie; `Eq` on either
    /// side; another field ignored; `None` without a range field.
    #[test]
    fn tightest_bounds_picks_the_greatest_lower_and_least_upper() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let t = |filter: &[Predicate]| tightest_bounds(Some(1), filter);
        assert_eq!(t(&[]), (None, None));
        assert_eq!(
            t(&[u32_pred(1, Ge, 1), u32_pred(1, Gt, 5)]),
            (Some(1), None)
        );
        assert_eq!(
            t(&[u32_pred(1, Gt, 5), u32_pred(1, Ge, 1)]),
            (Some(0), None)
        );
        assert_eq!(
            t(&[u32_pred(1, Ge, 5), u32_pred(1, Gt, 5)]),
            (Some(1), None),
            "Gt beats Ge at the same literal"
        );
        assert_eq!(
            t(&[u32_pred(1, Le, 9), u32_pred(1, Lt, 9), u32_pred(1, Lt, 12)]),
            (None, Some(1)),
            "Lt beats Le at the same literal; the least wins"
        );
        assert_eq!(
            t(&[u32_pred(1, Ge, 1), u32_pred(1, Eq, 4), u32_pred(1, Le, 9)]),
            (Some(1), Some(1)),
            "Eq is the tightest of both sides here"
        );
        assert_eq!(
            t(&[u32_pred(1, Eq, 4), u32_pred(1, Gt, 6)]),
            (Some(1), Some(0)),
            "a lower bound past the Eq: an empty range, honestly"
        );
        assert_eq!(
            t(&[u32_pred(3, Gt, 99), u32_pred(1, Ne, 2), u32_pred(1, Lt, 3)]),
            (None, Some(2)),
            "another field and Ne ignored"
        );
        assert_eq!(tightest_bounds(None, &[u32_pred(1, Ge, 1)]), (None, None));
    }

    /// `QBT-FR-003` (ADR-0083), acceptance criterion 2: a redundant looser
    /// bound ahead of the tight one changes what is read, not what is
    /// returned — `query_candidates` walks the tight range only (one
    /// `get` per admitted id), `dispatch` answers the scan's rows, and a
    /// pure-range count with two bounds per side comes from the index.
    #[test]
    fn a_looser_bound_beside_a_tighter_one_walks_the_tight_range_only() {
        use CompareOp::{Ge, Gt, Le, Lt};
        let id = RecordId::from_u128;
        // field 1 is 3, 5, 9: `>= 1 AND > 3 AND <= 9 AND < 9` admits 5 only.
        let filter = [
            u32_pred(1, Ge, 1),
            u32_pred(1, Gt, 3),
            u32_pred(1, Le, 9),
            u32_pred(1, Lt, 9),
        ];
        let store = PlannerFixture::with_range(1);
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(
            plan,
            QueryPlan::IndexRange {
                lower: Some(1),
                upper: Some(3)
            }
        );
        assert_eq!(
            ids_of(&query_candidates(&store, plan, &filter)),
            vec![id(2)]
        );
        assert_eq!(
            (store.gets(), store.scans()),
            (1, 0),
            "only the tight range read"
        );
        let scanned = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        let rows_of = |response| match response {
            Response::Rows { mut rows } => {
                rows.sort_by_key(|(id, _)| *id);
                rows
            }
            other => panic!("expected Rows, got {other:?}"),
        };
        let request = || Request::Query {
            select: Selection::All,
            filter: filter.to_vec(),
            limit: None,
        };
        assert_eq!(
            rows_of(dispatch(&store, request())),
            rows_of(dispatch(&scanned, request()))
        );
        let counted = PlannerFixture::with_range(1);
        match dispatch(
            &counted,
            Request::Aggregate {
                group_by: vec![],
                filter: filter.to_vec(),
                aggregates: vec![AggregateSpec {
                    func: AggregateFn::Count,
                    field: None,
                }],
                limit: None,
            },
        ) {
            Response::Groups { groups } => assert_eq!(groups[0].values, vec![ScanValue::I64(1)]),
            other => panic!("expected Groups, got {other:?}"),
        }
        assert_eq!(
            (counted.gets(), counted.scans(), counted.range_counts()),
            (0, 0, 1)
        );
    }

    /// `QCW-FR-004` (ADR-0081), acceptance criterion 2: the eligible
    /// count comes from the index — no `get`, no scan — and `dispatch`
    /// answers exactly what the decode path answers, `limit` included;
    /// an ineligible shape still decodes.
    #[test]
    fn dispatch_counts_a_pure_range_from_the_index_without_reading_a_record() {
        use CompareOp::{Ge, Gt, Lt};
        let count = || AggregateSpec {
            func: AggregateFn::Count,
            field: None,
        };
        let groups_of = |response| match response {
            Response::Groups { groups } => groups,
            other => panic!("expected Groups, got {other:?}"),
        };
        let request = |filter: Vec<Predicate>, limit| Request::Aggregate {
            group_by: vec![],
            filter,
            aggregates: vec![count()],
            limit,
        };
        // field 1 > 3: rows 2 (5) and 3 (9).
        let store = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(&store, request(vec![u32_pred(1, Gt, 3)], None)));
        assert_eq!(
            groups,
            vec![AggregateGroup {
                key: vec![],
                values: vec![ScanValue::I64(2)]
            }]
        );
        assert_eq!(
            (store.gets(), store.scans(), store.range_counts()),
            (0, 0, 1),
            "counted from the index"
        );
        // The decode path's answer is identical.
        let scanned = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
        assert_eq!(
            groups_of(dispatch(&scanned, request(vec![u32_pred(1, Gt, 3)], None))),
            groups
        );
        assert_eq!(scanned.scans(), 1);
        // No filter: the whole index. Two bounds. `limit: Some(0)`: no group.
        let store = PlannerFixture::with_range(1);
        assert_eq!(
            groups_of(dispatch(&store, request(vec![], None)))[0].values,
            vec![ScanValue::I64(3)]
        );
        assert_eq!(
            groups_of(dispatch(
                &store,
                request(vec![u32_pred(1, Ge, 5), u32_pred(1, Lt, 9)], None)
            ))[0]
                .values,
            vec![ScanValue::I64(1)]
        );
        assert!(groups_of(dispatch(&store, request(vec![u32_pred(1, Gt, 3)], Some(0)))).is_empty());
        assert_eq!(
            (store.gets(), store.scans(), store.range_counts()),
            (0, 0, 3)
        );
        // Ineligible (a predicate on another field): decoded, still exact.
        let store = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(
            &store,
            request(vec![u32_pred(1, Gt, 3), labrador()], None),
        ));
        assert_eq!(groups[0].values, vec![ScanValue::I64(1)]);
        assert_eq!(store.range_counts(), 0, "never counted from the index");
        assert!(
            store.gets() + store.scans() > 0,
            "decoded (the intersection's bucket refuses here, so it scanned)"
        );
        // A refusing adapter decodes, still exact.
        let store = PlannerFixture::with_refusing_range(1);
        let groups = groups_of(dispatch(&store, request(vec![u32_pred(1, Gt, 3)], None)));
        assert_eq!(groups[0].values, vec![ScanValue::I64(2)]);
        assert_eq!((store.range_counts(), store.scans()), (1, 1));
    }

    /// `QKW-FR-003` (ADR-0082), acceptance criterion 1: eligible for
    /// `SUM`/`AVG`/`MIN`/`MAX` of the range field, alone or beside
    /// `COUNT(*)`, over `counted_walk_applies`'s shapes; not for an
    /// aggregate over another field, a `group_by`, or a second field in
    /// the filter.
    #[test]
    fn keyed_walk_applies_only_to_aggregates_over_the_range_field() {
        use CompareOp::{Ge, Gt};
        let spec = |func, field| AggregateSpec { func, field };
        let applies = |group_by: &[FieldRef], filter: &[Predicate], aggs: &[AggregateSpec]| {
            keyed_walk_applies(Some(1), group_by, filter, aggs)
        };
        for func in [
            AggregateFn::Sum,
            AggregateFn::Avg,
            AggregateFn::Min,
            AggregateFn::Max,
        ] {
            assert!(
                applies(&[], &[u32_pred(1, Ge, 1)], &[spec(func, Some(1))]),
                "{func:?}"
            );
            assert!(
                applies(&[], &[], &[spec(func, Some(1))]),
                "{func:?}, no filter"
            );
            assert!(
                !applies(&[], &[u32_pred(1, Ge, 1)], &[spec(func, Some(3))]),
                "{func:?} over another field"
            );
        }
        assert!(applies(
            &[],
            &[u32_pred(1, Gt, 1)],
            &[
                spec(AggregateFn::Count, None),
                spec(AggregateFn::Min, Some(1)),
                spec(AggregateFn::Max, Some(1))
            ]
        ));
        assert!(
            !applies(&[2], &[], &[spec(AggregateFn::Min, Some(1))]),
            "group_by"
        );
        assert!(
            !applies(
                &[],
                &[u32_pred(1, Ge, 1), labrador()],
                &[spec(AggregateFn::Max, Some(1))]
            ),
            "a second field in the filter"
        );
        assert!(
            !applies(
                &[],
                &[],
                &[
                    spec(AggregateFn::Count, None),
                    spec(AggregateFn::Sum, Some(3))
                ]
            ),
            "COUNT(*) beside a SUM over another field"
        );
        assert!(!keyed_walk_applies(
            None,
            &[],
            &[],
            &[spec(AggregateFn::Min, Some(1))]
        ));
    }

    /// `QKW-FR-004` (ADR-0082), acceptance criterion 2: `SUM`/`AVG`/`MIN`/
    /// `MAX` of the range field come from the walked keys with no record
    /// decoded, in the field's own kind, and equal the decode path's
    /// answers — over a range, over everything, and over nothing (the
    /// `0`/`0.0` empties); a count-only request still takes
    /// `counted_walk` (no keys); an aggregate over another field decodes.
    #[test]
    fn dispatch_reduces_the_range_field_from_the_walked_keys_without_reading_a_record() {
        use CompareOp::{Ge, Gt, Lt};
        let spec = |func, field| AggregateSpec { func, field };
        let all = || {
            vec![
                spec(AggregateFn::Count, None),
                spec(AggregateFn::Sum, Some(1)),
                spec(AggregateFn::Avg, Some(1)),
                spec(AggregateFn::Min, Some(1)),
                spec(AggregateFn::Max, Some(1)),
            ]
        };
        let groups_of = |response| match response {
            Response::Groups { groups } => groups,
            other => panic!("expected Groups, got {other:?}"),
        };
        let request = |filter: Vec<Predicate>, aggregates| Request::Aggregate {
            group_by: vec![],
            filter,
            aggregates,
            limit: None,
        };
        // field 1 (U32) is 3, 5, 9 on rows 1, 2, 3.
        for filter in [
            vec![],
            vec![u32_pred(1, Gt, 3)],
            vec![u32_pred(1, Ge, 5), u32_pred(1, Lt, 9)],
            vec![u32_pred(1, Gt, 9)],
        ] {
            let keyed = PlannerFixture::with_range(1);
            let scanned = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
            let a = groups_of(dispatch(&keyed, request(filter.clone(), all())));
            let b = groups_of(dispatch(&scanned, request(filter.clone(), all())));
            assert_eq!(a, b, "{filter:?}");
            assert_eq!(
                (keyed.gets(), keyed.scans(), keyed.range_counts()),
                (0, 0, 1),
                "{filter:?}: keys only"
            );
            assert_eq!(scanned.scans(), 1);
        }
        let keyed = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(&keyed, request(vec![u32_pred(1, Gt, 3)], all())));
        assert_eq!(
            groups[0].values,
            vec![
                ScanValue::I64(2),
                ScanValue::I64(14),
                ScanValue::F64(7.0),
                ScanValue::U32(5),
                ScanValue::U32(9)
            ],
            "5 and 9: count 2, sum 14, mean 7, min 5, max 9 in the field's kind"
        );
        let keyed = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(&keyed, request(vec![u32_pred(1, Gt, 9)], all())));
        assert_eq!(
            groups[0].values,
            vec![
                ScanValue::I64(0),
                ScanValue::I64(0),
                ScanValue::F64(0.0),
                ScanValue::U32(0),
                ScanValue::U32(0)
            ],
            "over nothing: the decode path's own empties"
        );
        // Count only: `counted_walk`, no keys materialized.
        let keyed = PlannerFixture::with_range(1);
        groups_of(dispatch(
            &keyed,
            request(
                vec![u32_pred(1, Gt, 3)],
                vec![spec(AggregateFn::Count, None)],
            ),
        ));
        assert_eq!((keyed.gets(), keyed.range_counts()), (0, 1));
        // An aggregate over another field decodes.
        let keyed = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(
            &keyed,
            request(
                vec![u32_pred(1, Gt, 3)],
                vec![spec(AggregateFn::Max, Some(3))],
            ),
        ));
        assert_eq!(keyed.range_counts(), 0);
        assert!(keyed.gets() > 0);
        assert_eq!(groups.len(), 1);
    }

    /// `QKG-FR-001` (ADR-0084), acceptance criterion 1: a `group_by` of
    /// exactly the range field is eligible — beside `COUNT(*)` and the
    /// four reductions, with or without bounds; any other `group_by`,
    /// or the range field beside another, is not.
    #[test]
    fn keyed_walk_applies_to_a_group_by_of_the_range_field() {
        use CompareOp::Ge;
        let spec = |func, field| AggregateSpec { func, field };
        let applies = |group_by: &[FieldRef], filter: &[Predicate], aggs: &[AggregateSpec]| {
            keyed_walk_applies(Some(1), group_by, filter, aggs)
        };
        let count = spec(AggregateFn::Count, None);
        assert!(applies(&[1], &[], std::slice::from_ref(&count)));
        assert!(applies(
            &[1],
            &[u32_pred(1, Ge, 1)],
            std::slice::from_ref(&count)
        ));
        for func in [
            AggregateFn::Sum,
            AggregateFn::Avg,
            AggregateFn::Min,
            AggregateFn::Max,
        ] {
            assert!(
                applies(
                    &[1],
                    &[u32_pred(1, Ge, 1)],
                    &[count.clone(), spec(func, Some(1))]
                ),
                "{func:?}"
            );
            assert!(
                !applies(&[1], &[], &[spec(func, Some(3))]),
                "{func:?} over another field"
            );
        }
        assert!(
            !applies(&[2], &[], std::slice::from_ref(&count)),
            "another field"
        );
        assert!(
            !applies(&[1, 2], &[], std::slice::from_ref(&count)),
            "the key beside another"
        );
        assert!(
            !applies(&[2, 1], &[], std::slice::from_ref(&count)),
            "another beside the key"
        );
        assert!(
            !applies(
                &[1],
                &[u32_pred(1, Ge, 1), labrador()],
                std::slice::from_ref(&count)
            ),
            "a second field in the filter"
        );
        assert!(!keyed_walk_applies(None, &[1], &[], &[count]));
    }

    /// `QKG-FR-002`/`QKG-FR-003` (ADR-0084), acceptance criterion 2: a
    /// `group_by` of the range field is one group per run of equal keys
    /// in the walk, keyed in the field's kind, each column reduced over
    /// the run, no record decoded — the decode path's identical groups
    /// under a bound (over a range, over nothing); the same set over
    /// everything; `limit` truncating the groups; the one shape that
    /// still decodes (a `limit` with no bound); a count-only grouped
    /// request taking the keys, not `counted_walk`.
    #[test]
    fn dispatch_groups_the_walked_keys_by_run_without_reading_a_record() {
        use CompareOp::{Ge, Gt, Lt};
        let spec = |func, field| AggregateSpec { func, field };
        let all = || {
            vec![
                spec(AggregateFn::Count, None),
                spec(AggregateFn::Sum, Some(1)),
                spec(AggregateFn::Avg, Some(1)),
                spec(AggregateFn::Min, Some(1)),
                spec(AggregateFn::Max, Some(1)),
            ]
        };
        // Field 1 (U32): 3, 3, 5, 9, 9, 9 on ids 1..=6 — two runs of
        // repeated keys and one singleton.
        let rows = || -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
            [3u32, 3, 5, 9, 9, 9]
                .into_iter()
                .enumerate()
                .map(|(i, key)| {
                    (
                        RecordId::from_u128(i as u128 + 1),
                        vec![(1, ScanValue::U32(key)), (2, ScanValue::Str("x".into()))],
                    )
                })
                .collect()
        };
        let keyed = || PlannerFixture::with_range_over(1, rows());
        let scanned = || PlannerFixture {
            rows: Some(rows()),
            ..PlannerFixture::with_index(Err(ErrorCode::Unsupported))
        };
        let groups_of = |response| match response {
            Response::Groups { groups } => groups,
            other => panic!("expected Groups, got {other:?}"),
        };
        let request = |filter: Vec<Predicate>, aggregates, limit| Request::Aggregate {
            group_by: vec![1],
            filter,
            aggregates,
            limit,
        };
        // Under a bound: the decode path's identical groups, in order.
        for (filter, limit) in [
            (vec![u32_pred(1, Gt, 3)], None),
            (vec![u32_pred(1, Ge, 3), u32_pred(1, Lt, 9)], None),
            (vec![u32_pred(1, Ge, 3)], Some(2)),
            (vec![u32_pred(1, Gt, 9)], None),
        ] {
            let (k, d) = (keyed(), scanned());
            let a = groups_of(dispatch(&k, request(filter.clone(), all(), limit)));
            let b = groups_of(dispatch(&d, request(filter.clone(), all(), limit)));
            assert_eq!(a, b, "{filter:?} {limit:?}");
            assert_eq!(
                (k.gets(), k.scans(), k.range_counts()),
                (0, 0, 1),
                "{filter:?}: keys only"
            );
            assert_eq!(d.scans(), 1);
        }
        let k = keyed();
        let groups = groups_of(dispatch(&k, request(vec![u32_pred(1, Gt, 3)], all(), None)));
        assert_eq!(
            groups,
            vec![
                AggregateGroup {
                    key: vec![(1, ScanValue::U32(5))],
                    values: vec![
                        ScanValue::I64(1),
                        ScanValue::I64(5),
                        ScanValue::F64(5.0),
                        ScanValue::U32(5),
                        ScanValue::U32(5)
                    ],
                },
                AggregateGroup {
                    key: vec![(1, ScanValue::U32(9))],
                    values: vec![
                        ScanValue::I64(3),
                        ScanValue::I64(27),
                        ScanValue::F64(9.0),
                        ScanValue::U32(9),
                        ScanValue::U32(9)
                    ],
                },
            ],
            "one group per run, keyed in the field's kind"
        );
        let k = keyed();
        let groups = groups_of(dispatch(&k, request(vec![u32_pred(1, Gt, 9)], all(), None)));
        assert!(groups.is_empty(), "over nothing: no keyed group");
        // Over everything: the same set as the decode path (which
        // buckets in scan order — here id order, which is key order).
        let (k, d) = (keyed(), scanned());
        let mut a = groups_of(dispatch(&k, request(vec![], all(), None)));
        let mut b = groups_of(dispatch(&d, request(vec![], all(), None)));
        assert_eq!(a.len(), 3);
        let by_key = |g: &AggregateGroup| match g.key[0].1 {
            ScanValue::U32(k) => k,
            ref other => panic!("{other:?}"),
        };
        a.sort_by_key(by_key);
        b.sort_by_key(by_key);
        assert_eq!(a, b);
        assert_eq!((k.gets(), k.range_counts()), (0, 1));
        // A `limit` with no bound: decodes, so the truncated set is the
        // decode path's own.
        let k = keyed();
        let groups = groups_of(dispatch(
            &k,
            request(vec![], vec![spec(AggregateFn::Count, None)], Some(1)),
        ));
        assert_eq!(groups.len(), 1);
        assert_eq!((k.range_counts(), k.scans()), (0, 1));
        // Count-only but grouped: the keys, not `counted_walk`'s count.
        let k = keyed();
        let groups = groups_of(dispatch(
            &k,
            request(
                vec![u32_pred(1, Ge, 3)],
                vec![spec(AggregateFn::Count, None)],
                None,
            ),
        ));
        assert_eq!(
            groups
                .iter()
                .map(|g| (g.key[0].1.clone(), g.values[0].clone()))
                .collect::<Vec<_>>(),
            vec![
                (ScanValue::U32(3), ScanValue::I64(2)),
                (ScanValue::U32(5), ScanValue::I64(1)),
                (ScanValue::U32(9), ScanValue::I64(3)),
            ]
        );
        assert_eq!((k.gets(), k.range_counts()), (0, 1));
    }

    /// `QPM-FR-001` (ADR-0086), acceptance criterion 1: every planned
    /// read classifies as the path `dispatch` takes — the four candidate
    /// plans, the three walk-only paths, the held-back grouped shape as
    /// its candidate step — and every other request as `None`.
    #[test]
    fn plan_of_names_the_path_each_planned_read_takes() {
        use CompareOp::{Ge, Gt};
        let spec = |func, field| AggregateSpec { func, field };
        let count = || vec![spec(AggregateFn::Count, None)];
        let query = |filter| Request::Query {
            select: Selection::All,
            filter,
            limit: None,
        };
        let aggregate = |group_by, filter, aggregates, limit| Request::Aggregate {
            group_by,
            filter,
            aggregates,
            limit,
        };
        let page = |order_by, filter| Request::FilteredPage {
            order_by,
            after: None,
            limit: 10,
            filter,
        };
        let ranged = PlannerFixture::with_range(1);
        let eq_only = PlannerFixture::with_index(Ok(vec![]));
        // The candidate step, per consumer.
        assert_eq!(plan_of(&ranged, &query(vec![])), Some(PlanKind::FullScan));
        assert_eq!(
            plan_of(&eq_only, &query(vec![labrador()])),
            Some(PlanKind::IndexEq)
        );
        assert_eq!(
            plan_of(&ranged, &query(vec![u32_pred(1, Gt, 3)])),
            Some(PlanKind::IndexRange)
        );
        assert_eq!(
            plan_of(&ranged, &query(vec![labrador(), u32_pred(1, Gt, 3)])),
            Some(PlanKind::IndexIntersect)
        );
        assert_eq!(
            plan_of(&ranged, &page(2, vec![u32_pred(1, Gt, 3)])),
            Some(PlanKind::IndexRange),
            "a page not ordered by the range field: its candidate step"
        );
        assert_eq!(
            plan_of(&ranged, &page(1, vec![u32_pred(1, Gt, 3)])),
            Some(PlanKind::BoundedWalk)
        );
        assert_eq!(
            plan_of(
                &ranged,
                &Request::Join(JoinSpec {
                    left_filter: vec![u32_pred(1, Ge, 1)],
                    ..join_spec(JoinRelation::Neighbors(None))
                })
            ),
            Some(PlanKind::IndexRange)
        );
        // The walk-only aggregates.
        assert_eq!(
            plan_of(
                &ranged,
                &aggregate(vec![], vec![u32_pred(1, Gt, 3)], count(), None)
            ),
            Some(PlanKind::CountedWalk)
        );
        assert_eq!(
            plan_of(
                &ranged,
                &aggregate(vec![], vec![], vec![spec(AggregateFn::Min, Some(1))], None)
            ),
            Some(PlanKind::KeyedWalk)
        );
        assert_eq!(
            plan_of(&ranged, &aggregate(vec![1], vec![], count(), None)),
            Some(PlanKind::KeyedWalk),
            "grouped by the key: the keys, even for a count"
        );
        assert_eq!(
            plan_of(&ranged, &aggregate(vec![1], vec![], count(), Some(1))),
            Some(PlanKind::FullScan),
            "grouped, no bound, a limit: decodes (QKG-FR-003)"
        );
        assert_eq!(
            plan_of(
                &ranged,
                &aggregate(vec![], vec![u32_pred(1, Gt, 3), labrador()], count(), None)
            ),
            Some(PlanKind::IndexIntersect),
            "a second field: the candidate step"
        );
        assert_eq!(
            plan_of(&eq_only, &aggregate(vec![], vec![], count(), None)),
            Some(PlanKind::FullScan),
            "no range field: never a walk"
        );
        // Not a planned read.
        for req in [
            Request::GetById {
                id: RecordId::from_u128(1),
            },
            Request::Page {
                order_by: 1,
                after: None,
                limit: 1,
            },
            Request::Metrics,
            Request::Delete {
                id: RecordId::from_u128(1),
            },
        ] {
            assert_eq!(plan_of(&ranged, &req), None, "{req:?}");
        }
    }

    /// `QRF-FR-003`/`QRF-FR-004` (ADR-0087), acceptance criteria 1–2:
    /// `KeyStats::with` folds count/sum/min/max exactly (empties `None`);
    /// an ungrouped reduction of the range field takes one `range_stats`
    /// fold and materializes no key, with the same answers `ADR-0082`
    /// pinned (a range, everything, nothing — the empties included) and
    /// the decode path's; a grouped one still takes the keys.
    #[test]
    fn dispatch_reduces_the_range_field_from_one_fold_without_materializing_keys() {
        use CompareOp::{Ge, Gt, Lt};
        assert_eq!(
            [3i64, 5, 9]
                .iter()
                .fold(KeyStats::default(), KeyStats::with),
            KeyStats {
                count: 3,
                sum: 17,
                min: Some(3),
                max: Some(9)
            }
        );
        assert_eq!(
            [9i64, -1, 4]
                .iter()
                .fold(KeyStats::default(), KeyStats::with),
            KeyStats {
                count: 3,
                sum: 12,
                min: Some(-1),
                max: Some(9)
            },
            "order-independent"
        );
        assert_eq!(KeyStats::default().min, None);
        let spec = |func, field| AggregateSpec { func, field };
        let all = || {
            vec![
                spec(AggregateFn::Count, None),
                spec(AggregateFn::Sum, Some(1)),
                spec(AggregateFn::Avg, Some(1)),
                spec(AggregateFn::Min, Some(1)),
                spec(AggregateFn::Max, Some(1)),
            ]
        };
        let groups_of = |response| match response {
            Response::Groups { groups } => groups,
            other => panic!("expected Groups, got {other:?}"),
        };
        let request = |group_by, filter: Vec<Predicate>| Request::Aggregate {
            group_by,
            filter,
            aggregates: all(),
            limit: None,
        };
        // field 1 (U32) is 3, 5, 9 on rows 1, 2, 3.
        for filter in [
            vec![],
            vec![u32_pred(1, Gt, 3)],
            vec![u32_pred(1, Ge, 5), u32_pred(1, Lt, 9)],
            vec![u32_pred(1, Gt, 9)],
        ] {
            let keyed = PlannerFixture::with_range(1);
            let scanned = PlannerFixture::with_index(Err(ErrorCode::Unsupported));
            let a = groups_of(dispatch(&keyed, request(vec![], filter.clone())));
            let b = groups_of(dispatch(&scanned, request(vec![], filter.clone())));
            assert_eq!(a, b, "{filter:?}");
            assert_eq!(
                (keyed.gets(), keyed.stat_folds(), keyed.key_walks()),
                (0, 1, 0),
                "{filter:?}: one fold, no keys"
            );
        }
        let keyed = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(&keyed, request(vec![], vec![u32_pred(1, Gt, 3)])));
        assert_eq!(
            groups[0].values,
            vec![
                ScanValue::I64(2),
                ScanValue::I64(14),
                ScanValue::F64(7.0),
                ScanValue::U32(5),
                ScanValue::U32(9)
            ]
        );
        let keyed = PlannerFixture::with_range(1);
        let groups = groups_of(dispatch(&keyed, request(vec![], vec![u32_pred(1, Gt, 9)])));
        assert_eq!(
            groups[0].values,
            vec![
                ScanValue::I64(0),
                ScanValue::I64(0),
                ScanValue::F64(0.0),
                ScanValue::U32(0),
                ScanValue::U32(0)
            ],
            "over nothing: the empties"
        );
        // Grouped: the keys, since runs need them.
        let keyed = PlannerFixture::with_range(1);
        groups_of(dispatch(&keyed, request(vec![1], vec![u32_pred(1, Gt, 3)])));
        assert_eq!((keyed.stat_folds(), keyed.key_walks()), (0, 1));
    }

    /// `QPR-FR-004` (ADR-0075), acceptance criterion 3: each comparator's
    /// bound, and `Unbounded` for an absent side.
    #[test]
    fn range_bounds_maps_each_comparator_to_its_bound() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt};
        let filter = [
            u32_pred(1, Gt, 1),
            u32_pred(1, Ge, 2),
            u32_pred(1, Lt, 3),
            u32_pred(1, Le, 4),
            u32_pred(1, Eq, 5),
        ];
        assert_eq!(
            range_bounds(&filter, Some(0), Some(2)),
            (
                Bound::Excluded(ScanValue::U32(1)),
                Bound::Excluded(ScanValue::U32(3))
            )
        );
        assert_eq!(
            range_bounds(&filter, Some(1), Some(3)),
            (
                Bound::Included(ScanValue::U32(2)),
                Bound::Included(ScanValue::U32(4))
            )
        );
        assert_eq!(
            range_bounds(&filter, Some(4), Some(4)),
            (
                Bound::Included(ScanValue::U32(5)),
                Bound::Included(ScanValue::U32(5))
            )
        );
        assert_eq!(
            range_bounds(&filter, Some(0), None),
            (Bound::Excluded(ScanValue::U32(1)), Bound::Unbounded)
        );
        assert_eq!(
            range_bounds(&filter, None, Some(3)),
            (Bound::Unbounded, Bound::Included(ScanValue::U32(4)))
        );
    }

    /// `QPR-FR-004`, acceptance criterion 4: the range path reads exactly
    /// the walked ids through `get` and never scans — one-sided,
    /// two-sided, and the one-key `Eq` walk.
    #[test]
    fn query_candidates_range_path_reads_only_the_walked_ids() {
        use CompareOp::{Eq, Ge, Gt, Lt};
        let store = PlannerFixture::with_range(1);
        let filter = [u32_pred(1, Gt, 3)];
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(
            plan,
            QueryPlan::IndexRange {
                lower: Some(0),
                upper: None
            }
        );
        let candidates = query_candidates(&store, plan, &filter);
        assert_eq!(store.gets(), 2, "one get per walked id");
        assert_eq!(store.scans(), 0, "the range path never scans");
        assert_eq!(
            candidates.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![RecordId::from_u128(2), RecordId::from_u128(3)],
            "field 1 > 3: the poodle (5) then the labrador (9), in key order"
        );

        let store = PlannerFixture::with_range(1);
        let filter = [u32_pred(1, Ge, 5), u32_pred(1, Lt, 9)];
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(
            ids_of(&query_candidates(&store, plan, &filter)),
            vec![RecordId::from_u128(2)],
            "5 <= field 1 < 9"
        );
        assert_eq!((store.gets(), store.scans()), (1, 0));

        let store = PlannerFixture::with_range(1);
        let filter = [u32_pred(1, Eq, 9)];
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert_eq!(
            ids_of(&query_candidates(&store, plan, &filter)),
            vec![RecordId::from_u128(3)],
            "field 1 = 9 as a one-key walk"
        );
        assert_eq!((store.gets(), store.scans()), (1, 0));
    }

    /// `QPR-FR-004`, acceptance criterion 4: a `range_ids` refusal on a
    /// field the adapter declared range-indexed falls back to one full
    /// scan with the identical result — and, through `dispatch`, a
    /// `Response::Rows`, never an error.
    #[test]
    fn query_candidates_range_refusal_falls_back_to_a_full_scan_without_an_error() {
        let store = PlannerFixture::with_refusing_range(1);
        let filter = [u32_pred(1, CompareOp::Gt, 3)];
        let plan = plan_query(&store.describe(), store.range_field(), &filter);
        assert!(matches!(plan, QueryPlan::IndexRange { .. }));
        let candidates = query_candidates(&store, plan, &filter);
        assert_eq!(store.scans(), 1);
        assert_eq!(store.gets(), 0);
        assert_eq!(ids_of(&candidates), ids_of(&sql_test_rows()));

        match dispatch(
            &store,
            Request::Query {
                select: Selection::All,
                filter: filter.to_vec(),
                limit: None,
            },
        ) {
            Response::Rows { rows } => assert_eq!(
                ids_of(&rows),
                vec![RecordId::from_u128(2), RecordId::from_u128(3)]
            ),
            other => panic!("expected Rows, got {other:?}"),
        }
    }

    /// `QPR-FR-005` (ADR-0075), acceptance criterion 4: every consumer —
    /// `Query` rows, `Aggregate` groups, the `filtered_page` default's
    /// exact sequence, `evaluate_join`'s pairs — returns the same result
    /// through the range walk as through the full scan, the walk never
    /// scanning; and a contradictory range answers zero rows, not an
    /// error.
    #[test]
    fn every_consumer_returns_the_same_result_on_the_range_walk_and_the_scan() {
        use CompareOp::{Ge, Gt, Lt};
        let filter = vec![u32_pred(1, Ge, 5)];
        let walked = || PlannerFixture::with_range(1);
        let scanned = || PlannerFixture::with_index(Err(ErrorCode::Unsupported));

        // Query.
        let rows_of = |response| match response {
            Response::Rows { mut rows } => {
                rows.sort_by_key(|(id, _)| *id);
                rows
            }
            other => panic!("expected Rows, got {other:?}"),
        };
        let request = || Request::Query {
            select: Selection::All,
            filter: filter.clone(),
            limit: None,
        };
        let (a, b) = (walked(), scanned());
        let (ra, rb) = (
            rows_of(dispatch(&a, request())),
            rows_of(dispatch(&b, request())),
        );
        assert_eq!(ra, rb);
        assert_eq!(
            ids_of(&ra),
            vec![RecordId::from_u128(2), RecordId::from_u128(3)]
        );
        assert_eq!((a.scans(), a.gets()), (0, 2));
        assert_eq!(b.scans(), 1);

        // Aggregate.
        let groups_of = |response| match response {
            Response::Groups { mut groups } => {
                groups.sort_by(|a, b| format!("{:?}", a.key).cmp(&format!("{:?}", b.key)));
                groups
            }
            other => panic!("expected Groups, got {other:?}"),
        };
        let request = || Request::Aggregate {
            group_by: vec![2],
            filter: filter.clone(),
            aggregates: vec![
                AggregateSpec {
                    func: AggregateFn::Count,
                    field: None,
                },
                AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(1),
                },
            ],
            limit: None,
        };
        let (a, b) = (walked(), scanned());
        let (ga, gb) = (
            groups_of(dispatch(&a, request())),
            groups_of(dispatch(&b, request())),
        );
        assert_eq!(ga, gb);
        assert_eq!(ga.len(), 2, "labrador (9) and poodle (5)");
        assert_eq!(a.scans(), 0);

        // The filtered_page default: the identical sequence.
        let (a, b) = (walked(), scanned());
        let pa = a.filtered_page(1, None, 10, &filter).unwrap();
        let pb = b.filtered_page(1, None, 10, &filter).unwrap();
        assert_eq!(pa, pb);
        assert_eq!(
            pa.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![RecordId::from_u128(2), RecordId::from_u128(3)]
        );
        assert_eq!(a.scans(), 0);

        // Join: the left side walks; the 1 — 3 edge yields (3, 1) only.
        let spec = JoinSpec {
            relation: JoinRelation::Neighbors(None),
            right_table: None,
            left: Selection::Fields(vec![2]),
            right: Selection::Fields(vec![1]),
            left_filter: filter.clone(),
            right_filter: vec![],
            limit: None,
        };
        let pair_ids = |rows: &[JoinedRow]| {
            let mut ids: Vec<(RecordId, RecordId)> =
                rows.iter().map(|r| (r.left_id, r.right_id)).collect();
            ids.sort();
            ids
        };
        let (a, b) = (walked(), scanned());
        let ja = evaluate_join(&a, &a, &spec);
        let jb = evaluate_join(&b, &b, &spec);
        assert_eq!(pair_ids(&ja), pair_ids(&jb));
        assert_eq!(
            pair_ids(&ja),
            vec![(RecordId::from_u128(3), RecordId::from_u128(1))]
        );
        assert_eq!(a.scans(), 0);

        // A contradictory range is data, not an error.
        let a = walked();
        match dispatch(
            &a,
            Request::Query {
                select: Selection::All,
                filter: vec![u32_pred(1, Gt, 9), u32_pred(1, Lt, 3)],
                limit: None,
            },
        ) {
            Response::Rows { rows } => assert!(rows.is_empty()),
            other => panic!("expected Rows, got {other:?}"),
        }
        assert_eq!(a.scans(), 0);
    }

    /// `QPR-FR-002` (ADR-0075): the key-to-pair mapping every `Uuid`-keyed
    /// adapter uses — exact at every key because `nil()`/`max()` bracket
    /// every real id — and `Malformed` for a non-`I64` bound.
    #[test]
    fn uuid_pair_bounds_bracket_each_key_with_the_id_sentinels() {
        let (nil, max) = (RecordId::nil(), RecordId::max());
        let i = |k| ScanValue::I64(k);
        assert_eq!(
            uuid_pair_bounds(Bound::Included(i(5)), Bound::Excluded(i(9))).unwrap(),
            (Bound::Included((5, nil)), Bound::Excluded((9, nil)))
        );
        assert_eq!(
            uuid_pair_bounds(Bound::Excluded(i(5)), Bound::Included(i(9))).unwrap(),
            (Bound::Excluded((5, max)), Bound::Included((9, max)))
        );
        assert_eq!(
            uuid_pair_bounds(Bound::Unbounded, Bound::Unbounded).unwrap(),
            (Bound::Unbounded, Bound::Unbounded)
        );
        assert_eq!(
            uuid_pair_bounds(Bound::Included(ScanValue::U32(5)), Bound::Unbounded),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            uuid_pair_bounds(
                Bound::Unbounded,
                Bound::Excluded(ScanValue::Str("x".into()))
            ),
            Err(ErrorCode::Malformed)
        );
    }

    /// `FPW-FR-001` (ADR-0076) as widened by `FPM-FR-001` (ADR-0077),
    /// acceptance criterion 1: the walk applies whenever `order_by` is
    /// the range field — bounds, `Ne`, a second field's predicate, an
    /// equality on an *unindexed* field — and not for another
    /// `order_by`, no range field, or a filter that plans the declared
    /// equality index (equality-first, `plan_query`'s own rule).
    #[test]
    fn bounded_walk_applies_to_the_ordered_field_unless_an_equality_index_plans() {
        use CompareOp::{Eq, Ge, Lt, Ne};
        // `planner_schema`: `1` unindexed `U32`, `2` indexed `Str`, `3`
        // indexed `U32`; the fixture's range field is `1` here.
        let schema = planner_schema();
        let i = |op, k| Predicate {
            field: 1,
            op,
            value: ScanValue::U32(k),
        };
        let applies = |order_by, range_field, filter: &[Predicate]| {
            bounded_walk_applies(order_by, range_field, &schema, filter)
        };
        assert!(applies(1, Some(1), &[]));
        assert!(applies(1, Some(1), &[i(Ge, 1)]));
        assert!(applies(1, Some(1), &[i(Ge, 1), i(Lt, 9)]));
        assert!(applies(1, Some(1), &[i(Eq, 5)]));
        assert!(applies(1, Some(1), &[i(Ne, 5)]), "Ne is a reject now");
        assert!(
            bounded_walk_applies(
                1,
                Some(1),
                &sql_test_schema(),
                &[i(Ge, 1), eq(2, ScanValue::Str("x".into()))]
            ),
            "an equality on an unindexed field is a reject (`sql_test_schema` indexes nothing)"
        );
        assert!(
            !applies(1, Some(1), &[i(Ge, 1), eq(3, ScanValue::U32(2))]),
            "an equality on the second declared index keeps the bucket too"
        );
        assert!(
            applies(
                1,
                Some(1),
                &[
                    i(Ge, 1),
                    Predicate {
                        field: 2,
                        op: Ne,
                        value: ScanValue::Str("x".into())
                    }
                ]
            ),
            "Ne on an indexed field does not plan the index"
        );
        assert!(
            !applies(1, Some(1), &[i(Ge, 1), eq(2, ScanValue::Str("x".into()))]),
            "an equality on the declared index keeps the bucket"
        );
        assert!(!applies(3, Some(1), &[i(Ge, 1)]), "another order_by");
        assert!(!applies(1, None, &[i(Ge, 1)]), "no range field");
        assert!(!applies(1, None, &[]), "no range field, empty filter");
    }

    /// `FPM-FR-002` (ADR-0077): the walked key is the record's own
    /// `I64` `order_by` value; a missing field or another kind is
    /// `Malformed`.
    #[test]
    fn walked_key_reads_the_order_by_field_as_i64() {
        let fields = vec![(1, ScanValue::U32(3)), (6, ScanValue::I64(-7))];
        assert_eq!(walked_key(&fields, 6), Ok(-7));
        assert_eq!(walked_key(&fields, 1), Err(ErrorCode::Malformed));
        assert_eq!(walked_key(&fields, 9), Err(ErrorCode::Malformed));
    }

    /// An `I64`-keyed store for [`bounded_filtered_page`] alone: rows by
    /// id, and a sorted `(key, id)` index the test's `walk` closure pages
    /// — which may name ids the store no longer holds (a concurrent
    /// delete's window), or hold a record under a key the index does not
    /// (a concurrent re-key's). Counts every `get`.
    struct WalkFixture {
        rows: Vec<(RecordId, Vec<(FieldRef, ScanValue)>)>,
        index: Vec<(i64, RecordId)>,
        gets: std::sync::atomic::AtomicUsize,
    }

    impl WalkFixture {
        /// `n` rows, id *n* at key `10 * n`, field `7` the `Str` bucket
        /// `n % buckets` — every key distinct, every bucket recurring.
        fn new(n: u128, buckets: u128) -> Self {
            let rows: Vec<_> = (1..=n)
                .map(|n| {
                    (
                        RecordId::from_u128(n),
                        vec![
                            (6, ScanValue::I64(10 * n as i64)),
                            (7, ScanValue::Str(format!("b{}", n % buckets))),
                        ],
                    )
                })
                .collect();
            let index = rows
                .iter()
                .map(|(id, fields)| (walked_key(fields, 6).unwrap(), *id))
                .collect();
            Self {
                rows,
                index,
                gets: Default::default(),
            }
        }
        fn walk(&self, after: Option<(i64, RecordId)>, limit: usize) -> Vec<RecordId> {
            self.index
                .iter()
                .filter(|pair| after.is_none_or(|cursor| **pair > cursor))
                .take(limit)
                .map(|(_, id)| *id)
                .collect()
        }
        fn gets(&self) -> usize {
            self.gets.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl ConnectionStore for WalkFixture {
        fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.rows
                .iter()
                .find(|(row_id, _)| *row_id == id)
                .map(|(_, fields)| fields.clone())
        }
        fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
            self.rows.clone()
        }
        fn filter_eq(&self, _: FieldRef, _: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn scan_field(&self, _: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn update_field(&self, _: RecordId, _: FieldRef, _: ScanValue) -> Result<bool, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn parent(&self, _: RecordId) -> Result<ParentLookup, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn children(&self, _: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn neighbors(&self, _: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn neighbors_by_relation(&self, _: RecordId, _: &str) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn list_relation_kinds(&self) -> Vec<String> {
            Vec::new()
        }
        fn validate_op(&self, _: &TransactionOp) -> Result<(), ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn describe(&self) -> DomainSchema {
            planner_schema()
        }
        fn apply_transaction(
            &self,
            _: &[TransactionOp],
            _: &[(RecordId, FieldRef, ScanValue)],
        ) -> Result<(), (usize, ErrorCode)> {
            Err((0, ErrorCode::Unsupported))
        }
    }

    /// `FPM-FR-002`/`003` (ADR-0077), acceptance criterion 2: the walk
    /// passes rejects until the page fills (reading exactly the pairs it
    /// had to), ends at the first cut, ends at the index's end, answers
    /// exactly what the default answers for every shape, and — the
    /// concurrency windows — back-fills past a vanished id, resumes after
    /// a re-keyed row, and stops rather than re-walk a chunk that
    /// advanced nothing.
    #[test]
    fn bounded_filtered_page_walks_past_rejects_until_the_page_fills() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let fixture = WalkFixture::new(40, 4);
        let id = RecordId::from_u128;
        let i = |op, k: i64| Predicate {
            field: 6,
            op,
            value: ScanValue::I64(k),
        };
        let b = |op, bucket: &str| Predicate {
            field: 7,
            op,
            value: ScanValue::Str(bucket.into()),
        };
        let page = |fixture: &WalkFixture, after, limit, filter: &[Predicate]| {
            bounded_filtered_page(fixture, |c, n| fixture.walk(c, n), 6, after, limit, filter)
                .unwrap()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        let default = |fixture: &WalkFixture, after, limit, filter: &[Predicate]| {
            filtered_page_by_candidates(fixture, 6, after, limit, filter)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };

        // Bucket b1 is ids 1, 5, 9, …: three of them past key 100 need
        // three chunks of 3 — pairs 11..=19 read, nothing further.
        let before = fixture.gets();
        assert_eq!(
            page(&fixture, None, 3, &[i(Gt, 100), b(Eq, "b1")]),
            vec![id(13), id(17), id(21)]
        );
        assert_eq!(fixture.gets() - before, 11, "pairs 11..=21, one get each");
        // The cut ends the page short, rejects notwithstanding.
        assert_eq!(
            page(&fixture, None, 10, &[i(Gt, 100), i(Le, 200), b(Eq, "b1")]),
            vec![id(13), id(17)]
        );
        // The index's end ends the page short.
        assert_eq!(
            page(&fixture, None, 10, &[i(Ge, 300), b(Eq, "b0")]),
            vec![id(32), id(36), id(40)]
        );
        // `Ne` on the walked field is a reject, not a cut.
        assert_eq!(
            page(&fixture, None, 3, &[i(Ge, 100), i(Ne, 110), i(Ne, 130)]),
            vec![id(10), id(12), id(14)]
        );
        // A client cursor composes with the bound and the rejects.
        assert_eq!(
            page(
                &fixture,
                Some((ScanValue::I64(170), id(17))),
                2,
                &[i(Gt, 100), b(Ne, "b1")]
            ),
            vec![id(18), id(19)]
        );
        // Every shape, against the default as the exact oracle.
        type Shape = (Option<(ScanValue, RecordId)>, usize, Vec<Predicate>);
        let shapes: Vec<Shape> = vec![
            (None, 5, vec![]),
            (None, 5, vec![b(Eq, "b2")]),
            (None, 100, vec![b(Eq, "b2")]),
            (None, 4, vec![i(Gt, 100), b(Eq, "b1")]),
            (None, 4, vec![i(Gt, 100), i(Lt, 250), b(Ne, "b1")]),
            (None, 4, vec![i(Eq, 200)]),
            (None, 4, vec![i(Eq, 200), b(Eq, "b1")]),
            (None, 4, vec![i(Gt, 300), i(Lt, 100)]),
            (Some((ScanValue::I64(120), id(12))), 3, vec![b(Eq, "b0")]),
            (
                Some((ScanValue::I64(120), id(12))),
                3,
                vec![i(Ge, 300), b(Eq, "b0")],
            ),
            (Some((ScanValue::I64(999), id(1))), 3, vec![b(Eq, "b0")]),
        ];
        for (after, limit, filter) in shapes {
            assert_eq!(
                page(&fixture, after.clone(), limit, &filter),
                default(&fixture, after.clone(), limit, &filter),
                "after {after:?}, limit {limit}, filter {filter:?}"
            );
        }

        // A vanished id (in the index, not the store) is back-filled.
        let mut deleted = WalkFixture::new(10, 1);
        deleted.rows.retain(|(id, _)| *id != RecordId::from_u128(3));
        assert_eq!(
            page(&deleted, None, 3, &[i(Gt, 10)]),
            vec![id(2), id(4), id(5)],
            "3 skipped, 5 back-filled"
        );
        // A re-keyed row (read at a later key than walked) moves the
        // *next* chunk's start to its read key: even ids are b0; the
        // first chunk of 3 walks 2, 3 (read at 65, rejected as b1), 4,
        // so the second chunk resumes after (65, 3) — 6 (at 60) is
        // passed over, 8 fills the page. `Page`'s consistency class,
        // named; the ids of the same chunk are still read.
        let mut rekeyed = WalkFixture::new(10, 2);
        rekeyed.rows[2].1[0] = (6, ScanValue::I64(65));
        assert_eq!(
            page(&rekeyed, None, 3, &[i(Gt, 10), b(Eq, "b0")]),
            vec![id(2), id(4), id(8)]
        );
        // A chunk that advances nothing ends the page: every id vanished.
        let mut emptied = WalkFixture::new(6, 1);
        emptied.rows.retain(|(id, _)| *id < RecordId::from_u128(3));
        let before = emptied.gets();
        assert_eq!(page(&emptied, None, 2, &[b(Eq, "b0")]), vec![id(1), id(2)]);
        assert_eq!(
            page(
                &emptied,
                Some((ScanValue::I64(20), id(2))),
                2,
                &[b(Eq, "b0")]
            ),
            Vec::<RecordId>::new()
        );
        assert_eq!(
            emptied.gets() - before,
            4,
            "2 read, then one chunk of 2 vanished, then stop"
        );
        // A record read with the wrong kind under `order_by` is `Malformed`.
        let mut malformed = WalkFixture::new(2, 1);
        malformed.rows[0].1[0] = (6, ScanValue::U32(1));
        assert_eq!(
            bounded_filtered_page(&malformed, |c, n| malformed.walk(c, n), 6, None, 2, &[]),
            Err(ErrorCode::Malformed)
        );
    }

    /// `FPW-FR-002` (ADR-0076), acceptance criterion 2: the start cursor
    /// is the later of the client's cursor and every lower bound's own
    /// cursor; `Lt`/`Le` contribute nothing, nor (`FPM-FR-002`) does a
    /// predicate on another field; `i64::MIN` has no cursor; a
    /// non-`I64` literal on the walked field or cursor is `Malformed`.
    #[test]
    fn bounded_walk_start_is_the_later_of_the_cursor_and_the_tightest_lower_bound() {
        use CompareOp::{Eq, Ge, Gt, Le, Lt};
        let i = |op, k| Predicate {
            field: 6,
            op,
            value: ScanValue::I64(k),
        };
        let (max, id7) = (RecordId::max(), RecordId::from_u128(7));
        assert_eq!(bounded_walk_start(6, None, &[]), Ok(None));
        assert_eq!(bounded_walk_start(6, None, &[i(Gt, 5)]), Ok(Some((5, max))));
        assert_eq!(bounded_walk_start(6, None, &[i(Ge, 5)]), Ok(Some((4, max))));
        assert_eq!(bounded_walk_start(6, None, &[i(Eq, 5)]), Ok(Some((4, max))));
        assert_eq!(bounded_walk_start(6, None, &[i(Lt, 5), i(Le, 9)]), Ok(None));
        assert_eq!(bounded_walk_start(6, None, &[i(Ge, i64::MIN)]), Ok(None));
        assert_eq!(
            bounded_walk_start(6, None, &[i(Gt, i64::MIN)]),
            Ok(Some((i64::MIN, max)))
        );
        assert_eq!(
            bounded_walk_start(6, None, &[i(Gt, 1), i(Gt, 5), i(Ge, 3)]),
            Ok(Some((5, max))),
            "the tightest lower bound"
        );
        let cursor = Some((ScanValue::I64(9), id7));
        assert_eq!(
            bounded_walk_start(6, cursor.clone(), &[i(Gt, 5)]),
            Ok(Some((9, id7))),
            "the cursor is later than the bound"
        );
        assert_eq!(
            bounded_walk_start(6, cursor.clone(), &[i(Ge, 20)]),
            Ok(Some((19, max))),
            "the bound is later than the cursor"
        );
        assert_eq!(
            bounded_walk_start(6, cursor.clone(), &[i(Gt, 9)]),
            Ok(Some((9, max))),
            "same key: (9, max) is later than (9, id 7)"
        );
        assert_eq!(bounded_walk_start(6, cursor, &[]), Ok(Some((9, id7))));
        assert_eq!(
            bounded_walk_start(6, Some((ScanValue::U32(9), id7)), &[]),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            bounded_walk_start(6, None, &[eq(6, ScanValue::Str("x".into()))]),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            bounded_walk_start(6, None, &[i(Gt, 5), eq(2, ScanValue::Str("x".into()))]),
            Ok(Some((5, max))),
            "another field's predicate is a reject, not a bound"
        );
    }

    /// `SQL-FR-006`: an empty filter matches every row; a filter matching
    /// nothing returns an empty `Vec`, never an error.
    #[test]
    fn evaluate_query_empty_filter_matches_everything_and_a_filter_matching_nothing_is_empty() {
        let all = evaluate_query(sql_test_rows(), &Selection::All, &[], None);
        assert_eq!(all.len(), 3);
        let none = evaluate_query(
            sql_test_rows(),
            &Selection::All,
            &[Predicate {
                field: 1,
                op: CompareOp::Gt,
                value: ScanValue::U32(100),
            }],
            None,
        );
        assert!(none.is_empty());
    }

    /// `SQL-FR-006`: `AND`-ed predicates across every comparator kind —
    /// only the row(s) matching every predicate come back.
    #[test]
    fn evaluate_query_ands_every_predicate_across_every_comparator() {
        let result = evaluate_query(
            sql_test_rows(),
            &Selection::All,
            &[
                Predicate {
                    field: 1,
                    op: CompareOp::Ge,
                    value: ScanValue::U32(5),
                },
                Predicate {
                    field: 2,
                    op: CompareOp::Eq,
                    value: ScanValue::Str("labrador".into()),
                },
            ],
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, RecordId::from_u128(3));

        for (op, expected_ids) in [
            (CompareOp::Eq, vec![2]),
            (CompareOp::Ne, vec![1, 3]),
            (CompareOp::Lt, vec![1]),
            (CompareOp::Le, vec![1, 2]),
            (CompareOp::Gt, vec![3]),
            (CompareOp::Ge, vec![2, 3]),
        ] {
            let rows = evaluate_query(
                sql_test_rows(),
                &Selection::All,
                &[Predicate {
                    field: 1,
                    op,
                    value: ScanValue::U32(5),
                }],
                None,
            );
            let mut ids: Vec<u128> = rows.iter().map(|(id, _)| id.as_u128() % 1000).collect();
            ids.sort();
            assert_eq!(ids, expected_ids, "comparator {op:?}");
        }
    }

    /// `SQL-FR-003`: `Selection::All` returns every field;
    /// `Selection::Fields` returns only the named subset.
    #[test]
    fn evaluate_query_selection_all_vs_named_fields() {
        let all = evaluate_query(sql_test_rows(), &Selection::All, &[], None);
        assert_eq!(all[0].1.len(), 2);
        let named = evaluate_query(sql_test_rows(), &Selection::Fields(vec![2]), &[], None);
        assert_eq!(named[0].1, vec![(2, ScanValue::Str("labrador".into()))]);
    }

    /// `SQL-FR-001`: `limit` truncates the row count and nothing else —
    /// shorter than the match count truncates, longer (or `None`) leaves
    /// every match.
    #[test]
    fn evaluate_query_limit_truncates_the_row_count_only() {
        let limited = evaluate_query(sql_test_rows(), &Selection::All, &[], Some(2));
        assert_eq!(limited.len(), 2);
        let unbounded = evaluate_query(sql_test_rows(), &Selection::All, &[], Some(100));
        assert_eq!(unbounded.len(), 3);
        let zero = evaluate_query(sql_test_rows(), &Selection::All, &[], Some(0));
        assert!(zero.is_empty());
    }

    /// `ENT4-FR-003` (ADR-0041): a `StrList`-kinded field in `group_by`
    /// is `Malformed` — the one path a list could otherwise take into
    /// `Response::Groups`, which the rule-3 strip does not cover. A
    /// `StrList` predicate is `Malformed` too (`value_matches_kind` has
    /// no list arm), and `Sum`/`Avg`/`Min`/`Max` over it fall under the
    /// existing orderable-kind rule.
    #[test]
    fn validate_aggregate_refuses_a_str_list_group_key_and_predicate() {
        let mut schema = sql_test_schema();
        schema.fields.push(protocol::FieldDescriptor {
            tag: 7,
            name: "aliases".into(),
            value_kind: protocol::ValueKind::StrList,
            capabilities: protocol::FieldCapabilities {
                filter_eq: false,
                scan: false,
                update: false,
            },
        });
        assert_eq!(
            validate_aggregate(&schema, &[7], &[], &[]),
            Err(ErrorCode::Malformed),
            "StrList in group_by"
        );
        assert_eq!(
            validate_aggregate(
                &schema,
                &[],
                &[Predicate {
                    field: 7,
                    op: CompareOp::Eq,
                    value: ScanValue::StrList(vec!["Ada".into()]),
                }],
                &[]
            ),
            Err(ErrorCode::Malformed),
            "StrList predicate"
        );
        assert_eq!(
            validate_aggregate(
                &schema,
                &[],
                &[],
                &[AggregateSpec {
                    func: AggregateFn::Max,
                    field: Some(7),
                }]
            ),
            Err(ErrorCode::Malformed),
            "MAX over a StrList"
        );
    }

    /// `AGG-FR-006`: an unknown field in `group_by` or an `AggregateSpec`
    /// is `UnknownField`; `Count` with an explicit field, `Sum`/`Avg`/
    /// `Min`/`Max` with no field, and `Sum`/`Avg`/`Min`/`Max` against
    /// `breed` (`Str`, non-orderable) are each `Malformed`; a fully valid
    /// spec is `Ok`.
    #[test]
    fn validate_aggregate_reports_unknown_fields_and_malformed_specs() {
        let schema = sql_test_schema();
        assert_eq!(
            validate_aggregate(&schema, &[99], &[], &[]),
            Err(ErrorCode::UnknownField),
            "unknown field in group_by"
        );
        assert_eq!(
            validate_aggregate(
                &schema,
                &[],
                &[],
                &[AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(99),
                }]
            ),
            Err(ErrorCode::UnknownField),
            "unknown field in an AggregateSpec"
        );
        assert_eq!(
            validate_aggregate(
                &schema,
                &[],
                &[],
                &[AggregateSpec {
                    func: AggregateFn::Count,
                    field: Some(1),
                }]
            ),
            Err(ErrorCode::Malformed),
            "COUNT with an explicit field"
        );
        for func in [
            AggregateFn::Sum,
            AggregateFn::Avg,
            AggregateFn::Min,
            AggregateFn::Max,
        ] {
            assert_eq!(
                validate_aggregate(&schema, &[], &[], &[AggregateSpec { func, field: None }]),
                Err(ErrorCode::Malformed),
                "{func:?} with no field"
            );
            assert_eq!(
                validate_aggregate(
                    &schema,
                    &[],
                    &[],
                    &[AggregateSpec {
                        func,
                        field: Some(2),
                    }]
                ),
                Err(ErrorCode::Malformed),
                "{func:?} against a Str field"
            );
        }
        assert_eq!(
            validate_aggregate(
                &schema,
                &[2],
                &[],
                &[
                    AggregateSpec {
                        func: AggregateFn::Count,
                        field: None,
                    },
                    AggregateSpec {
                        func: AggregateFn::Sum,
                        field: Some(1),
                    }
                ]
            ),
            Ok(()),
            "GROUP BY breed, COUNT(*), SUM(age) is valid"
        );
    }

    /// `AGG-FR-007`: `group_by` empty means exactly one implicit bucket
    /// over every filtered row; a filter matching nothing produces zero
    /// groups when `group_by` is non-empty, or one group whose `Count` is
    /// `0` when `group_by` is empty.
    #[test]
    fn evaluate_aggregate_group_by_empty_vs_grouped_and_a_filter_matching_nothing() {
        let whole_table = evaluate_aggregate(
            sql_test_rows(),
            &[],
            &[],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            None,
            &sql_test_schema(),
        );
        assert_eq!(whole_table.len(), 1);
        assert!(whole_table[0].key.is_empty());
        assert_eq!(whole_table[0].values, vec![ScanValue::I64(3)]);

        let grouped = evaluate_aggregate(
            sql_test_rows(),
            &[2],
            &[],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            None,
            &sql_test_schema(),
        );
        assert_eq!(grouped.len(), 2, "labrador and poodle");

        let no_group_by_no_match = evaluate_aggregate(
            sql_test_rows(),
            &[],
            &[Predicate {
                field: 1,
                op: CompareOp::Gt,
                value: ScanValue::U32(100),
            }],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            None,
            &sql_test_schema(),
        );
        assert_eq!(no_group_by_no_match.len(), 1);
        assert!(no_group_by_no_match[0].key.is_empty());
        assert_eq!(no_group_by_no_match[0].values, vec![ScanValue::I64(0)]);

        let grouped_no_match = evaluate_aggregate(
            sql_test_rows(),
            &[2],
            &[Predicate {
                field: 1,
                op: CompareOp::Gt,
                value: ScanValue::U32(100),
            }],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            None,
            &sql_test_schema(),
        );
        assert!(grouped_no_match.is_empty());
    }

    /// The implicit whole-table bucket's `Min`/`Max`/`Avg` on zero
    /// matching rows fall back to a `schema`-typed zero rather than
    /// panicking — the one case `AGG-FR-008`'s "a group with zero rows
    /// never appears" guarantee does not cover, since this group is the
    /// deliberate exception (acceptance criterion 4).
    #[test]
    fn evaluate_aggregate_min_max_avg_on_an_empty_implicit_bucket_do_not_panic() {
        let groups = evaluate_aggregate(
            sql_test_rows(),
            &[],
            &[Predicate {
                field: 1,
                op: CompareOp::Gt,
                value: ScanValue::U32(100),
            }],
            &[
                AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Avg,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Min,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Max,
                    field: Some(1),
                },
            ],
            None,
            &sql_test_schema(),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].values[0], ScanValue::I64(0), "SUM");
        assert_eq!(groups[0].values[1], ScanValue::F64(0.0), "AVG");
        assert_eq!(groups[0].values[2], ScanValue::U32(0), "MIN, age is U32");
        assert_eq!(groups[0].values[3], ScanValue::U32(0), "MAX, age is U32");
    }

    /// `AGG-FR-008`: every aggregate function's reduction, including the
    /// `Sum`/`Count`-vs-`Avg` cross-check acceptance criterion 6 names —
    /// `AVG` equals that same group's `SUM` divided by its `COUNT`.
    #[test]
    fn evaluate_aggregate_every_function_reduces_correctly() {
        // Grouped by breed: labrador = {age 3, age 9}, poodle = {age 5}.
        let groups = evaluate_aggregate(
            sql_test_rows(),
            &[2],
            &[],
            &[
                AggregateSpec {
                    func: AggregateFn::Count,
                    field: None,
                },
                AggregateSpec {
                    func: AggregateFn::Sum,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Avg,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Min,
                    field: Some(1),
                },
                AggregateSpec {
                    func: AggregateFn::Max,
                    field: Some(1),
                },
            ],
            None,
            &sql_test_schema(),
        );
        assert_eq!(groups.len(), 2);
        let by_breed = |breed: &str| {
            groups
                .iter()
                .find(|g| g.key == vec![(2, ScanValue::Str(breed.into()))])
                .unwrap_or_else(|| panic!("no group for {breed}"))
        };
        let labrador = by_breed("labrador");
        assert_eq!(labrador.values[0], ScanValue::I64(2), "COUNT");
        assert_eq!(labrador.values[1], ScanValue::I64(12), "SUM(3+9)");
        assert_eq!(labrador.values[2], ScanValue::F64(6.0), "AVG(12/2)");
        assert_eq!(labrador.values[3], ScanValue::U32(3), "MIN");
        assert_eq!(labrador.values[4], ScanValue::U32(9), "MAX");
        let poodle = by_breed("poodle");
        assert_eq!(poodle.values[0], ScanValue::I64(1), "COUNT");
        assert_eq!(poodle.values[1], ScanValue::I64(5), "SUM");
        assert_eq!(poodle.values[2], ScanValue::F64(5.0), "AVG(5/1)");
        assert_eq!(poodle.values[3], ScanValue::U32(5), "MIN");
        assert_eq!(poodle.values[4], ScanValue::U32(5), "MAX");
    }

    /// `AGG-FR-007`: `limit` truncates the *group* count, applied after
    /// the full reduction.
    #[test]
    fn evaluate_aggregate_limit_truncates_the_group_count_only() {
        let limited = evaluate_aggregate(
            sql_test_rows(),
            &[2],
            &[],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            Some(1),
            &sql_test_schema(),
        );
        assert_eq!(limited.len(), 1);
        let unbounded = evaluate_aggregate(
            sql_test_rows(),
            &[2],
            &[],
            &[AggregateSpec {
                func: AggregateFn::Count,
                field: None,
            }],
            Some(100),
            &sql_test_schema(),
        );
        assert_eq!(unbounded.len(), 2);
    }

    /// `AGB-FR-001`/`AGB-FR-002` (ADR-0085), acceptance criterion 1: the
    /// hashed bucket yields exactly the buckets the linear search did —
    /// first-seen order (rows arriving out of key order), a composite
    /// `Str`+`Bool` key, a row lacking the group field (its key is the
    /// shorter tuple, one bucket of its own), and a re-check of every
    /// value kind a key can hold against `ScanValue`'s own equality.
    #[test]
    fn evaluate_aggregate_hashed_buckets_are_the_linear_search_s_buckets_in_first_seen_order() {
        let row = |id: u128, fields: Vec<(FieldRef, ScanValue)>| (RecordId::from_u128(id), fields);
        let rows = vec![
            row(
                1,
                vec![(1, ScanValue::U32(9)), (2, ScanValue::Str("b".into()))],
            ),
            row(
                2,
                vec![(1, ScanValue::U32(3)), (2, ScanValue::Str("a".into()))],
            ),
            row(
                3,
                vec![(1, ScanValue::U32(9)), (2, ScanValue::Str("b".into()))],
            ),
            row(4, vec![(2, ScanValue::Str("a".into()))]),
            row(
                5,
                vec![(1, ScanValue::U32(3)), (2, ScanValue::Str("a".into()))],
            ),
            row(
                6,
                vec![(1, ScanValue::U32(9)), (2, ScanValue::Str("a".into()))],
            ),
        ];
        let count = AggregateSpec {
            func: AggregateFn::Count,
            field: None,
        };
        let groups = evaluate_aggregate(
            rows.clone(),
            &[1, 2],
            &[],
            std::slice::from_ref(&count),
            None,
            &planner_schema(),
        );
        let shape: Vec<(Vec<(FieldRef, ScanValue)>, i64)> = groups
            .into_iter()
            .map(|g| {
                let n = match g.values[0] {
                    ScanValue::I64(n) => n,
                    ref other => panic!("{other:?}"),
                };
                (g.key, n)
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (
                    vec![(1, ScanValue::U32(9)), (2, ScanValue::Str("b".into()))],
                    2
                ),
                (
                    vec![(1, ScanValue::U32(3)), (2, ScanValue::Str("a".into()))],
                    2
                ),
                (vec![(2, ScanValue::Str("a".into()))], 1),
                (
                    vec![(1, ScanValue::U32(9)), (2, ScanValue::Str("a".into()))],
                    1
                ),
            ],
            "first-seen order; a row lacking the field keys on what it has"
        );
        // `limit` still truncates the first-seen order.
        let two = evaluate_aggregate(rows, &[1, 2], &[], &[count], Some(2), &planner_schema());
        assert_eq!(two.len(), 2);
        assert_eq!(two[1].key[0], (1, ScanValue::U32(3)));
        // Every kind a key can hold, equal exactly as `ScanValue` is.
        for (a, b, same) in [
            (ScanValue::U32(1), ScanValue::U32(1), true),
            (ScanValue::U32(1), ScanValue::I64(1), false),
            (ScanValue::I64(-1), ScanValue::I64(-1), true),
            (ScanValue::Bool(true), ScanValue::Bool(false), false),
            (ScanValue::Str("x".into()), ScanValue::Str("x".into()), true),
            (
                ScanValue::Str("x".into()),
                ScanValue::Str("y".into()),
                false,
            ),
            (
                ScanValue::StrList(vec!["a".into()]),
                ScanValue::StrList(vec!["a".into()]),
                true,
            ),
            (ScanValue::F64(1.5), ScanValue::F64(1.5), true),
            (ScanValue::F64(1.5), ScanValue::F64(2.5), false),
        ] {
            assert_eq!(
                BucketKey::of(&a) == BucketKey::of(&b),
                same,
                "{a:?} vs {b:?}"
            );
            assert_eq!(a == b, same);
        }
    }

    /// Spin up `serve` over `FixtureStore` on a loopback port with
    /// authentication configured, so the `Hello` intercept is exercised
    /// exactly where it sits: ahead of the auth gate. Returns the address
    /// and a connected, framed client stream.
    fn hello_fixture() -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => panic!("bind loopback listener: {e}"),
        };
        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(e) => panic!("listener address: {e}"),
        };
        let auth = ServeOptions::new(Some("ro".into()), Some("rw".into()));
        thread::spawn(move || serve(listener, Arc::new(FixtureStore), auth));
        let stream = match TcpStream::connect(addr) {
            Ok(s) => s,
            Err(e) => panic!("connect to fixture server: {e}"),
        };
        let peer = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => panic!("clone client stream: {e}"),
        };
        (BufReader::new(stream), BufWriter::new(peer))
    }

    fn roundtrip(
        reader: &mut BufReader<TcpStream>,
        writer: &mut BufWriter<TcpStream>,
        req: &Request,
    ) -> Response {
        if let Err(e) = framing::write_message(writer, req) {
            panic!("write request: {e}");
        }
        if let Err(e) = writer.flush() {
            panic!("flush request: {e}");
        }
        match framing::read_message(reader) {
            Ok(resp) => resp,
            Err(e) => panic!("read response: {e}"),
        }
    }

    /// Design criteria 3 and 4 on one connection each: a newer client is
    /// answered with this build's own version, an older one with its own;
    /// `Hello` is answered on an auth-configured server *before* any
    /// token is presented, and the gate behind it is intact — the next
    /// non-`Hello` request is still `Unauthenticated`.
    #[test]
    fn hello_is_answered_unauthenticated_with_the_min_version() {
        let (mut reader, mut writer) = hello_fixture();
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: PROTOCOL_VERSION + 3
                }
            ),
            Response::Hello {
                protocol_version: PROTOCOL_VERSION
            }
        );
        assert_eq!(
            roundtrip(&mut reader, &mut writer, &Request::DescribeSchema),
            err_response(ErrorCode::Unauthenticated)
        );

        let (mut reader, mut writer) = hello_fixture();
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: 1
                }
            ),
            Response::Hello {
                protocol_version: 1
            }
        );
        assert_eq!(
            roundtrip(&mut reader, &mut writer, &Request::DescribeSchema),
            err_response(ErrorCode::Unauthenticated)
        );
    }

    /// `PROTO-FR-004`: version 0 and a second `Hello` are each `Malformed`,
    /// and neither ends the connection — the client can carry on (here,
    /// into the auth gate, which is still in place).
    #[test]
    fn hello_version_zero_and_a_second_hello_are_malformed_but_not_fatal() {
        let (mut reader, mut writer) = hello_fixture();
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: 0
                }
            ),
            err_response(ErrorCode::Malformed)
        );
        // The rejected frame was still the first frame; a `Hello` after it
        // is no longer first.
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: 1
                }
            ),
            err_response(ErrorCode::Malformed)
        );
        assert_eq!(
            roundtrip(&mut reader, &mut writer, &Request::DescribeSchema),
            err_response(ErrorCode::Unauthenticated)
        );

        // A valid first `Hello`, then a second valid one: the second is
        // `Malformed` too.
        let (mut reader, mut writer) = hello_fixture();
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            ),
            Response::Hello {
                protocol_version: PROTOCOL_VERSION
            }
        );
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            ),
            err_response(ErrorCode::Malformed)
        );
        // And a `Hello` that is not the first frame at all — after an
        // `Authenticate` — is `Malformed` regardless of the auth outcome.
        let (mut reader, mut writer) = hello_fixture();
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Authenticate { token: "rw".into() }
            ),
            Response::Ok
        );
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            ),
            err_response(ErrorCode::Malformed)
        );
        // The connection is still authenticated and serving.
        assert_eq!(
            roundtrip(
                &mut reader,
                &mut writer,
                &Request::GetById {
                    id: RecordId::from_u128(1)
                }
            ),
            Response::Record {
                id: RecordId::from_u128(1),
                fields: vec![(FIELD_A, ScanValue::U32(7))]
            }
        );
    }

    #[test]
    fn auth_config_default_is_unconfigured() {
        assert!(!ServeOptions::default().is_configured());
        assert_eq!(ServeOptions::default().check("anything"), None);
    }

    #[test]
    fn auth_config_check_maps_each_token_to_its_own_class() {
        let auth = ServeOptions::new(Some("ro-secret".into()), Some("rw-secret".into()));
        assert!(auth.is_configured());
        assert_eq!(auth.check("ro-secret"), Some(TokenClass::ReadOnly));
        assert_eq!(auth.check("rw-secret"), Some(TokenClass::ReadWrite));
        assert_eq!(auth.check("wrong"), None);
        // A prefix or superstring of a real token must not match — rules
        // out an accidental substring/prefix comparison bug.
        assert_eq!(auth.check("ro-secret-extra"), None);
        assert_eq!(auth.check("ro-secre"), None);
    }

    #[test]
    fn auth_config_works_with_only_one_class_configured() {
        let read_only_only = ServeOptions::new(Some("ro-secret".into()), None);
        assert_eq!(
            read_only_only.check("ro-secret"),
            Some(TokenClass::ReadOnly)
        );
        assert_eq!(read_only_only.check("rw-secret"), None);

        let read_write_only = ServeOptions::new(None, Some("rw-secret".into()));
        assert_eq!(
            read_write_only.check("rw-secret"),
            Some(TokenClass::ReadWrite)
        );
        assert_eq!(read_write_only.check("ro-secret"), None);
    }

    /// `CLS-FR-003`: exact byte equality, and only exact byte equality —
    /// a prefix/superstring must not match, matching `check`'s own
    /// substring-safety test above.
    #[test]
    fn class_for_certificate_matches_by_exact_der_bytes_only() {
        let auth = ServeOptions::default()
            .with_certificate_class(vec![1, 2, 3], TokenClass::ReadOnly)
            .with_certificate_class(vec![4, 5, 6], TokenClass::ReadWrite);
        assert_eq!(
            auth.class_for_certificate(&[1, 2, 3]),
            Some(TokenClass::ReadOnly)
        );
        assert_eq!(
            auth.class_for_certificate(&[4, 5, 6]),
            Some(TokenClass::ReadWrite)
        );
        assert_eq!(auth.class_for_certificate(&[1, 2, 3, 4]), None);
        assert_eq!(auth.class_for_certificate(&[1, 2]), None);
        assert_eq!(auth.class_for_certificate(&[9, 9, 9]), None);
    }

    /// `CLS-FR-003`: a certificates-only `ServeOptions` (no tokens) is
    /// `is_configured()` — the safe direction `AUTH-FR-007` requires of a
    /// configured server (`SERVER-MTLS-CLASS-DESIGN.md`'s "Security,
    /// privacy, and compatibility").
    #[test]
    fn is_configured_is_true_with_only_a_certificate_class() {
        let auth = ServeOptions::default().with_certificate_class(vec![1], TokenClass::ReadOnly);
        assert!(auth.is_configured());
    }

    /// `RPL-FR-002`: a replication-token-only `ServeOptions` (no
    /// read-only/read-write tokens, no certificates) is `is_configured()`
    /// too — the same "any credential configured closes the anonymous
    /// `ReadWrite` default" posture a `read_only_token`-only server
    /// already has, so `Authenticate` is a real gate, not a no-op, the
    /// moment an operator sets `SERVER_AUTH_REPLICATION_TOKEN` alone.
    #[test]
    fn is_configured_is_true_with_only_a_replication_token() {
        let auth = ServeOptions::default().with_replication_token("repl-secret".to_string());
        assert!(auth.is_configured());
    }

    /// `RPL-FR-002`: `check` classes a token matching `replication_token`
    /// as `TokenClass::Replication`, distinct from every other
    /// configured token — even one that happens to share a slot's exact
    /// bytes with another class would still classify by which slot
    /// actually matched (here, none do, since the three tokens differ).
    #[test]
    fn check_classes_a_replication_token_distinctly() {
        let mut auth = ServeOptions::new(Some("ro-secret".into()), Some("rw-secret".into()));
        auth = auth.with_replication_token("repl-secret".to_string());
        assert_eq!(auth.check("repl-secret"), Some(TokenClass::Replication));
        assert_eq!(auth.check("ro-secret"), Some(TokenClass::ReadOnly));
        assert_eq!(auth.check("rw-secret"), Some(TokenClass::ReadWrite));
        assert_eq!(auth.check("unknown"), None);
    }

    /// `CLS-FR-006`: `Debug` prints counts per class, never a configured
    /// leaf's bytes.
    #[test]
    fn auth_config_debug_prints_certificate_counts_not_bytes() {
        let auth = ServeOptions::default()
            .with_certificate_class(vec![0xAB, 0xCD], TokenClass::ReadOnly)
            .with_certificate_class(vec![0xEF], TokenClass::ReadWrite)
            .with_certificate_class(vec![0x12], TokenClass::ReadWrite);
        let printed = format!("{auth:?}");
        assert!(printed.contains("read_only_certificates: 1"));
        assert!(printed.contains("read_write_certificates: 2"));
        assert!(!printed.contains("171")); // 0xAB as decimal — no raw byte ever printed
        assert!(!printed.contains("[171, 205]"));
    }

    /// `RL-FR-006`, acceptance criterion 6: valid input parses, every
    /// documented malformed shape is rejected.
    #[test]
    fn rate_limit_parse_accepts_valid_and_rejects_malformed_input() {
        assert_eq!(
            RateLimit::parse("10/60").unwrap(),
            RateLimit {
                failures: 10,
                window: Duration::from_secs(60),
            }
        );
        for bad in ["10", "0/60", "10/0", "a/b", "10/", "/60", "", "10/60/1"] {
            assert!(
                RateLimit::parse(bad).is_err(),
                "{bad:?} should have been rejected"
            );
        }
    }

    /// Acceptance criterion 3: two peers are tracked independently.
    #[test]
    fn failure_table_tracks_each_peer_independently() {
        let table = FailureTable::new(RateLimit {
            failures: 3,
            window: Duration::from_secs(60),
        });
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "127.0.0.2".parse().unwrap();
        for _ in 0..3 {
            table.note_failure(a);
        }
        assert!(table.is_throttled(a));
        assert!(!table.is_throttled(b));
    }

    /// Acceptance criterion 4: after the window elapses, a peer that was
    /// over budget is under budget again.
    #[test]
    fn failure_table_forgets_a_peer_once_its_window_elapses() {
        let table = FailureTable::new(RateLimit {
            failures: 1,
            window: Duration::from_millis(20),
        });
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        table.note_failure(peer);
        assert!(table.is_throttled(peer));
        std::thread::sleep(Duration::from_millis(30));
        assert!(!table.is_throttled(peer));
    }

    /// Acceptance criterion 5: inserting more addresses than
    /// `MAX_TRACKED_PEERS` evicts (here, since nothing has expired) the
    /// oldest entries first — the table never exceeds the cap, and the
    /// earliest-tracked peer is the one that falls out.
    #[test]
    fn failure_table_never_exceeds_max_tracked_peers() {
        let table = FailureTable::new(RateLimit {
            failures: 1,
            window: Duration::from_secs(3600),
        });
        let first = IpAddr::V4(std::net::Ipv4Addr::from(0u32));
        for i in 0..(MAX_TRACKED_PEERS as u32 + 10) {
            let peer = IpAddr::V4(std::net::Ipv4Addr::from(i));
            table.note_failure(peer);
            assert!(table.peers.lock().unwrap().len() <= MAX_TRACKED_PEERS);
        }
        assert!(!table.peers.lock().unwrap().contains_key(&first));
    }

    /// `AUTH-FR-006`'s empirical half: a wrong token that differs from the
    /// configured one at the very first byte must not check measurably
    /// faster than one that differs only at the very last byte — the
    /// classic signature of an early-exit (non-constant-time) comparison.
    /// Measured directly against `ServeOptions::check` (not over a real TCP
    /// round trip, unlike the rest of this crate's server tests): a
    /// network hop's own jitter (microseconds to milliseconds) would
    /// completely swamp the signal this specific claim is about — a
    /// difference on the order of one byte comparison in a same-length
    /// byte string. Every configured token is checked unconditionally
    /// regardless of position (see `check`'s own doc comment), so this is
    /// expected to hold structurally, not just empirically; the timing
    /// measurement is still real evidence per `SERVER-AUTH-DESIGN.md`'s
    /// own "not just a read-through" verification plan.
    #[test]
    fn token_comparison_time_does_not_depend_on_where_the_mismatch_is() {
        use std::time::Instant;

        let configured = "a".repeat(64);
        let auth = ServeOptions::new(None, Some(configured.clone()));

        let mut differs_at_start = "b".to_string();
        differs_at_start.push_str(&"a".repeat(63));
        let mut differs_at_end = "a".repeat(63);
        differs_at_end.push('b');
        assert_eq!(differs_at_start.len(), configured.len());
        assert_eq!(differs_at_end.len(), configured.len());

        const ITERATIONS: u32 = 20_000;

        // Warm up (first-touch page faults, branch predictor, etc.) before
        // the real measurement, same discipline this crate's own
        // benchmarks already use.
        for _ in 0..1_000 {
            std::hint::black_box(auth.check(std::hint::black_box(&differs_at_start)));
            std::hint::black_box(auth.check(std::hint::black_box(&differs_at_end)));
        }

        let start_timer = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(auth.check(std::hint::black_box(&differs_at_start)));
        }
        let start_elapsed = start_timer.elapsed();

        let end_timer = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(auth.check(std::hint::black_box(&differs_at_end)));
        }
        let end_elapsed = end_timer.elapsed();

        let ratio = start_elapsed.as_secs_f64() / end_elapsed.as_secs_f64().max(1e-12);
        assert!(
            (0.2..5.0).contains(&ratio),
            "mismatch-position timing ratio {ratio} is well outside a noise-only \
             range (first-byte-diff: {start_elapsed:?}, last-byte-diff: {end_elapsed:?}) \
             — investigate for an early-exit comparison"
        );
    }

    /// `JOIN-FR-002`/`003` (ADR-0044): a fixture with every relation kind
    /// real and self-referential — three records; symmetric edges 1–2
    /// (label `r`) and 2–3 (label `q`); 2 and 3 report to 1.
    struct JoinFixture;

    impl JoinFixture {
        fn value(id: RecordId) -> Option<u32> {
            [1u128, 2, 3]
                .into_iter()
                .find(|n| RecordId::from_u128(*n) == id)
                .map(|n| n as u32)
        }
    }

    impl ConnectionStore for JoinFixture {
        fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
            Self::value(id).map(|v| vec![(FIELD_A, ScanValue::U32(v))])
        }
        fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
            [1u128, 2, 3]
                .into_iter()
                .map(|n| {
                    (
                        RecordId::from_u128(n),
                        vec![(FIELD_A, ScanValue::U32(n as u32))],
                    )
                })
                .collect()
        }
        fn filter_eq(&self, _f: FieldRef, _v: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn scan_field(&self, _f: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn update_field(&self, _: RecordId, _: FieldRef, _: ScanValue) -> Result<bool, ErrorCode> {
            Err(ErrorCode::Unsupported)
        }
        fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
            Ok(match Self::value(id) {
                Some(1) => ParentLookup::NoParent,
                Some(_) => ParentLookup::Parent(RecordId::from_u128(1)),
                None => ParentLookup::ChildNotFound,
            })
        }
        fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            Ok(if id == RecordId::from_u128(1) {
                vec![RecordId::from_u128(2), RecordId::from_u128(3)]
            } else {
                vec![]
            })
        }
        fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
            let mut all = self.neighbors_by_relation(id, "r")?;
            all.extend(self.neighbors_by_relation(id, "q")?);
            Ok(all)
        }
        fn neighbors_by_relation(
            &self,
            id: RecordId,
            relation: &str,
        ) -> Result<Vec<RecordId>, ErrorCode> {
            let edge = match relation {
                "r" => (1u128, 2u128),
                "q" => (2, 3),
                _ => return Err(ErrorCode::Malformed),
            };
            Ok(if id == RecordId::from_u128(edge.0) {
                vec![RecordId::from_u128(edge.1)]
            } else if id == RecordId::from_u128(edge.1) {
                vec![RecordId::from_u128(edge.0)]
            } else {
                vec![]
            })
        }
        fn list_relation_kinds(&self) -> Vec<String> {
            vec!["r".into(), "q".into()]
        }
        fn describe_relations(&self) -> Vec<RelationDescriptor> {
            let mut out =
                default_relation_descriptors(&self.describe(), self.list_relation_kinds());
            out.push(RelationDescriptor {
                name: "parent".into(),
                kind: JoinRelation::Parent,
                target_table: None,
            });
            out.push(RelationDescriptor {
                name: "children".into(),
                kind: JoinRelation::Children,
                target_table: None,
            });
            out
        }
        fn validate_op(&self, _op: &TransactionOp) -> Result<(), ErrorCode> {
            Ok(())
        }
        fn describe(&self) -> DomainSchema {
            let mut schema = FixtureStore.describe();
            schema.relations.neighbors = true;
            schema
        }
        fn apply_transaction(
            &self,
            _updates: &[TransactionOp],
            _read_set: &[(RecordId, FieldRef, ScanValue)],
        ) -> Result<(), (usize, ErrorCode)> {
            Ok(())
        }
    }

    fn join_spec(relation: JoinRelation) -> JoinSpec {
        JoinSpec {
            relation,
            right_table: None,
            left: Selection::All,
            right: Selection::All,
            left_filter: vec![],
            right_filter: vec![],
            limit: None,
        }
    }

    fn pairs(rows: &[JoinedRow]) -> Vec<(u128, u128)> {
        rows.iter()
            .map(|r| (r.left_id.as_u128(), r.right_id.as_u128()))
            .collect()
    }

    /// `JOIN-FR-002`: the default lists `neighbors` plus one entry per
    /// label when the schema has a symmetric relation, and never
    /// `parent`/`children` — an adapter must claim those itself. A domain
    /// with no symmetric relation (`FixtureStore`) lists nothing.
    #[test]
    fn default_relation_descriptors_list_symmetric_labels_only() {
        assert!(FixtureStore.describe_relations().is_empty());
        let listed = default_relation_descriptors(
            &JoinFixture.describe(),
            JoinFixture.list_relation_kinds(),
        );
        assert_eq!(
            listed,
            vec![
                RelationDescriptor {
                    name: "neighbors".into(),
                    kind: JoinRelation::Neighbors(None),
                    target_table: None,
                },
                RelationDescriptor {
                    name: "r".into(),
                    kind: JoinRelation::Neighbors(Some("r".into())),
                    target_table: None,
                },
                RelationDescriptor {
                    name: "q".into(),
                    kind: JoinRelation::Neighbors(Some("q".into())),
                    target_table: None,
                },
            ]
        );
        // The override adds the directed pair.
        assert_eq!(JoinFixture.describe_relations().len(), 5);
    }

    /// `JOIN-FR-003`: every rejection, with existing codes only.
    #[test]
    fn validate_join_refuses_other_tables_unknown_fields_and_unlisted_relations() {
        let schema = JoinFixture.describe();
        let relations = JoinFixture.describe_relations();
        let mut other_table = join_spec(JoinRelation::Parent);
        other_table.right_table = Some("customer".into());
        assert_eq!(
            validate_join(&schema, &relations, None, &other_table),
            Err(ErrorCode::Malformed),
            "right_table is ADR-0045's, Malformed until then"
        );
        let mut bad_left = join_spec(JoinRelation::Parent);
        bad_left.left = Selection::Fields(vec![99]);
        assert_eq!(
            validate_join(&schema, &relations, None, &bad_left),
            Err(ErrorCode::UnknownField)
        );
        let mut bad_right_filter = join_spec(JoinRelation::Parent);
        bad_right_filter.right_filter = vec![Predicate {
            field: FIELD_A,
            op: CompareOp::Eq,
            value: ScanValue::Str("x".into()),
        }];
        assert_eq!(
            validate_join(&schema, &relations, None, &bad_right_filter),
            Err(ErrorCode::Malformed)
        );
        // `FixtureStore` lists no relation at all: any join is Malformed.
        assert_eq!(
            validate_join(
                &FixtureStore.describe(),
                &FixtureStore.describe_relations(),
                None,
                &join_spec(JoinRelation::Parent)
            ),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            validate_join(
                &schema,
                &relations,
                None,
                &join_spec(JoinRelation::Neighbors(Some("zzz".into())))
            ),
            Err(ErrorCode::Malformed)
        );
        // Listed, but its rows live in another table: Unsupported.
        let cross = vec![RelationDescriptor {
            name: "parent".into(),
            kind: JoinRelation::Parent,
            target_table: Some("customer".into()),
        }];
        assert_eq!(
            validate_join(&schema, &cross, None, &join_spec(JoinRelation::Parent)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            validate_join(
                &schema,
                &relations,
                None,
                &join_spec(JoinRelation::Children)
            ),
            Ok(())
        );
    }

    /// `JOIN-FR-003`/`004`: the index nested loop over every relation
    /// kind — both orientations of a symmetric edge, a parentless left
    /// row producing nothing, both filters, both projections, `limit`.
    #[test]
    fn evaluate_join_walks_every_relation_kind_and_applies_filters_projections_and_limit() {
        let store = JoinFixture;
        assert_eq!(
            pairs(&evaluate_join(
                &store,
                &store,
                &join_spec(JoinRelation::Neighbors(None))
            )),
            vec![(1, 2), (2, 1), (2, 3), (3, 2)]
        );
        assert_eq!(
            pairs(&evaluate_join(
                &store,
                &store,
                &join_spec(JoinRelation::Neighbors(Some("r".into())))
            )),
            vec![(1, 2), (2, 1)]
        );
        assert_eq!(
            pairs(&evaluate_join(
                &store,
                &store,
                &join_spec(JoinRelation::Parent)
            )),
            vec![(2, 1), (3, 1)],
            "1 has no parent: no row for it"
        );
        assert_eq!(
            pairs(&evaluate_join(
                &store,
                &store,
                &join_spec(JoinRelation::Children)
            )),
            vec![(1, 2), (1, 3)]
        );
        let mut filtered = join_spec(JoinRelation::Children);
        filtered.right_filter = vec![Predicate {
            field: FIELD_A,
            op: CompareOp::Gt,
            value: ScanValue::U32(2),
        }];
        filtered.left = Selection::Fields(vec![]);
        let rows = evaluate_join(&store, &store, &filtered);
        assert_eq!(pairs(&rows), vec![(1, 3)]);
        assert!(rows[0].left.is_empty(), "left projection applied");
        assert_eq!(rows[0].right, vec![(FIELD_A, ScanValue::U32(3))]);
        let mut left_filtered = join_spec(JoinRelation::Neighbors(None));
        left_filtered.left_filter = vec![Predicate {
            field: FIELD_A,
            op: CompareOp::Eq,
            value: ScanValue::U32(2),
        }];
        assert_eq!(
            pairs(&evaluate_join(&store, &store, &left_filtered)),
            vec![(2, 1), (2, 3)]
        );
        let mut limited = join_spec(JoinRelation::Neighbors(None));
        limited.limit = Some(1);
        assert_eq!(
            pairs(&evaluate_join(&store, &store, &limited)),
            vec![(1, 2)]
        );
        limited.limit = Some(0);
        assert!(evaluate_join(&store, &store, &limited).is_empty());
    }

    /// `JOIN-FR-001`/`002`: `dispatch` routes both protocol-12 requests to
    /// the store, validating first.
    #[test]
    fn dispatch_answers_join_and_describe_relations() {
        match dispatch(&JoinFixture, Request::Join(join_spec(JoinRelation::Parent))) {
            Response::JoinedRows { rows } => assert_eq!(pairs(&rows), vec![(2, 1), (3, 1)]),
            other => panic!("expected JoinedRows, got {other:?}"),
        }
        assert_eq!(
            dispatch(
                &FixtureStore,
                Request::Join(join_spec(JoinRelation::Parent))
            ),
            err_response(ErrorCode::Malformed),
            "FixtureStore lists no relation"
        );
        match dispatch(&JoinFixture, Request::DescribeRelations) {
            Response::Relations { relations } => assert_eq!(relations.len(), 5),
            other => panic!("expected Relations, got {other:?}"),
        }
    }
    /// `INS-FR-006`/`007` (ADR-0046): `dispatch` maps the adapter's two
    /// normal outcomes to `Ok`/`Err { Duplicate }`, its refusals to the
    /// code it gave, and the trait's default to `Unsupported`.
    #[test]
    fn dispatch_answers_insert_per_the_adapter_and_unsupported_by_default() {
        struct InsertFixture;
        impl ConnectionStore for InsertFixture {
            fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
                FixtureStore.get(id)
            }
            fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
                FixtureStore.scan_all()
            }
            fn filter_eq(&self, f: FieldRef, v: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.filter_eq(f, v)
            }
            fn scan_field(&self, f: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
                FixtureStore.scan_field(f)
            }
            fn update_field(
                &self,
                id: RecordId,
                f: FieldRef,
                v: ScanValue,
            ) -> Result<bool, ErrorCode> {
                FixtureStore.update_field(id, f, v)
            }
            fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
                FixtureStore.parent(id)
            }
            fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.children(id)
            }
            fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.neighbors(id)
            }
            fn neighbors_by_relation(
                &self,
                id: RecordId,
                r: &str,
            ) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.neighbors_by_relation(id, r)
            }
            fn list_relation_kinds(&self) -> Vec<String> {
                FixtureStore.list_relation_kinds()
            }
            fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
                FixtureStore.validate_op(op)
            }
            fn describe(&self) -> DomainSchema {
                FixtureStore.describe()
            }
            fn apply_transaction(
                &self,
                u: &[TransactionOp],
                r: &[(RecordId, FieldRef, ScanValue)],
            ) -> Result<(), (usize, ErrorCode)> {
                FixtureStore.apply_transaction(u, r)
            }
            fn insert_record(
                &self,
                id: RecordId,
                fields: Vec<(FieldRef, ScanValue)>,
            ) -> Result<InsertOutcome, ErrorCode> {
                match fields.as_slice() {
                    [(FIELD_A, ScanValue::U32(_))] if id == RecordId::from_u128(1) => {
                        Ok(InsertOutcome::Duplicate)
                    }
                    [(FIELD_A, ScanValue::U32(_))] => Ok(InsertOutcome::Inserted),
                    [(FIELD_A, _)] => Err(ErrorCode::Malformed),
                    _ => Err(ErrorCode::UnknownField),
                }
            }
        }
        let fields = vec![(FIELD_A, ScanValue::U32(9))];
        let insert = |id: u128, fields: Vec<(FieldRef, ScanValue)>| Request::Insert {
            id: RecordId::from_u128(id),
            fields,
        };
        assert_eq!(
            dispatch(&InsertFixture, insert(2, fields.clone())),
            Response::Ok
        );
        assert_eq!(
            dispatch(&InsertFixture, insert(1, fields.clone())),
            err_response(ErrorCode::Duplicate)
        );
        assert_eq!(
            dispatch(
                &InsertFixture,
                insert(2, vec![(FIELD_A, ScanValue::Str("x".into()))])
            ),
            err_response(ErrorCode::Malformed)
        );
        assert_eq!(
            dispatch(&InsertFixture, insert(2, vec![(7, ScanValue::U32(1))])),
            err_response(ErrorCode::UnknownField)
        );
        assert_eq!(
            dispatch(&FixtureStore, insert(2, fields)),
            err_response(ErrorCode::Unsupported),
            "the trait's default"
        );
    }
    /// `LNK-FR-009`/`010` (ADR-0047): `dispatch` answers `Ok` for both
    /// link outcomes, passes a refusal's code through, and the trait's
    /// default is `Unsupported`.
    #[test]
    fn dispatch_answers_link_ok_for_both_outcomes_and_unsupported_by_default() {
        struct LinkFixture;
        impl ConnectionStore for LinkFixture {
            fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
                FixtureStore.get(id)
            }
            fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
                FixtureStore.scan_all()
            }
            fn filter_eq(&self, f: FieldRef, v: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.filter_eq(f, v)
            }
            fn scan_field(&self, f: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
                FixtureStore.scan_field(f)
            }
            fn update_field(
                &self,
                id: RecordId,
                f: FieldRef,
                v: ScanValue,
            ) -> Result<bool, ErrorCode> {
                FixtureStore.update_field(id, f, v)
            }
            fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
                FixtureStore.parent(id)
            }
            fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.children(id)
            }
            fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.neighbors(id)
            }
            fn neighbors_by_relation(
                &self,
                id: RecordId,
                r: &str,
            ) -> Result<Vec<RecordId>, ErrorCode> {
                FixtureStore.neighbors_by_relation(id, r)
            }
            fn list_relation_kinds(&self) -> Vec<String> {
                FixtureStore.list_relation_kinds()
            }
            fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
                FixtureStore.validate_op(op)
            }
            fn describe(&self) -> DomainSchema {
                FixtureStore.describe()
            }
            fn apply_transaction(
                &self,
                u: &[TransactionOp],
                r: &[(RecordId, FieldRef, ScanValue)],
            ) -> Result<(), (usize, ErrorCode)> {
                FixtureStore.apply_transaction(u, r)
            }
            fn link_records(
                &self,
                left: RecordId,
                right: RecordId,
                relation: &str,
            ) -> Result<LinkOutcome, ErrorCode> {
                match (left == right, relation) {
                    (true, _) => Err(ErrorCode::Malformed),
                    (false, "again") => Ok(LinkOutcome::AlreadyLinked),
                    (false, "knows") => Ok(LinkOutcome::Linked),
                    _ => Err(ErrorCode::RecordNotFound),
                }
            }
        }
        let link = |l: u128, r: u128, relation: &str| Request::Link {
            left: RecordId::from_u128(l),
            right: RecordId::from_u128(r),
            relation: relation.into(),
        };
        assert_eq!(dispatch(&LinkFixture, link(1, 2, "knows")), Response::Ok);
        assert_eq!(dispatch(&LinkFixture, link(1, 2, "again")), Response::Ok);
        assert_eq!(
            dispatch(&LinkFixture, link(1, 1, "knows")),
            err_response(ErrorCode::Malformed)
        );
        assert_eq!(
            dispatch(&LinkFixture, link(1, 9, "x")),
            err_response(ErrorCode::RecordNotFound)
        );
        assert_eq!(
            dispatch(&FixtureStore, link(1, 2, "knows")),
            err_response(ErrorCode::Unsupported),
            "the trait's default"
        );
    }

    /// `PAG-FR-002` (ADR-0055): rows sort by `(order_by value, id)`, the
    /// cursor is strict, `U32` and `I64` share one order, and `limit`
    /// truncates after the sort.
    #[test]
    fn page_rows_orders_by_value_then_id_strictly_after_the_cursor() {
        let id = RecordId::from_u128;
        let rows = vec![
            (id(3), vec![(0, ScanValue::I64(20))]),
            (id(1), vec![(0, ScanValue::I64(10))]),
            (id(2), vec![(0, ScanValue::I64(20))]),
            (id(4), vec![(0, ScanValue::I64(-5))]),
        ];
        let ids = |rows: Vec<(RecordId, Vec<(FieldRef, ScanValue)>)>| {
            rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(page_rows(rows.clone(), 0, None, 10)),
            vec![id(4), id(1), id(2), id(3)]
        );
        assert_eq!(ids(page_rows(rows.clone(), 0, None, 2)), vec![id(4), id(1)]);
        // Strictly after `(10, id 1)`: the two 20s, id-ordered.
        assert_eq!(
            ids(page_rows(
                rows.clone(),
                0,
                Some((ScanValue::I64(10), id(1))),
                10
            )),
            vec![id(2), id(3)]
        );
        // Strictly after `(20, id 2)`: only id 3 — the tie-break is the id.
        assert_eq!(
            ids(page_rows(
                rows.clone(),
                0,
                Some((ScanValue::I64(20), id(2))),
                10
            )),
            vec![id(3)]
        );
        // "Everything after 10": a cursor at the maximum id.
        assert_eq!(
            ids(page_rows(
                rows,
                0,
                Some((ScanValue::I64(10), RecordId::from_u128(u128::MAX))),
                10
            )),
            vec![id(2), id(3)]
        );
    }

    /// `page_ids` is the order's one home: the same cases as above on
    /// bare keys, plus the boundaries the selection path adds — a limit
    /// of zero, a limit past the end, and a limit that leaves exactly one
    /// key out (the `select_nth_unstable` pivot itself).
    #[test]
    fn page_ids_selects_the_same_page_as_a_full_sort_at_every_limit() {
        let id = RecordId::from_u128;
        let keys = vec![(id(3), 20), (id(1), 10), (id(2), 20), (id(4), -5)];
        let sorted = vec![id(4), id(1), id(2), id(3)];
        for limit in 0..=5 {
            assert_eq!(
                page_ids(keys.clone(), None, limit),
                sorted[..limit.min(4)].to_vec(),
                "limit {limit}"
            );
        }
        assert_eq!(
            page_ids(keys.clone(), Some((ScanValue::I64(10), id(1))), 1),
            vec![id(2)]
        );
        assert_eq!(
            page_ids(keys, Some((ScanValue::Str("x".into()), id(1))), 10),
            sorted,
            "a cursor of no orderable kind sorts before everything"
        );
    }
}
