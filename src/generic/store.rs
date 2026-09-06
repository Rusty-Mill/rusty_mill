//! The composable capability-wrapper layers — promoted from
//! `docs/design/GENERIC-SCHEMA-DESIGN.md` §2/§4. Each layer adds exactly
//! one capability on top of an inner store that already provides
//! `GetById`, and — since Rust has no trait delegation — must manually
//! forward every other capability trait its inner store already provides,
//! or that capability silently disappears once wrapped. That forwarding
//! tax is real (§4.5 of the design doc) and is paid explicitly below, not
//! hidden.
//!
//! `Flush` (added during promotion, not part of the original design doc)
//! is the one new capability this round adds: a store composed with
//! [`crate::generic::mmap_store::GenericMmapStore`] somewhere inside it
//! needs a way to force its durable field to disk through however many
//! wrapper layers sit on top — [`GenericProductionStore`](super::production::GenericProductionStore)
//! is generic over the whole composed stack and has no other way to reach
//! in. Every wrapper below forwards it, same as every other capability.

use super::edge_blob::{self, EdgeBlob};
use super::insert_log;
use super::query::{
    AllIds, Children, FilterEq, GetById, Insert, Link, MultiLink, Neighbors, Parent, Replace,
    ScanField, UpdateField,
};
use super::record_blob::{encode_tagged_image, parse_tagged_header, TAGGED_HEADER_LEN};
use super::traits::{ChildOf, IndexedField, Record, ScannableField, SchemaTag, SymmetricRelation};
use super::{InsertError, LinkError, LinkOutcome, NotFound, ReplaceError};
use crate::durability::record_blob::{EncodedRecordBlob, Fnv1a64};
use crate::durability::DurabilityError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// Owns the records — the base of every composed stack. The generic
/// analogue of `CanonicalStore`'s `HashMap<Uuid, DogRecord>`.
pub struct BaseStore<R: Record> {
    records: HashMap<R::Id, R>,
}

impl<R: Record + Clone> BaseStore<R> {
    pub fn new(records: Vec<R>) -> Self {
        Self {
            records: records.into_iter().map(|r| (r.id(), r)).collect(),
        }
    }
}

impl<R: Record + Clone> GetById<R> for BaseStore<R> {
    fn get(&self, id: R::Id) -> Option<R> {
        self.records.get(&id).cloned()
    }
}

/// `INS-FR-005`: the in-memory root of a composed stack accepts a record
/// it was not built with — a duplicate id is refused with nothing
/// written, the same rule the durable core applies.
impl<R: Record + Clone> Insert<R> for BaseStore<R> {
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        let id = record.id();
        if self.records.contains_key(&id) {
            return Err(InsertError::Duplicate(id));
        }
        self.records.insert(id, record);
        Ok(())
    }
}

/// `REP-FR-004`: the in-memory root swaps the record whole — an unknown
/// id is refused with nothing written, the same rule the durable core
/// applies.
impl<R: Record + Clone> Replace<R> for BaseStore<R> {
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        let id = record.id();
        if !self.records.contains_key(&id) {
            return Err(ReplaceError::NotFound(id));
        }
        self.records.insert(id, record);
        Ok(())
    }
}

/// A store with no durable field forwards `flush` as a no-op — `BaseStore`
/// is always purely in-memory (durability, when present, is added by
/// [`crate::generic::mmap_store::GenericMmapStore`] sitting somewhere
/// inside the composed stack, not by `BaseStore` itself).
impl<R: Record + Clone> Flush for BaseStore<R> {
    fn flush(&self) -> Result<(), DurabilityError> {
        Ok(())
    }
}

/// Forces a store's durable field(s) to physical disk — see this module's
/// docs for why this exists. A no-op for any layer/stack with nothing
/// durable inside it.
pub trait Flush {
    fn flush(&self) -> Result<(), DurabilityError>;
}

/// Adds one `FilterEq` capability over an inner store — the generic
/// analogue of `CanonicalStore`'s `breed_index`.
pub struct Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
{
    inner: S,
    index: HashMap<R::IndexValue, Vec<R::Id>>,
    _marker: PhantomData<Marker>,
}

impl<S, R, Marker> Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
{
    pub fn new(inner: S, records: &[R]) -> Self {
        let mut index: HashMap<R::IndexValue, Vec<R::Id>> = HashMap::new();
        for record in records {
            index
                .entry(record.indexed_value().clone())
                .or_default()
                .push(record.id());
        }
        Self {
            inner,
            index,
            _marker: PhantomData,
        }
    }
}

// `INS-FR-005`: the index bucket gains the id only after the inner store
// accepted the record; the value is taken before the record is moved.
impl<S, R, Marker> Insert<R> for Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
    S: Insert<R>,
{
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        let id = record.id();
        let value = record.indexed_value().clone();
        self.inner.insert(record)?;
        self.index.entry(value).or_default().push(id);
        Ok(())
    }
}

// `REP-FR-004`: the id moves from the old value's bucket to the new
// one's only after the inner store accepted the record; the old value
// is read from the inner store before the record is moved.
impl<S, R, Marker> Replace<R> for Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
    S: Replace<R> + GetById<R>,
{
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        let id = record.id();
        let old_value = self
            .inner
            .get(id)
            .map(|old| old.indexed_value().clone())
            .ok_or(ReplaceError::NotFound(id))?;
        let new_value = record.indexed_value().clone();
        self.inner.replace(record)?;
        if old_value != new_value {
            if let Some(bucket) = self.index.get_mut(&old_value) {
                bucket.retain(|other| *other != id);
                if bucket.is_empty() {
                    self.index.remove(&old_value);
                }
            }
            self.index.entry(new_value).or_default().push(id);
        }
        Ok(())
    }
}

impl<S, R, Marker> FilterEq<R, Marker> for Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
{
    fn filter_eq(&self, value: &R::IndexValue) -> Vec<R::Id> {
        self.index.get(value).cloned().unwrap_or_default()
    }
}

// Forwarding impl: without this, `Indexed<S, ..>` doesn't expose
// `GetById` even though its inner store already does.
impl<S, R, Marker> GetById<R> for Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
    S: GetById<R>,
{
    fn get(&self, id: R::Id) -> Option<R> {
        self.inner.get(id)
    }
}

impl<S, R, Marker> Flush for Indexed<S, R, Marker>
where
    R: IndexedField<Marker>,
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

/// Adds one `ScanField`/`UpdateField` capability over an inner store — the
/// generic analogue of `CanonicalCachedStore`'s `age_cache` +
/// `position_index`, entirely in-memory (see
/// [`crate::generic::mmap_store::GenericMmapStore`] for the durable
/// analogue of this same shape).
pub struct Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
{
    inner: S,
    position_index: HashMap<R::Id, usize>,
    cache: Vec<R::ScanValue>,
    _marker: PhantomData<Marker>,
}

impl<S, R, Marker> Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
{
    /// Access to the inner store — needed by domain-specific, concrete
    /// (non-generic) forwarding impls like `forward_scannable_pairs!`'s
    /// generated pairs (see this file's module docs on why that can't be
    /// one generic impl). `inner`/`inner_mut` rather than a public field:
    /// keeps the rest of `Scanned`'s representation (`position_index`,
    /// `cache`) private.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn new(inner: S, records: &[R]) -> Self {
        let mut position_index = HashMap::with_capacity(records.len());
        let mut cache = Vec::with_capacity(records.len());
        for (position, record) in records.iter().enumerate() {
            position_index.insert(record.id(), position);
            cache.push(record.scannable_value());
        }
        Self {
            inner,
            position_index,
            cache,
            _marker: PhantomData,
        }
    }
}

// `INS-FR-005`: a new cache position for the record, after the inner
// store accepted it.
impl<S, R, Marker> Insert<R> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
    S: Insert<R>,
{
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        let id = record.id();
        let value = record.scannable_value();
        self.inner.insert(record)?;
        self.position_index.insert(id, self.cache.len());
        self.cache.push(value);
        Ok(())
    }
}

// `REP-FR-004`: the cached value is rewritten in place after the inner
// store accepted the record.
impl<S, R, Marker> Replace<R> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
    S: Replace<R>,
{
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        let id = record.id();
        let value = record.scannable_value();
        self.inner.replace(record)?;
        let position = *self
            .position_index
            .get(&id)
            .ok_or(ReplaceError::NotFound(id))?;
        self.cache[position] = value;
        Ok(())
    }
}

impl<S, R, Marker> ScanField<R, Marker> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
{
    fn scan(&self) -> Vec<R::ScanValue> {
        self.cache.clone()
    }
}

impl<S, R, Marker> UpdateField<R, Marker> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
{
    fn update(&mut self, id: R::Id, value: R::ScanValue) -> Result<(), NotFound<R::Id>> {
        let position = *self.position_index.get(&id).ok_or(NotFound(id))?;
        self.cache[position] = value;
        Ok(())
    }
}

// Forwarding impl: `Scanned<S, ..>` re-exposing `GetById` from its inner
// store — write-through consistent with `UpdateField::update`, unlike an
// earlier version of this impl (see the fix note below).
impl<S, R, Marker> GetById<R> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
    S: GetById<R>,
{
    /// Patches the record `inner.get` returns with this layer's own live
    /// cached value before returning it — not a blind forward. An earlier
    /// version of this impl returned `self.inner.get(id)` unmodified,
    /// which meant a `Scanned` layer's own `UpdateField::update` (which
    /// only ever writes into `self.cache`, never down into `BaseStore`'s
    /// records map several layers below) was invisible to `get`. Reusing
    /// `set_scannable_value` (added for [`super::mmap_store::GenericMmapStore`]'s
    /// analogous gap) fixes it here too, but the mechanism is different: a
    /// single hand-fused struct like `GenericMmapStore` merges two views
    /// it owns directly, while `Scanned` — a separate struct layered on
    /// top of whatever owns the record — has no way to reach down into
    /// that owner's storage, so it patches on the way *up* through `get`
    /// instead. When multiple `Scanned` layers stack (e.g. `Order`'s
    /// `Amount`/`CreatedAt`/`DiscountCents`), each one patches only its
    /// own field as the call unwinds, so the record is fully consistent
    /// by the time it reaches the outermost caller — no change needed in
    /// `Indexed`/`Symmetric`/`Reversed`, none of which own any
    /// `ScannableField` data to patch.
    fn get(&self, id: R::Id) -> Option<R> {
        let mut record = self.inner.get(id)?;
        if let Some(&position) = self.position_index.get(&id) {
            record.set_scannable_value(self.cache[position]);
        }
        Some(record)
    }
}

impl<S, R, Marker> Flush for Scanned<S, R, Marker>
where
    R: ScannableField<Marker>,
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

// Forwarding impl: `Scanned<S, ..>` re-exposing `FilterEq` from its inner
// store (e.g. `Scanned<Indexed<BaseStore<R>, R, Breed>, R, Age>` still
// needs to answer `filter_eq` on `Breed`) — note the two distinct marker
// type parameters (`IndexMarker` for the field `FilterEq` is being
// forwarded for, `Marker` for the field `Scanned` itself owns).
impl<S, R, Marker, IndexMarker> FilterEq<R, IndexMarker> for Scanned<S, R, Marker>
where
    R: ScannableField<Marker> + IndexedField<IndexMarker>,
    S: FilterEq<R, IndexMarker>,
{
    // Bare `R::IndexValue` is unambiguous here even though `R` is bound
    // by both `ScannableField<Marker>` and `IndexedField<IndexMarker>` —
    // see `traits.rs`'s module docs for the associated-type rename this
    // relies on.
    fn filter_eq(&self, value: &R::IndexValue) -> Vec<R::Id> {
        self.inner.filter_eq(value)
    }
}

// Forwarding impl, generated for every ordered pair of `ScannableField`
// markers a record declares: `Scanned<S, ..>` needs to re-expose
// `ScanField`/`UpdateField` for a field *other* than its own once a
// record has more than one scannable field (`Order` has `Amount`,
// `CreatedAt`, `DiscountCents`) and the layers stack.
//
// **This is NOT expressible as one generic impl over "any other marker,"
// unlike every other forwarding impl in this file** — a first attempt,
// `impl<S, R, Marker, OtherMarker> ScanField<R, OtherMarker> for
// Scanned<S, R, Marker>`, doesn't even reach an associated-type ambiguity:
// it fails to compile at all, with `E0119: conflicting implementations of
// trait ScanField<_, _> for type Scanned<_, _, _>`. Rust's coherence
// checker has no way to know `OtherMarker != Marker`, so that impl and
// `Scanned`'s own direct `ScanField<R, Marker>` impl above are seen as
// *potentially* the same impl (the case `OtherMarker = Marker`) — a real
// orphan/overlap violation, not a naming or inference problem, and not
// fixable by disambiguating `R::ScanValue` (fully-qualified syntax
// doesn't touch impl coherence at all). Stable Rust has no negative bound
// expressing "these two type parameters are unequal," so there is no way
// to write this as one generic impl.
//
// The only way to make this compile: one concrete, non-generic impl per
// *ordered pair* of markers — the tax for N scannable fields on one
// record is O(N²), not O(N). What's not unavoidable is a human hand-
// writing and maintaining each pair — the macro below generates them from
// a field list, so a new scannable field costs one macro-invocation
// entry, not new hand-written impls. See `order_customer.rs`'s
// invocation.
#[macro_export]
macro_rules! forward_scannable_pairs {
    // Entry points, one per layer kind. Each layer that owns one
    // scannable field and forwards the others needs its own set of pairs;
    // the layer is spelled by name (not as a path) because a `$layer:path`
    // fragment cannot be followed by `<`, and the two layers this crate
    // has are enumerated so the invocation site doesn't need to know
    // where they live. Adding a third layer means adding one arm here.
    // `for Layer; $record; Marker1: Value1, Marker2: Value2, ...`
    //
    // These arms come before the bare `$record:ty` one on purpose: `for`
    // also opens a `for<'a> ...` type, so a `ty` fragment matcher tried
    // first would fail *hard* on `for Scanned` instead of falling through.
    (for Scanned; $record:ty; $($marker:ident : $value:ty),+ $(,)?) => {
        $crate::forward_scannable_pairs!(
            @rotate [$crate::generic::store::Scanned] $record; []; [$($marker : $value),+]
        );
    };
    (for MmapScanned; $record:ty; $($marker:ident : $value:ty),+ $(,)?) => {
        $crate::forward_scannable_pairs!(
            @rotate [$crate::generic::mmap_scanned::MmapScanned] $record; []; [$($marker : $value),+]
        );
    };

    // The original entry point: a record type and its `ScannableField`
    // markers, each with its concrete `ScanValue` type (needed because
    // macro_rules! can't look up an associated type). Generates the pairs
    // for the in-memory `Scanned` layer.
    // `$record; Marker1: Value1, Marker2: Value2, ...`
    ($record:ty; $($marker:ident : $value:ty),+ $(,)?) => {
        $crate::forward_scannable_pairs!(for Scanned; $record; $($marker : $value),+);
    };

    // Peels `$owner` off the front of the not-yet-processed list, emits
    // its pairs against everything else (`$prefix` — already-processed
    // owners, still needed as forwarding targets — plus `$rest`, the
    // still-to-be-processed owners), then recurses with `$owner` moved
    // into `$prefix`. The standard "rotating accumulator" trick for
    // generating all off-diagonal pairs from a list in `macro_rules!` —
    // chosen specifically because `macro_rules!` cannot compare two
    // matched fragments for equality (there is no `$a == $b` for
    // `:ident`/`:ty` matchers), so the diagonal (`owner == owner`) has to
    // be excluded *structurally*, by construction, rather than by a
    // runtime-style check. `$owner` never appears in the "everything
    // else" list at the point it's used, by construction — it's been
    // removed from `$rest` and not yet added to `$prefix`.
    // The layer's path travels as one bracketed token tree (`$layer:tt`,
    // e.g. `[$crate::generic::store::Scanned]`) through the internal
    // arms — a single `tt` so it can sit inside the `@pairs` repetition
    // without a depth mismatch — and is only opened up by the payload arm,
    // which splices it in front of `<S, $record, $owner>`.
    (@rotate $layer:tt $record:ty; [$($prefix:ident : $prefix_value:ty),*]; [$owner:ident : $owner_value:ty $(, $rest:ident : $rest_value:ty)*]) => {
        $crate::forward_scannable_pairs!(
            @pairs $layer $record; $owner : $owner_value;
            [$($prefix : $prefix_value,)* $($rest : $rest_value),*]
        );
        $crate::forward_scannable_pairs!(
            @rotate $layer $record; [$($prefix : $prefix_value,)* $owner : $owner_value]; [$($rest : $rest_value),*]
        );
    };
    // Base case: nothing left to peel off — every owner has had its
    // pairs generated.
    (@rotate $layer:tt $record:ty; [$($prefix:ident : $prefix_value:ty),*]; []) => {};

    // Emits one `@impl_pair` per marker in the "everything else" list,
    // for the fixed `$owner`.
    (@pairs $layer:tt $record:ty; $owner:ident : $owner_value:ty; [$($forwarded:ident : $forwarded_value:ty),* $(,)?]) => {
        $(
            $crate::forward_scannable_pairs!(@impl_pair $layer $record; $owner : $owner_value; $forwarded : $forwarded_value);
        )*
    };

    // The actual payload: one concrete `ScanField`/`UpdateField` pair.
    // Both layers expose the same `inner`/`inner_mut` accessors, which is
    // all the generated body relies on.
    (@impl_pair [$($layer:tt)*] $record:ty; $owner:ident : $owner_value:ty; $forwarded:ident : $forwarded_value:ty) => {
        impl<S> $crate::generic::query::ScanField<$record, $forwarded>
            for $($layer)*<S, $record, $owner>
        where
            S: $crate::generic::query::ScanField<$record, $forwarded>,
        {
            fn scan(&self) -> Vec<$forwarded_value> {
                $($layer)*::inner(self).scan()
            }
        }

        impl<S> $crate::generic::query::UpdateField<$record, $forwarded>
            for $($layer)*<S, $record, $owner>
        where
            S: $crate::generic::query::UpdateField<$record, $forwarded>,
        {
            fn update(
                &mut self,
                id: <$record as $crate::generic::traits::Record>::Id,
                value: $forwarded_value,
            ) -> Result<
                (),
                $crate::generic::NotFound<<$record as $crate::generic::traits::Record>::Id>,
            > {
                $($layer)*::inner_mut(self).update(id, value)
            }
        }
    };
}

/// Adds one `Neighbors` capability over an inner store — the generic
/// analogue of `CanonicalCachedStore`'s `adjacency_index`.
pub struct Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
{
    inner: S,
    adjacency: HashMap<R::Id, Vec<R::Id>>,
    /// Where this layer's edge blob lives — `Some` from `create`/`open`/
    /// `open_portable`, `None` from `new` (`LNK-FR-002`, ADR-0047): a
    /// runtime [`Link`] is logged beside the blob when there is one, and
    /// stays in memory when there is not.
    edges_path: Option<PathBuf>,
    _marker: PhantomData<Marker>,
}

impl<S, R, Marker> Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
{
    pub fn new(inner: S, edges: &[(R::Id, R::Id)]) -> Self {
        let mut adjacency: HashMap<R::Id, Vec<R::Id>> = HashMap::new();
        for &(a, b) in edges {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
        Self {
            inner,
            adjacency,
            edges_path: None,
            _marker: PhantomData,
        }
    }

    fn with_path(mut self, edges_path: &Path) -> Self {
        self.edges_path = Some(edges_path.to_path_buf());
        self
    }

    fn has_edge(&self, a: R::Id, b: R::Id) -> bool {
        self.adjacency.get(&a).is_some_and(|v| v.contains(&b))
    }
}

/// `LNK-FR-004`: `edges` followed by every logged pair not already
/// present in either orientation, in log order — the same skip that
/// makes a fold idempotent for records. Shared by both relation layers.
fn merge_edge_lists<Id: Copy + Eq + std::hash::Hash>(
    mut edges: Vec<(Id, Id)>,
    logged: Vec<(Id, Id)>,
) -> Vec<(Id, Id)> {
    if logged.is_empty() {
        return edges;
    }
    let mut seen: std::collections::HashSet<(Id, Id)> = std::collections::HashSet::new();
    for &(a, b) in &edges {
        seen.insert((a, b));
        seen.insert((b, a));
    }
    for (a, b) in logged {
        if seen.insert((a, b)) {
            seen.insert((b, a));
            edges.push((a, b));
        }
    }
    edges
}

/// The edge list [`Symmetric::read_portable_edges`] returns: the pairs
/// [`Symmetric::create`] was given, in the order it was given them.
type PortableEdges<R> = Vec<(<R as Record>::Id, <R as Record>::Id)>;

/// File portability for the edge list — `STORAGE-016`, per
/// `docs/design/SYMMETRIC-EDGE-PORTABILITY-DESIGN.md` (Accepted) and
/// ADR-0018. [`Symmetric::new`] builds its adjacency from a caller-supplied
/// slice and persists nothing, which is right for the in-memory stacks
/// but leaves a durable stack's symmetric edges living only in the
/// caller's hands: `GenericMmapStore` carries its records in
/// `<path>.records`, so a directory holding that pair could rebuild the
/// core store but not the `Symmetric` layer above it. This block adds the
/// `create`/`open`/`open_portable` triple that closes the gap — the edge
/// list, as given and in caller order, in a companion "edge blob" at a
/// caller-supplied `edges_path` (see `crate::generic::edge_blob`, and
/// `edges_path` there for the `<path>.edges` single-relation convention
/// the domain helpers use).
///
/// Bounded `R::Id: Serialize + DeserializeOwned` and `R: SchemaTag` here
/// and here only (`SYMPORT-FR-006`, `SCHTAG-FR-002`): `new`, the
/// `Neighbors` impl, and every forwarding impl keep their bounds exactly,
/// and no existing call site changes. The blob's header carries
/// `R::SCHEMA_TAG`'s hash, so an edge blob written for one record type is
/// refused, by name, when opened as a relation over another with the
/// same `Id` type (`SCHTAG-FR-001`). A durable stack assembled with `new`
/// rather than `create`/`open` writes no blob and is not portable —
/// closed by convention in the domain helpers, not by type (the design
/// doc's stated non-goal).
impl<S, R, Marker> Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker> + SchemaTag,
    R::Id: Serialize + DeserializeOwned,
{
    /// Write `edges` to `edges_path`, then [`Self::new`] (`SYMPORT-FR-001`).
    /// Always writes: this is the constructor for a fresh stack, the
    /// analogue of `GenericMmapStore::create`. `inner` is received already
    /// built; if the blob write fails it is dropped with the error, and
    /// its own files (if any) remain valid for a retried `create` or an
    /// [`Self::open`].
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::Serde`] if `edges` can't be serialized,
    /// [`DurabilityError::Io`] if the blob can't be written.
    pub fn create(
        inner: S,
        edges: &[(R::Id, R::Id)],
        edges_path: &Path,
    ) -> Result<Self, DurabilityError> {
        // `LNK-FR-004`: a fresh layer starts with no edge log.
        insert_log::clear(&insert_log::log_path(edges_path))?;
        EdgeBlob::new(edges, R::SCHEMA_TAG)
            .encode()?
            .write(edges_path)?;
        Ok(Self::new(inner, edges).with_path(edges_path))
    }

    /// [`Self::new`] over the caller's `edges`, keeping the blob at
    /// `edges_path` current with them (`SYMPORT-FR-004`): the header's
    /// fingerprint is compared against `edges` first, and only if they
    /// differ — a changed or reordered edge list, a missing, short,
    /// foreign, wrong-version, or wrong-tag file, a directory written
    /// before the edge blob existed or before it carried a tag
    /// (`SCHTAG-FR-004`) — is the blob re-encoded and rewritten. The
    /// common reopen with the same edges never writes. The adjacency is
    /// always built from `edges`, never from the blob, on this path.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::Serde`] if a stale blob's edges can't
    /// be serialized, [`DurabilityError::Io`] if it can't be rewritten.
    pub fn open(
        inner: S,
        edges: &[(R::Id, R::Id)],
        edges_path: &Path,
    ) -> Result<Self, DurabilityError> {
        // `LNK-FR-004`: fold the edge log in first, so the rewrite below
        // covers every runtime link and the log can then be cleared.
        let log = insert_log::log_path(edges_path);
        let edges = merge_edge_lists(edges.to_vec(), insert_log::read_items(&log, R::SCHEMA_TAG)?);
        let blob = EdgeBlob::new(&edges, R::SCHEMA_TAG);
        if !blob.is_current_at(edges_path) {
            blob.encode()?.write(edges_path)?;
        }
        insert_log::clear(&log)?;
        Ok(Self::new(inner, &edges).with_path(edges_path))
    }

    /// Add one edge at runtime — `LNK-FR-002` (ADR-0047): both ids must
    /// have a record in the store beneath, `a != b`, and an edge already
    /// present in either orientation is [`LinkOutcome::AlreadyLinked`]
    /// with nothing written. Otherwise the pair is appended to the edge
    /// log at `<edges_path>.inserts` and `sync_data`ed *before* the
    /// adjacency gains both directions, so `Linked` means durable; the
    /// next [`Self::open`] folds the log into the blob. A layer built
    /// with [`Self::new`] has no path and links in memory only.
    ///
    /// # Errors
    ///
    /// [`LinkError::UnknownRecord`] (the first missing endpoint, `a`
    /// before `b`), [`LinkError::SelfLoop`], or
    /// [`LinkError::Durability`] if the log append fails.
    pub fn link(&mut self, a: R::Id, b: R::Id) -> Result<LinkOutcome, LinkError<R::Id>>
    where
        S: GetById<R>,
    {
        if self.inner.get(a).is_none() {
            return Err(LinkError::UnknownRecord(a));
        }
        if self.inner.get(b).is_none() {
            return Err(LinkError::UnknownRecord(b));
        }
        if a == b {
            return Err(LinkError::SelfLoop(a));
        }
        if self.has_edge(a, b) {
            return Ok(LinkOutcome::AlreadyLinked);
        }
        if let Some(path) = &self.edges_path {
            insert_log::append_item(&insert_log::log_path(path), R::SCHEMA_TAG, &(a, b))?;
        }
        self.adjacency.entry(a).or_default().push(b);
        self.adjacency.entry(b).or_default().push(a);
        Ok(LinkOutcome::Linked)
    }

    /// The edge list persisted at `edges_path`, in the order it was
    /// written (`SYMPORT-FR-002`), followed by every runtime-linked pair
    /// its edge log holds that the blob does not (`LNK-FR-004`). Reads
    /// only the blob and the log; never writes, never touches the inner
    /// store.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::RecordBlobUnreadable`], naming
    /// `edges_path`, if the blob is missing, isn't one (wrong magic — a
    /// `GENBLOB\0` or `DOGBLOB\0` file included), was written by an
    /// incompatible version, carries another record type's schema tag
    /// (`SCHTAG-FR-003`), doesn't decode, or doesn't match its own header
    /// fingerprint (`SYMPORT-FR-005`).
    pub fn read_portable_edges(edges_path: &Path) -> Result<PortableEdges<R>, DurabilityError> {
        let persisted = edge_blob::read(edges_path, R::SCHEMA_TAG)?;
        let logged = insert_log::read_items(&insert_log::log_path(edges_path), R::SCHEMA_TAG)?;
        Ok(merge_edge_lists(persisted, logged))
    }

    /// Rebuild the layer from its blob alone — exactly
    /// `Ok(Self::new(inner, &Self::read_portable_edges(edges_path)?))`
    /// (`SYMPORT-FR-003`). Because the blob preserves edge order and
    /// [`Self::new`] pushes adjacency entries in edge order, `neighbors`
    /// returns the same sequences the original layer did
    /// (`SYMPORT-FR-008`). Never writes.
    ///
    /// # Errors
    ///
    /// Everything [`Self::read_portable_edges`] can return.
    pub fn open_portable(inner: S, edges_path: &Path) -> Result<Self, DurabilityError> {
        // Through `open`, since `LNK-FR-004`: a non-empty edge log is
        // folded into the blob and cleared; with no log, `open`'s
        // currency check passes and nothing is written, as before.
        Self::open(inner, &Self::read_portable_edges(edges_path)?, edges_path)
    }
}

// `LNK-FR-002`: the inherent [`Symmetric::link`], reached through the
// trait every layer above forwards.
impl<S, R, Marker> Link<R, Marker> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker> + SchemaTag,
    R::Id: Serialize + DeserializeOwned,
    S: GetById<R>,
{
    fn link(&mut self, a: R::Id, b: R::Id) -> Result<LinkOutcome, LinkError<R::Id>> {
        Symmetric::link(self, a, b)
    }
}

impl<S, R, Marker> Neighbors<R, Marker> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
{
    fn neighbors(&self, id: R::Id) -> Vec<R::Id> {
        self.adjacency.get(&id).cloned().unwrap_or_default()
    }
}

// `INS-FR-005`: a record inserted at runtime has no edges — `neighbors`
// answers `[]` for it — and the edge blob is untouched; relation
// insertion is the next round in this line (ADR-0046's Non-goals).
impl<S, R, Marker> Insert<R> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
    S: Insert<R>,
{
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        self.inner.insert(record)
    }
}

// `REP-FR-004`: edges are not part of a record — a replaced record keeps
// every neighbor it had; the edge blob is untouched.
impl<S, R, Marker> Replace<R> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
    S: Replace<R>,
{
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        self.inner.replace(record)
    }
}

// Forwarding impl: `Symmetric<S, ..>` re-exposing `GetById`.
impl<S, R, Marker> GetById<R> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
    S: GetById<R>,
{
    fn get(&self, id: R::Id) -> Option<R> {
        self.inner.get(id)
    }
}

// Forwarding impl: `Symmetric<S, ..>` re-exposing `AllIds` (`SQL-FR-005`,
// ADR-0034) — the identical shape every other capability is forwarded
// through this layer in.
impl<S, R, Marker> AllIds<R> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
    S: AllIds<R>,
{
    fn all_ids(&self) -> Vec<R::Id> {
        self.inner.all_ids()
    }
}

impl<S, R, Marker> Flush for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker>,
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

// Forwarding impl: `Symmetric<S, ..>` re-exposing `FilterEq`.
impl<S, R, Marker, IndexMarker> FilterEq<R, IndexMarker> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker> + IndexedField<IndexMarker>,
    S: FilterEq<R, IndexMarker>,
{
    fn filter_eq(&self, value: &R::IndexValue) -> Vec<R::Id> {
        self.inner.filter_eq(value)
    }
}

// Forwarding impl: `Symmetric<S, ..>` re-exposing `ScanField`.
impl<S, R, Marker, ScanMarker> ScanField<R, ScanMarker> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker> + ScannableField<ScanMarker>,
    S: ScanField<R, ScanMarker>,
{
    fn scan(&self) -> Vec<R::ScanValue> {
        self.inner.scan()
    }
}

// Forwarding impl: `Symmetric<S, ..>` re-exposing `UpdateField`.
impl<S, R, Marker, ScanMarker> UpdateField<R, ScanMarker> for Symmetric<S, R, Marker>
where
    R: SymmetricRelation<Marker> + ScannableField<ScanMarker>,
    S: UpdateField<R, ScanMarker>,
{
    fn update(&mut self, id: R::Id, value: R::ScanValue) -> Result<(), NotFound<R::Id>> {
        self.inner.update(id, value)
    }
}

/// More than one named `SymmetricRelation` over the same record type `R`
/// — `ENT2-FR-002`/`003` (ADR-0039). See [`super::query::MultiNeighbors`]'s
/// own doc comment for why this is a genuinely new primitive, not a
/// `Symmetric`-forwarding fix: the generic-forwarding-impl shape
/// `Reversed`'s own `Neighbors` fix (`FR-012`) used was tried first and
/// confirmed, directly with `rustc`, to conflict (`E0119`) with
/// `Symmetric`'s own existing direct `Neighbors` impl — a conflict
/// `Reversed` never has, since its own relation is `ChildOf`, a
/// different trait. `MultiSymmetric` sidesteps this by keying relations
/// at runtime (a `String` label) rather than at the type level (a
/// `Marker`), matching `Request::NeighborsByRelation`'s own wire shape
/// exactly. Each relation's edge list is independently durable
/// (`STORAGE-016`'s own `EdgeBlob` mechanism, reused directly — one blob
/// per label, at `<path>.<label>.edges`), the same real portability
/// `Symmetric` already has, generalized to more than one label.
/// Whether `label` may name a relation — `LNK-FR-001`/`005` (ADR-0047).
/// A label becomes part of a file name (`<path>.<label>.edges`), so it
/// is 1 to 64 bytes of ASCII letters, digits, `_`, or `-`, not starting
/// with `-`: no path separators, no `..`, no whitespace, nothing that
/// could steer a file name. Checked at the server boundary *and* at the
/// layer, so no caller can bypass it. The consumer's own labels keep
/// case and may contain spaces (`ADR-0042` F3); a bridge maps those
/// (`works with` → `works_with`) — stated in the design's Non-goals.
pub fn valid_relation_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

/// The label manifest — `<path>.relations`, `LNK-FR-006` (ADR-0047): the
/// labels a [`MultiSymmetric`] has ever had an edge blob for, so a
/// portable reopen discovers a label created at runtime without the
/// caller naming it. A tagged blob (`GENLABL\0`, version 1, the record
/// type's tag, the body fingerprinted) of `Vec<String>` in first-seen
/// order; a missing file is the empty set, so every directory written
/// before this round reopens exactly as before.
mod label_manifest {
    use super::*;

    const MAGIC: [u8; 8] = *b"GENLABL\0";
    const VERSION: u32 = 1;

    pub(super) fn path(base: &Path) -> PathBuf {
        let mut name = base.as_os_str().to_owned();
        name.push(".relations");
        PathBuf::from(name)
    }

    pub(super) fn read(base: &Path, tag: &str) -> Result<Vec<String>, DurabilityError> {
        let manifest = path(base);
        let unreadable = |cause: String| DurabilityError::RecordBlobUnreadable {
            path: manifest.clone(),
            cause,
        };
        let bytes = match std::fs::read(&manifest) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let claimed = parse_tagged_header(&bytes, &MAGIC, VERSION, tag).map_err(unreadable)?;
        let body = &bytes[TAGGED_HEADER_LEN..];
        let mut hash = Fnv1a64::new();
        hash.update(body);
        if hash.finish() != claimed {
            return Err(unreadable("fingerprint mismatch".to_owned()));
        }
        crate::codec::decode(body).map_err(|e| unreadable(format!("body does not decode: {e}")))
    }

    pub(super) fn write(base: &Path, tag: &str, labels: &[String]) -> Result<(), DurabilityError> {
        let body = crate::codec::encode(labels)?;
        let mut hash = Fnv1a64::new();
        hash.update(&body);
        EncodedRecordBlob {
            image: encode_tagged_image(&MAGIC, VERSION, hash.finish(), tag, &body),
        }
        .write(&path(base))
    }
}

pub struct MultiSymmetric<S, R: Record> {
    inner: S,
    adjacency: HashMap<String, HashMap<R::Id, Vec<R::Id>>>,
    /// The base path its blobs, logs, and manifest hang off — `Some`
    /// from `create`/`open`/`open_portable`, `None` from `new`
    /// (`LNK-FR-005`).
    base_path: Option<PathBuf>,
}

/// A named list of relations, each a label paired with its own edge
/// list — the shape [`MultiSymmetric::new`]/[`MultiSymmetric::create`]/
/// [`MultiSymmetric::open`] all take. Factored out purely to keep those
/// signatures under clippy's `type_complexity` threshold; carries no
/// behavior of its own.
type LabeledRelations<R> = [(String, Vec<(<R as Record>::Id, <R as Record>::Id)>)];

/// The owned form of [`LabeledRelations`] — what the manifest union
/// (`LNK-FR-006`) builds before handing it to [`MultiSymmetric::new`].
type OwnedLabeledRelations<R> = Vec<(String, Vec<(<R as Record>::Id, <R as Record>::Id)>)>;

/// The suffix pattern `<path>.<label>.edges` uses — parallels
/// [`super::edge_blob::edges_path`]'s own single-relation `<path>.edges`
/// convention, generalized to more than one label sharing a base path.
fn labeled_edges_path(base: &Path, label: &str) -> std::path::PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(".");
    name.push(label);
    name.push(".edges");
    std::path::PathBuf::from(name)
}

impl<S, R: Record> MultiSymmetric<S, R> {
    /// Build the adjacency maps directly from `relations` — the
    /// `Symmetric::new` analogue, generalized to a labeled list instead
    /// of one slice. Writes nothing; see [`Self::create`]/[`Self::open`]
    /// for the durable constructors.
    pub fn new(inner: S, relations: &LabeledRelations<R>) -> Self {
        let mut adjacency: HashMap<String, HashMap<R::Id, Vec<R::Id>>> = HashMap::new();
        for (label, edges) in relations {
            let mut map: HashMap<R::Id, Vec<R::Id>> = HashMap::new();
            for &(a, b) in edges {
                map.entry(a).or_default().push(b);
                map.entry(b).or_default().push(a);
            }
            adjacency.insert(label.clone(), map);
        }
        Self {
            inner,
            adjacency,
            base_path: None,
        }
    }

    fn with_path(mut self, base: &Path) -> Self {
        self.base_path = Some(base.to_path_buf());
        self
    }

    /// `LNK-FR-006`/`007`: `relations` plus every manifest label it does
    /// not name (with an empty edge list), in caller-then-manifest order.
    fn with_manifest_labels(
        relations: &LabeledRelations<R>,
        manifest: Vec<String>,
    ) -> OwnedLabeledRelations<R> {
        let mut all: OwnedLabeledRelations<R> = relations.to_vec();
        for label in manifest {
            if !all.iter().any(|(l, _)| *l == label) {
                all.push((label, Vec::new()));
            }
        }
        all
    }
}

impl<S, R> MultiSymmetric<S, R>
where
    R: Record + SchemaTag,
    R::Id: Serialize + DeserializeOwned,
{
    /// Write every relation's edge blob to `<path>.<label>.edges`, then
    /// [`Self::new`] — the `Symmetric::create` analogue. Always writes,
    /// the constructor for a fresh stack.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::Serde`] if an edge list can't be
    /// serialized, [`DurabilityError::Io`] if a blob can't be written.
    pub fn create(
        inner: S,
        relations: &LabeledRelations<R>,
        path: &Path,
    ) -> Result<Self, DurabilityError> {
        for (label, edges) in relations {
            let blob_path = labeled_edges_path(path, label);
            insert_log::clear(&insert_log::log_path(&blob_path))?;
            EdgeBlob::new(edges, R::SCHEMA_TAG)
                .encode()?
                .write(&blob_path)?;
        }
        let labels: Vec<String> = relations.iter().map(|(l, _)| l.clone()).collect();
        label_manifest::write(path, R::SCHEMA_TAG, &labels)?;
        Ok(Self::new(inner, relations).with_path(path))
    }

    /// [`Self::new`] over `relations`, rewriting only the labels whose
    /// blob is stale — the `Symmetric::open` analogue, per label.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::Serde`] if a stale blob's edges can't
    /// be serialized, [`DurabilityError::Io`] if it can't be rewritten.
    pub fn open(
        inner: S,
        relations: &LabeledRelations<R>,
        path: &Path,
    ) -> Result<Self, DurabilityError> {
        // `LNK-FR-006`/`007`: the caller's labels plus the manifest's,
        // each folding its own edge log before the stale check.
        let manifest = label_manifest::read(path, R::SCHEMA_TAG)?;
        let mut all = Self::with_manifest_labels(relations, manifest.clone());
        for (label, edges) in &mut all {
            let blob_path = labeled_edges_path(path, label);
            let log = insert_log::log_path(&blob_path);
            *edges = merge_edge_lists(
                std::mem::take(edges),
                insert_log::read_items(&log, R::SCHEMA_TAG)?,
            );
            let blob = EdgeBlob::new(edges, R::SCHEMA_TAG);
            if !blob.is_current_at(&blob_path) {
                blob.encode()?.write(&blob_path)?;
            }
            insert_log::clear(&log)?;
        }
        let labels: Vec<String> = all.iter().map(|(l, _)| l.clone()).collect();
        if labels != manifest {
            label_manifest::write(path, R::SCHEMA_TAG, &labels)?;
        }
        Ok(Self::new(inner, &all).with_path(path))
    }

    /// Add one edge under `relation` at runtime — `LNK-FR-005` (ADR-0047):
    /// the label is validated ([`valid_relation_label`]), then
    /// [`Symmetric::link`]'s rules apply within that label's adjacency.
    /// A label with no adjacency yet is **created** — when the layer has
    /// a base path, an empty edge blob for it and the rewritten manifest
    /// land *before* the edge is logged, so every manifest label has a
    /// blob and a crash between the steps leaves an empty relation, not
    /// a dangling one. `relation_kinds` reports the new label at once.
    ///
    /// # Errors
    ///
    /// [`LinkError::InvalidLabel`], [`LinkError::UnknownRecord`],
    /// [`LinkError::SelfLoop`], or [`LinkError::Durability`].
    pub fn link(
        &mut self,
        relation: &str,
        a: R::Id,
        b: R::Id,
    ) -> Result<LinkOutcome, LinkError<R::Id>>
    where
        S: GetById<R>,
    {
        if !valid_relation_label(relation) {
            return Err(LinkError::InvalidLabel(relation.to_string()));
        }
        if self.inner.get(a).is_none() {
            return Err(LinkError::UnknownRecord(a));
        }
        if self.inner.get(b).is_none() {
            return Err(LinkError::UnknownRecord(b));
        }
        if a == b {
            return Err(LinkError::SelfLoop(a));
        }
        if !self.adjacency.contains_key(relation) {
            if let Some(base) = &self.base_path {
                let empty: [(R::Id, R::Id); 0] = [];
                EdgeBlob::new(&empty, R::SCHEMA_TAG)
                    .encode()?
                    .write(&labeled_edges_path(base, relation))?;
                let mut labels = label_manifest::read(base, R::SCHEMA_TAG)?;
                if !labels.iter().any(|l| l == relation) {
                    labels.push(relation.to_string());
                    label_manifest::write(base, R::SCHEMA_TAG, &labels)?;
                }
            }
            self.adjacency.insert(relation.to_string(), HashMap::new());
        }
        let adjacency = self
            .adjacency
            .get_mut(relation)
            .expect("inserted just above when absent");
        if adjacency.get(&a).is_some_and(|v| v.contains(&b)) {
            return Ok(LinkOutcome::AlreadyLinked);
        }
        if let Some(base) = &self.base_path {
            let log = insert_log::log_path(&labeled_edges_path(base, relation));
            insert_log::append_item(&log, R::SCHEMA_TAG, &(a, b))?;
        }
        adjacency.entry(a).or_default().push(b);
        adjacency.entry(b).or_default().push(a);
        Ok(LinkOutcome::Linked)
    }

    /// Rebuild every relation from its own blob alone — the caller's
    /// `labels` plus, since `LNK-FR-006` (ADR-0047), every label the
    /// manifest at `<path>.relations` names (a label created at runtime
    /// by [`Self::link`]). Before that round an open-ended label set had
    /// no manifest and the caller had to name every label; the caller's
    /// list is still honored, so `crate::generic::entity`'s compile-time
    /// `RELATION_LABELS` keeps working unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::RecordBlobUnreadable`], naming
    /// whichever labeled blob is missing or invalid.
    pub fn open_portable(inner: S, path: &Path, labels: &[&str]) -> Result<Self, DurabilityError> {
        let named: OwnedLabeledRelations<R> =
            labels.iter().map(|l| (l.to_string(), Vec::new())).collect();
        let manifest = label_manifest::read(path, R::SCHEMA_TAG)?;
        let mut relations = Self::with_manifest_labels(&named, manifest);
        for (label, edges) in &mut relations {
            *edges = edge_blob::read::<R::Id>(&labeled_edges_path(path, label), R::SCHEMA_TAG)?;
        }
        // Through `open`: any label's edge log is folded and cleared; with
        // no log nothing is written, as before (`LNK-FR-007`).
        Self::open(inner, &relations, path)
    }
}

// `LNK-FR-005`: the inherent [`MultiSymmetric::link`], through the trait.
impl<S, R> MultiLink<R> for MultiSymmetric<S, R>
where
    R: Record + SchemaTag,
    R::Id: Serialize + DeserializeOwned,
    S: GetById<R>,
{
    fn link(
        &mut self,
        relation: &str,
        a: R::Id,
        b: R::Id,
    ) -> Result<LinkOutcome, LinkError<R::Id>> {
        MultiSymmetric::link(self, relation, a, b)
    }
}

impl<S, R: Record> super::query::MultiNeighbors<R> for MultiSymmetric<S, R> {
    fn neighbors_by_relation(&self, relation: &str, id: R::Id) -> Option<Vec<R::Id>> {
        self.adjacency
            .get(relation)
            .map(|adj| adj.get(&id).cloned().unwrap_or_default())
    }

    fn all_neighbors(&self, id: R::Id) -> Vec<R::Id> {
        self.adjacency
            .values()
            .flat_map(|adj| adj.get(&id).cloned().unwrap_or_default())
            .collect()
    }

    fn relation_kinds(&self) -> Vec<String> {
        self.adjacency.keys().cloned().collect()
    }
}

// Forwarding impl: `MultiSymmetric<S, ..>` re-exposing `GetById`.
// `INS-FR-005`: same as `Symmetric` — no edges under any label for a
// record inserted at runtime; no edge blob touched.
impl<S, R: Record> Insert<R> for MultiSymmetric<S, R>
where
    S: Insert<R>,
{
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        self.inner.insert(record)
    }
}

// `REP-FR-004`: same as `Symmetric` — every label's edges survive a
// replace; no edge blob touched.
impl<S, R: Record> Replace<R> for MultiSymmetric<S, R>
where
    S: Replace<R>,
{
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        self.inner.replace(record)
    }
}

impl<S, R: Record> GetById<R> for MultiSymmetric<S, R>
where
    S: GetById<R>,
{
    fn get(&self, id: R::Id) -> Option<R> {
        self.inner.get(id)
    }
}

// Forwarding impl: `MultiSymmetric<S, ..>` re-exposing `AllIds`.
impl<S, R: Record> AllIds<R> for MultiSymmetric<S, R>
where
    S: AllIds<R>,
{
    fn all_ids(&self) -> Vec<R::Id> {
        self.inner.all_ids()
    }
}

impl<S, R: Record> Flush for MultiSymmetric<S, R>
where
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

// Forwarding impl: `MultiSymmetric<S, ..>` re-exposing `FilterEq`.
impl<S, R, IndexMarker> FilterEq<R, IndexMarker> for MultiSymmetric<S, R>
where
    R: Record + IndexedField<IndexMarker>,
    S: FilterEq<R, IndexMarker>,
{
    fn filter_eq(&self, value: &R::IndexValue) -> Vec<R::Id> {
        self.inner.filter_eq(value)
    }
}

// Forwarding impl: `MultiSymmetric<S, ..>` re-exposing `ScanField`.
impl<S, R, ScanMarker> ScanField<R, ScanMarker> for MultiSymmetric<S, R>
where
    R: Record + ScannableField<ScanMarker>,
    S: ScanField<R, ScanMarker>,
{
    fn scan(&self) -> Vec<R::ScanValue> {
        self.inner.scan()
    }
}

// Forwarding impl: `MultiSymmetric<S, ..>` re-exposing `UpdateField`.
impl<S, R, ScanMarker> UpdateField<R, ScanMarker> for MultiSymmetric<S, R>
where
    R: Record + ScannableField<ScanMarker>,
    S: UpdateField<R, ScanMarker>,
{
    fn update(&mut self, id: R::Id, value: R::ScanValue) -> Result<(), NotFound<R::Id>> {
        self.inner.update(id, value)
    }
}

/// A normalized secondary index over every string key a record resolves
/// under — `ENT3-FR-002`/`003` (ADR-0040, `docs/design/
/// SERVER-ENTITY-ALIASES-DESIGN.md`). See [`super::query::NameIndexed`]'s
/// own doc comment for why this is a separate primitive and not a second
/// `IndexedField` on [`super::mmap_store::GenericMmapStore`].
///
/// # Rebuilt from records, never separately persisted
///
/// Unlike [`MultiSymmetric`], whose per-label edge lists are their own
/// durable `<path>.<label>.edges` blobs (edges aren't derivable from the
/// records alone), this layer's map is fully derivable from each record's
/// own [`NameIndexed::index_keys`](super::query::NameIndexed::index_keys)
/// — so [`Self::new`] rebuilds it from the inner store's own `AllIds`/
/// `GetById` every time, exactly as `GenericMmapStore`'s own primary
/// `index` is rebuilt from its records at `create`/`open`. No new file,
/// no `BLOB_VERSION` bump, no portability work: whatever brings the
/// inner store back brings this index back with it. A real, named
/// asymmetry between the two "Multi-" primitives this line has produced,
/// not an oversight.
///
/// # Normalization lives here, in exactly one place
///
/// `normalize` (`pub(crate)`) — every run of whitespace collapsed to one
/// space, then `str::to_lowercase` (`ENT5-FR-001`, ADR-0042: matching
/// `rusty_remind_me`'s own `normalize_entity_name`, whose doc comment
/// records that a trim-only version split `"Bailey  Robertson"` and
/// `"Bailey Robertson"` into two entities) — is
/// applied to every key at build time *and* to every query in
/// [`FindByName::find_by_name`](super::query::FindByName::find_by_name),
/// so a caller may pass raw text and a record's own `index_keys` may
/// return raw text; neither side has to agree on a convention with the
/// other. ASCII-oriented, not full Unicode case folding — see the design
/// doc's own Non-goals.
pub struct NameIndex<S, R: super::query::NameIndexed> {
    inner: S,
    index: HashMap<String, Vec<R::Id>>,
}

/// `ENT3-FR-003`/`ENT5-FR-001`: the one normalization rule, shared by
/// build and query — and by [`super::entity::entity_id`], so a derived
/// id and the name index agree on what "the same name" means. Collapses
/// every run of Unicode whitespace (tabs, newlines, doubled spaces) to a
/// single space and trims the ends, then lowercases. `pub(crate)` so
/// `entity_id` can reuse it; not `pub` — the rule is an implementation
/// detail of the index, not a public API.
pub(crate) fn normalize(key: &str) -> String {
    key.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl<S, R> NameIndex<S, R>
where
    R: super::query::NameIndexed,
    S: GetById<R> + AllIds<R>,
{
    /// Wrap `inner`, building the normalized key map from its own records.
    /// Reads every record once; writes nothing.
    pub fn new(inner: S) -> Self {
        let mut index: HashMap<String, Vec<R::Id>> = HashMap::new();
        for id in inner.all_ids() {
            let Some(record) = inner.get(id) else {
                continue;
            };
            for key in record.index_keys() {
                let bucket = index.entry(normalize(&key)).or_default();
                if !bucket.contains(&id) {
                    bucket.push(id);
                }
            }
        }
        Self { inner, index }
    }
}

impl<S, R: super::query::NameIndexed> super::query::FindByName<R> for NameIndex<S, R> {
    fn find_by_name(&self, name: &str) -> Vec<R::Id> {
        self.index
            .get(&normalize(name))
            .cloned()
            .unwrap_or_default()
    }
}

// `INS-FR-005`: the record's keys are taken before it is moved into the
// inner store, and added to the index — normalized exactly as at build
// (`ENT3-FR-003`) — only after the inner insert succeeded.
impl<S, R: super::query::NameIndexed> Insert<R> for NameIndex<S, R>
where
    S: Insert<R>,
{
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>> {
        let id = record.id();
        let keys = record.index_keys();
        self.inner.insert(record)?;
        for key in keys {
            let bucket = self.index.entry(normalize(&key)).or_default();
            if !bucket.contains(&id) {
                bucket.push(id);
            }
        }
        Ok(())
    }
}

// `REP-FR-004`: the old record's keys (read from the inner store before
// the move) leave the index and the new record's keys enter it —
// normalized exactly as at build — only after the inner replace
// succeeded. A key both versions share is untouched.
impl<S, R: super::query::NameIndexed> Replace<R> for NameIndex<S, R>
where
    S: Replace<R> + GetById<R>,
{
    fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>> {
        let id = record.id();
        let old_keys: Vec<String> = self
            .inner
            .get(id)
            .map(|old| {
                old.index_keys()
                    .into_iter()
                    .map(|k| normalize(&k))
                    .collect()
            })
            .ok_or(ReplaceError::NotFound(id))?;
        let new_keys: Vec<String> = record
            .index_keys()
            .into_iter()
            .map(|k| normalize(&k))
            .collect();
        self.inner.replace(record)?;
        for key in old_keys.iter().filter(|k| !new_keys.contains(k)) {
            if let Some(bucket) = self.index.get_mut(key) {
                bucket.retain(|other| *other != id);
                if bucket.is_empty() {
                    self.index.remove(key);
                }
            }
        }
        for key in new_keys {
            let bucket = self.index.entry(key).or_default();
            if !bucket.contains(&id) {
                bucket.push(id);
            }
        }
        Ok(())
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `GetById`.
impl<S, R: super::query::NameIndexed> GetById<R> for NameIndex<S, R>
where
    S: GetById<R>,
{
    fn get(&self, id: R::Id) -> Option<R> {
        self.inner.get(id)
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `AllIds`.
impl<S, R: super::query::NameIndexed> AllIds<R> for NameIndex<S, R>
where
    S: AllIds<R>,
{
    fn all_ids(&self) -> Vec<R::Id> {
        self.inner.all_ids()
    }
}

impl<S, R: super::query::NameIndexed> Flush for NameIndex<S, R>
where
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `FilterEq` — the
// compile-time `IndexedField` index beneath (`Entity`'s `kind`) stays
// reachable through this layer, alongside this layer's own runtime-keyed
// `FindByName`. Different traits, no coherence overlap.
impl<S, R, IndexMarker> FilterEq<R, IndexMarker> for NameIndex<S, R>
where
    R: super::query::NameIndexed + IndexedField<IndexMarker>,
    S: FilterEq<R, IndexMarker>,
{
    fn filter_eq(&self, value: &R::IndexValue) -> Vec<R::Id> {
        self.inner.filter_eq(value)
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `ScanField`.
impl<S, R, ScanMarker> ScanField<R, ScanMarker> for NameIndex<S, R>
where
    R: super::query::NameIndexed + ScannableField<ScanMarker>,
    S: ScanField<R, ScanMarker>,
{
    fn scan(&self) -> Vec<R::ScanValue> {
        self.inner.scan()
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `UpdateField`. The
// scannable field is never one of a record's `index_keys` (it is `Copy`
// and fixed-width by `GenericMmapStore`'s own contract — a number, not a
// name), so an update through this layer never invalidates the map.
impl<S, R, ScanMarker> UpdateField<R, ScanMarker> for NameIndex<S, R>
where
    R: super::query::NameIndexed + ScannableField<ScanMarker>,
    S: UpdateField<R, ScanMarker>,
{
    fn update(&mut self, id: R::Id, value: R::ScanValue) -> Result<(), NotFound<R::Id>> {
        self.inner.update(id, value)
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing `MultiNeighbors` — the
// layer `Entity`'s own stack puts directly beneath this one.
impl<S, R: super::query::NameIndexed> super::query::MultiNeighbors<R> for NameIndex<S, R>
where
    S: super::query::MultiNeighbors<R>,
{
    fn neighbors_by_relation(&self, relation: &str, id: R::Id) -> Option<Vec<R::Id>> {
        self.inner.neighbors_by_relation(relation, id)
    }

    fn all_neighbors(&self, id: R::Id) -> Vec<R::Id> {
        self.inner.all_neighbors(id)
    }

    fn relation_kinds(&self) -> Vec<String> {
        self.inner.relation_kinds()
    }
}

// Forwarding impl: `NameIndex<S, ..>` re-exposing the marker-typed
// `Neighbors` too, so it also wraps a single-relation `Symmetric` stack —
// `Reversed`'s own forwarding shape (`FR-012`). No direct `Neighbors`
// impl exists on this type to conflict with, unlike `Symmetric`.
impl<S, R, R2, RelMarker> Neighbors<R2, RelMarker> for NameIndex<S, R>
where
    R: super::query::NameIndexed,
    R2: SymmetricRelation<RelMarker>,
    S: Neighbors<R2, RelMarker>,
{
    fn neighbors(&self, id: R2::Id) -> Vec<R2::Id> {
        self.inner.neighbors(id)
    }
}

// `LNK-FR-008`: `NameIndex` forwards both link shapes — the name index
// is over records, and an edge changes no name.
impl<S, R, R2, RelMarker> Link<R2, RelMarker> for NameIndex<S, R>
where
    R: super::query::NameIndexed,
    R2: SymmetricRelation<RelMarker>,
    S: Link<R2, RelMarker>,
{
    fn link(&mut self, a: R2::Id, b: R2::Id) -> Result<LinkOutcome, LinkError<R2::Id>> {
        self.inner.link(a, b)
    }
}

impl<S, R: super::query::NameIndexed> MultiLink<R> for NameIndex<S, R>
where
    S: MultiLink<R>,
{
    fn link(
        &mut self,
        relation: &str,
        a: R::Id,
        b: R::Id,
    ) -> Result<LinkOutcome, LinkError<R::Id>> {
        self.inner.link(relation, a, b)
    }
}

/// `Parent` (the cheap direction of a directed relation) needs no new
/// store state at all — a blanket impl over anything that already
/// provides `GetById<C>`, per the design doc §2.
///
/// `self.get(child_id)` supplies `GetById`'s own "not found" shape
/// directly, turned into this trait's `Err(NotFound(child_id))` via
/// `.ok_or(...)?`; `.parent_id()` (itself `Option<C::ParentId>` — see
/// `ChildOf`'s own doc comment) becomes the `Ok(...)` payload unchanged.
/// "Child not found" and "child found, has no parent" — which a bare
/// single-level `Option` return once collapsed to the same `None` (a
/// gap `Rule`'s `chain_to_root` had to work around directly) — are
/// distinct outcomes again: `Err` vs. `Ok(None)`.
impl<S, C, Marker> Parent<C, Marker> for S
where
    C: ChildOf<Marker>,
    S: GetById<C>,
{
    fn parent(&self, child_id: C::Id) -> Result<Option<C::ParentId>, NotFound<C::Id>> {
        let child = self.get(child_id).ok_or(NotFound(child_id))?;
        Ok(child.parent_id())
    }
}

/// Adds one `Children` capability over an inner store — the generic
/// analogue of a `HashMap<CustomerId, Vec<OrderId>>` reverse index, the
/// expensive direction of a directed relation (design doc §4.3).
pub struct Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
{
    inner: S,
    children_of: HashMap<P::Id, Vec<C::Id>>,
    _marker: PhantomData<(P, Marker)>,
}

impl<S, P, C, Marker> Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
{
    /// Entries with no parent (`child.parent_id()` returns `None`) are
    /// skipped naturally — not an error, not a special case, just nothing
    /// to index for a record that isn't anyone's child. Before the
    /// optional-parent fix, `ChildOf::parent_id` returned a bare
    /// `Self::ParentId`, so every record necessarily had exactly one
    /// parent entry to insert; this `if let` is the one behavioral change
    /// that fix required here, and it's a no-op for a domain like `Order`
    /// whose `parent_id` never returns `None`.
    pub fn new(inner: S, children: &[C]) -> Self {
        let mut children_of: HashMap<P::Id, Vec<C::Id>> = HashMap::new();
        for child in children {
            if let Some(parent_id) = child.parent_id() {
                children_of.entry(parent_id).or_default().push(child.id());
            }
        }
        Self {
            inner,
            children_of,
            _marker: PhantomData,
        }
    }
}

// `INS-FR-005`: a child inserted at runtime is indexed under its parent
// (when it has one — the same `if let` `new` uses), after the inner
// store accepted it.
impl<S, P, C, Marker> Insert<C> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    S: Insert<C>,
{
    fn insert(&mut self, record: C) -> Result<(), InsertError<C::Id>> {
        let id = record.id();
        let parent_id = record.parent_id();
        self.inner.insert(record)?;
        if let Some(parent_id) = parent_id {
            self.children_of.entry(parent_id).or_default().push(id);
        }
        Ok(())
    }
}

// `REP-FR-004`: a child whose parent changed leaves the old parent's
// list and joins the new one's, after the inner store accepted it; a
// parent that did not change is untouched.
impl<S, P, C, Marker> Replace<C> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    S: Replace<C> + GetById<C>,
{
    fn replace(&mut self, record: C) -> Result<(), ReplaceError<C::Id>> {
        let id = record.id();
        let old_parent = self
            .inner
            .get(id)
            .map(|old| old.parent_id())
            .ok_or(ReplaceError::NotFound(id))?;
        let new_parent = record.parent_id();
        self.inner.replace(record)?;
        if old_parent == new_parent {
            return Ok(());
        }
        if let Some(old) = old_parent {
            if let Some(children) = self.children_of.get_mut(&old) {
                children.retain(|other| *other != id);
                if children.is_empty() {
                    self.children_of.remove(&old);
                }
            }
        }
        if let Some(new) = new_parent {
            self.children_of.entry(new).or_default().push(id);
        }
        Ok(())
    }
}

impl<S, P, C, Marker> Children<P, C, Marker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
{
    fn children(&self, parent_id: P::Id) -> Vec<C::Id> {
        self.children_of
            .get(&parent_id)
            .cloned()
            .unwrap_or_default()
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `GetById` (for the child
// record type `C` — a store built entirely from `&[C]` can never actually
// provide `GetById<P>`, the parent type; see the design doc §4.3's own
// account of the real mistake caught here during the original design
// pass).
impl<S, P, C, Marker> GetById<C> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    S: GetById<C>,
{
    fn get(&self, id: C::Id) -> Option<C> {
        self.inner.get(id)
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `AllIds` on the child
// record type `C` (`SQL-FR-005`, ADR-0034) — the same "for `C`, not `P`"
// shape `GetById`/`FilterEq`/`ScanField`/`UpdateField` above already take.
impl<S, P, C, Marker> AllIds<C> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    S: AllIds<C>,
{
    fn all_ids(&self) -> Vec<C::Id> {
        self.inner.all_ids()
    }
}

impl<S, P, C, Marker> Flush for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    S: Flush,
{
    fn flush(&self) -> Result<(), DurabilityError> {
        self.inner.flush()
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `FilterEq` on `C`.
impl<S, P, C, Marker, IndexMarker> FilterEq<C, IndexMarker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id> + IndexedField<IndexMarker>,
    S: FilterEq<C, IndexMarker>,
{
    fn filter_eq(&self, value: &C::IndexValue) -> Vec<C::Id> {
        self.inner.filter_eq(value)
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `ScanField` on `C`.
impl<S, P, C, Marker, ScanMarker> ScanField<C, ScanMarker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id> + ScannableField<ScanMarker>,
    S: ScanField<C, ScanMarker>,
{
    fn scan(&self) -> Vec<C::ScanValue> {
        self.inner.scan()
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `UpdateField` on `C`.
impl<S, P, C, Marker, ScanMarker> UpdateField<C, ScanMarker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id> + ScannableField<ScanMarker>,
    S: UpdateField<C, ScanMarker>,
{
    fn update(&mut self, id: C::Id, value: C::ScanValue) -> Result<(), NotFound<C::Id>> {
        self.inner.update(id, value)
    }
}

// Forwarding impl: `Reversed<S, ..>` re-exposing `Neighbors` — added for
// the first domain needing `SymmetricRelation` and `ChildOf` together on
// one record type (an `Employee`-style domain: `reports_to`, a directed
// self-relation, plus `collaborates_with`, a symmetric one) —
// `SERVER-QUERY-LAYER`'s third-domain validation round. `R`/`RelMarker`
// are independent of this impl's own `P`/`C`/`Marker`, not tied to the
// `ChildOf` relation `Reversed` itself indexes — in the self-referential
// case that motivated this (`R = P = C`), the same record type
// participates in both relations, but nothing here requires that.
impl<S, P, C, Marker, R, RelMarker> Neighbors<R, RelMarker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    R: SymmetricRelation<RelMarker>,
    S: Neighbors<R, RelMarker>,
{
    fn neighbors(&self, id: R::Id) -> Vec<R::Id> {
        self.inner.neighbors(id)
    }
}

// `LNK-FR-008`: `Reversed` forwards `Link` for the symmetric relation
// beneath it (`Employee`'s `collaborates_with`); its own `ChildOf`
// relation is a record field, never linked.
impl<S, P, C, Marker, R, RelMarker> Link<R, RelMarker> for Reversed<S, P, C, Marker>
where
    P: Record,
    C: ChildOf<Marker, ParentId = P::Id>,
    R: SymmetricRelation<RelMarker>,
    S: Link<R, RelMarker>,
{
    fn link(&mut self, a: R::Id, b: R::Id) -> Result<LinkOutcome, LinkError<R::Id>> {
        self.inner.link(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fresh_temp_dir;
    use std::path::PathBuf;

    /// `ENT5-FR-001`: the three cases `rusty_remind_me`'s own
    /// `entity_id_test.rs` pins (a tab and a newline inside the name, a
    /// doubled space, and the empty string), plus this crate's own
    /// pre-existing case/trim cases — every variant of one name is one
    /// key.
    #[test]
    fn normalize_collapses_internal_whitespace_and_case() {
        assert_eq!(normalize(" Bailey\t Robertson\n"), "bailey robertson");
        assert_eq!(normalize("  Bailey   Robertson  "), "bailey robertson");
        assert_eq!(normalize("bailey robertson"), "bailey robertson");
        assert_eq!(normalize("BAILEY ROBERTSON"), "bailey robertson");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
        // Collapsing is not stripping: distinct words stay distinct.
        assert_ne!(normalize("Bailey Robertson"), normalize("BaileyRobertson"));
    }

    // The smallest record type that can sit under a `Symmetric` layer —
    // the blob is independent of what `S` is, so nothing here needs a
    // `.mmap` file or a real domain.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Node {
        id: u32,
    }

    impl Record for Node {
        type Id = u32;
        fn id(&self) -> u32 {
            self.id
        }
    }

    impl SchemaTag for Node {
        const SCHEMA_TAG: &'static str = "store::tests::Node";
    }

    struct Linked;
    impl SymmetricRelation<Linked> for Node {}

    type Layer = Symmetric<BaseStore<Node>, Node, Linked>;

    // A second record type over the same `u32` id and the same relation
    // marker: its edge blob is byte-for-byte a `Node` edge blob except for
    // the tag, which is the only thing that keeps it out of `Layer`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Other {
        id: u32,
    }

    impl Record for Other {
        type Id = u32;
        fn id(&self) -> u32 {
            self.id
        }
    }

    impl SchemaTag for Other {
        const SCHEMA_TAG: &'static str = "store::tests::Other";
    }

    impl SymmetricRelation<Linked> for Other {}

    type OtherLayer = Symmetric<BaseStore<Other>, Other, Linked>;

    fn nodes() -> Vec<Node> {
        (1..=4).map(|id| Node { id }).collect()
    }

    fn edges() -> Vec<(u32, u32)> {
        vec![(1, 2), (2, 3), (1, 3)]
    }

    fn scratch(label: &str) -> (PathBuf, PathBuf) {
        let dir = fresh_temp_dir(label).unwrap();
        let edges_path = edge_blob::edges_path(&dir.join("store.mmap"));
        (dir, edges_path)
    }

    fn all_neighbors(layer: &Layer) -> Vec<Vec<u32>> {
        (1..=4)
            .map(|id| Neighbors::<Node, Linked>::neighbors(layer, id))
            .collect()
    }

    /// `INS-FR-005` (ADR-0046): the in-memory root refuses a duplicate
    /// and accepts a new id; `Symmetric` forwards and answers no
    /// neighbors for the newcomer; its edge blob is untouched.
    #[test]
    fn insert_forwards_through_symmetric_with_no_edges_and_refuses_a_duplicate() {
        use super::super::query::Insert;
        use super::super::InsertError;
        let (dir, edges_path) = scratch("symmetric_insert");
        let mut layer = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        let before = std::fs::read(&edges_path).unwrap();

        Insert::<Node>::insert(&mut layer, Node { id: 5 }).unwrap();
        assert_eq!(GetById::<Node>::get(&layer, 5), Some(Node { id: 5 }));
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&layer, 5),
            Vec::<u32>::new()
        );
        assert_eq!(all_neighbors(&layer), all_neighbors(&layer), "unchanged");
        assert_eq!(std::fs::read(&edges_path).unwrap(), before);

        match Insert::<Node>::insert(&mut layer, Node { id: 5 }) {
            Err(InsertError::Duplicate(5)) => {}
            other => panic!("expected Duplicate(5), got {other:?}"),
        }
        match Insert::<Node>::insert(&mut layer, Node { id: 1 }) {
            Err(InsertError::Duplicate(1)) => {}
            other => panic!("expected Duplicate(1), got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `LNK-FR-001`: the label charset, checked at the layer as well as at
    /// the boundary.
    #[test]
    fn valid_relation_label_accepts_a_safe_charset_only() {
        for ok in ["relates_to", "works-with", "A1", "_x", &"a".repeat(64)] {
            assert!(valid_relation_label(ok), "{ok}");
        }
        for bad in [
            "",
            "-leading",
            "has space",
            "../up",
            "a/b",
            "tab\there",
            "ünïcode",
            &"a".repeat(65),
        ] {
            assert!(!valid_relation_label(bad), "{bad:?}");
        }
    }

    /// `LNK` acceptance criterion 1 (ADR-0047) on a `Symmetric` over a
    /// blob: a link is visible both ways at once; the repeat is
    /// `AlreadyLinked` with the log unchanged; a self-loop and an unknown
    /// id are refused with nothing written; after a portable reopen the
    /// edge is in the blob, the log gone; a second reopen writes nothing.
    #[test]
    fn link_is_immediately_visible_durable_and_folded_at_reopen() {
        let (dir, edges_path) = scratch("symmetric_link");
        let log = insert_log::log_path(&edges_path);
        {
            let mut layer = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
            assert!(!log.exists());
            assert_eq!(
                Link::<Node, Linked>::link(&mut layer, 1, 4).unwrap(),
                LinkOutcome::Linked
            );
            assert!(log.exists(), "the link is logged");
            assert_eq!(
                Neighbors::<Node, Linked>::neighbors(&layer, 1),
                vec![2, 3, 4]
            );
            assert_eq!(Neighbors::<Node, Linked>::neighbors(&layer, 4), vec![1]);
            let log_len = std::fs::metadata(&log).unwrap().len();
            assert_eq!(
                Link::<Node, Linked>::link(&mut layer, 4, 1).unwrap(),
                LinkOutcome::AlreadyLinked,
                "either orientation"
            );
            assert_eq!(
                std::fs::metadata(&log).unwrap().len(),
                log_len,
                "nothing written"
            );
            assert!(matches!(
                Link::<Node, Linked>::link(&mut layer, 2, 2),
                Err(LinkError::SelfLoop(2))
            ));
            assert!(matches!(
                Link::<Node, Linked>::link(&mut layer, 1, 9),
                Err(LinkError::UnknownRecord(9))
            ));
            assert!(matches!(
                Link::<Node, Linked>::link(&mut layer, 9, 1),
                Err(LinkError::UnknownRecord(9))
            ));
            assert_eq!(std::fs::metadata(&log).unwrap().len(), log_len);
        }

        assert_eq!(
            Layer::read_portable_edges(&edges_path).unwrap(),
            vec![(1, 2), (2, 3), (1, 3), (1, 4)],
            "blob order, then the logged edge"
        );
        let reopened = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&reopened, 4), vec![1]);
        assert!(!log.exists(), "the fold removed the log");
        let blob_bytes = std::fs::read(&edges_path).unwrap();
        drop(reopened);
        let again = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&again, 1),
            vec![2, 3, 4]
        );
        assert_eq!(
            std::fs::read(&edges_path).unwrap(),
            blob_bytes,
            "no rewrite"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `LNK-FR-002`: a layer built with `new` has no path — it links in
    /// memory only and never touches a file.
    #[test]
    fn a_layer_built_with_new_links_in_memory_only() {
        let mut layer = Layer::new(BaseStore::new(nodes()), &edges());
        assert_eq!(
            Link::<Node, Linked>::link(&mut layer, 1, 4).unwrap(),
            LinkOutcome::Linked
        );
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&layer, 4), vec![1]);
    }

    /// `LNK-FR-004`: a logged edge the blob already holds is skipped, so a
    /// fold that crashed after the rewrite replays without a double edge.
    #[test]
    fn a_logged_edge_the_blob_already_holds_is_skipped() {
        let (dir, edges_path) = scratch("symmetric_link_dup");
        drop(Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap());
        let log = insert_log::log_path(&edges_path);
        insert_log::append_item(&log, Node::SCHEMA_TAG, &(3u32, 2u32)).unwrap();
        insert_log::append_item(&log, Node::SCHEMA_TAG, &(4u32, 1u32)).unwrap();
        let reopened = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&reopened, 2),
            vec![1, 3]
        );
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&reopened, 4), vec![1]);
        assert!(!log.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    type Multi = MultiSymmetric<BaseStore<Node>, Node>;

    /// `LNK` acceptance criterion 2 (ADR-0047) on `MultiSymmetric`: a link
    /// under an existing label; a link under a **new** label creates it —
    /// its blob and the manifest exist, `relation_kinds` lists it — and a
    /// portable reopen naming only the original labels finds it; an
    /// invalid label touches no file.
    #[test]
    fn multi_link_creates_labels_recorded_in_a_manifest_the_portable_reopen_reads() {
        use super::super::query::MultiNeighbors;
        let dir = fresh_temp_dir("multi_link_manifest").unwrap();
        let base = dir.join("store.mmap");
        let relations = vec![("knows".to_string(), edges())];
        {
            let mut layer = Multi::create(BaseStore::new(nodes()), &relations, &base).unwrap();
            assert_eq!(
                label_manifest::read(&base, Node::SCHEMA_TAG).unwrap(),
                vec!["knows".to_string()]
            );
            assert_eq!(
                MultiLink::<Node>::link(&mut layer, "knows", 1, 4).unwrap(),
                LinkOutcome::Linked
            );
            assert_eq!(
                MultiLink::<Node>::link(&mut layer, "mentors", 2, 4).unwrap(),
                LinkOutcome::Linked,
                "a new label"
            );
            assert!(
                labeled_edges_path(&base, "mentors").is_file(),
                "an (empty) blob"
            );
            assert_eq!(
                label_manifest::read(&base, Node::SCHEMA_TAG).unwrap(),
                vec!["knows".to_string(), "mentors".to_string()]
            );
            let mut kinds = layer.relation_kinds();
            kinds.sort();
            assert_eq!(kinds, vec!["knows", "mentors"]);
            assert_eq!(layer.neighbors_by_relation("mentors", 4), Some(vec![2]));
            assert_eq!(
                MultiLink::<Node>::link(&mut layer, "mentors", 4, 2).unwrap(),
                LinkOutcome::AlreadyLinked
            );
            for bad in ["", "../x", "has space", "-x"] {
                assert!(
                    matches!(
                        MultiLink::<Node>::link(&mut layer, bad, 1, 2),
                        Err(LinkError::InvalidLabel(_))
                    ),
                    "{bad:?}"
                );
                assert!(!labeled_edges_path(&base, bad).exists());
            }
            assert!(matches!(
                MultiLink::<Node>::link(&mut layer, "knows", 1, 1),
                Err(LinkError::SelfLoop(1))
            ));
            assert!(matches!(
                MultiLink::<Node>::link(&mut layer, "knows", 1, 9),
                Err(LinkError::UnknownRecord(9))
            ));
        }
        let reopened = Multi::open_portable(BaseStore::new(nodes()), &base, &["knows"]).unwrap();
        let mut kinds = reopened.relation_kinds();
        kinds.sort();
        assert_eq!(kinds, vec!["knows", "mentors"], "the manifest supplied it");
        assert_eq!(reopened.neighbors_by_relation("mentors", 2), Some(vec![4]));
        assert_eq!(reopened.neighbors_by_relation("knows", 4), Some(vec![1]));
        assert!(!insert_log::log_path(&labeled_edges_path(&base, "mentors")).exists());
        assert!(!insert_log::log_path(&labeled_edges_path(&base, "knows")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `LNK-FR-006`: a directory written before this round has no
    /// manifest — it reopens as before, and `open` then writes one so the
    /// caller's labels are durable from then on.
    #[test]
    fn a_directory_without_a_manifest_reopens_and_gains_one() {
        use super::super::query::MultiNeighbors;
        let dir = fresh_temp_dir("multi_link_no_manifest").unwrap();
        let base = dir.join("store.mmap");
        let relations = vec![("knows".to_string(), edges())];
        drop(Multi::create(BaseStore::new(nodes()), &relations, &base).unwrap());
        std::fs::remove_file(label_manifest::path(&base)).unwrap();
        let reopened = Multi::open_portable(BaseStore::new(nodes()), &base, &["knows"]).unwrap();
        assert_eq!(reopened.relation_kinds(), vec!["knows"]);
        assert_eq!(
            label_manifest::read(&base, Node::SCHEMA_TAG).unwrap(),
            vec!["knows".to_string()]
        );
        // A foreign manifest is refused by name.
        label_manifest::write(&base, "other::Type", &["x".to_string()]).unwrap();
        match Multi::open_portable(BaseStore::new(nodes()), &base, &["knows"]) {
            Err(DurabilityError::RecordBlobUnreadable { path, cause }) => {
                assert!(path.ends_with("store.mmap.relations"), "{path:?}");
                assert!(cause.contains("schema tag mismatch"), "{cause}");
            }
            other => panic!("expected RecordBlobUnreadable, got {:?}", other.map(|_| ())),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_then_open_portable_rebuilds_the_same_adjacency_in_the_same_order() {
        let (dir, edges_path) = scratch("symmetric_create_open_portable");
        let original = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        assert!(edges_path.is_file());

        let portable = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(all_neighbors(&portable), all_neighbors(&original));
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&portable, 1),
            vec![2, 3],
            "edge order (1,2) before (1,3) must survive the round trip"
        );
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&portable, 4),
            Vec::<u32>::new()
        );
        assert_eq!(Layer::read_portable_edges(&edges_path).unwrap(), edges());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_the_same_edges_does_not_rewrite_the_blob() {
        let (dir, edges_path) = scratch("symmetric_open_no_rewrite");
        let _ = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        let before_bytes = std::fs::read(&edges_path).unwrap();
        let before_mtime = std::fs::metadata(&edges_path).unwrap().modified().unwrap();

        let reopened = Layer::open(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&reopened, 2),
            vec![1, 3]
        );
        assert_eq!(std::fs::read(&edges_path).unwrap(), before_bytes);
        assert_eq!(
            std::fs::metadata(&edges_path).unwrap().modified().unwrap(),
            before_mtime
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_changed_edges_rewrites_the_blob() {
        let (dir, edges_path) = scratch("symmetric_open_rewrite");
        let _ = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        let changed = vec![(1, 2), (3, 4)];

        let reopened = Layer::open(BaseStore::new(nodes()), &changed, &edges_path).unwrap();
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&reopened, 3), vec![4]);
        assert_eq!(Layer::read_portable_edges(&edges_path).unwrap(), changed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_reordered_edges_counts_as_changed() {
        let (dir, edges_path) = scratch("symmetric_open_reorder");
        let _ = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        let mut reordered = edges();
        reordered.swap(0, 2);

        let reopened = Layer::open(BaseStore::new(nodes()), &reordered, &edges_path).unwrap();
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&reopened, 1),
            vec![3, 2],
            "reordered input must be observable through neighbors"
        );
        assert_eq!(Layer::read_portable_edges(&edges_path).unwrap(), reordered);
        let portable = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(
            Neighbors::<Node, Linked>::neighbors(&portable, 1),
            vec![3, 2]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_blob_is_a_typed_error_from_open_portable_and_healed_by_open() {
        let (dir, edges_path) = scratch("symmetric_missing_blob");
        match Layer::open_portable(BaseStore::new(nodes()), &edges_path) {
            Err(DurabilityError::RecordBlobUnreadable { path, cause }) => {
                assert_eq!(path, edges_path);
                assert!(cause.starts_with("cannot read file"), "{cause}");
            }
            Err(other) => panic!("expected RecordBlobUnreadable, got {other:?}"),
            Ok(_) => panic!("expected RecordBlobUnreadable, got a layer"),
        }
        assert!(matches!(
            Layer::read_portable_edges(&edges_path),
            Err(DurabilityError::RecordBlobUnreadable { .. })
        ));

        // The pre-feature directory case: `open` with the caller's edges
        // writes the blob it finds missing, after which the portable
        // path works.
        let healed = Layer::open(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&healed, 1), vec![2, 3]);
        assert!(edges_path.is_file());
        let portable = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(all_neighbors(&portable), all_neighbors(&healed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_over_an_existing_blob_always_rewrites_it() {
        let (dir, edges_path) = scratch("symmetric_create_overwrites");
        let _ = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        let fresh = vec![(4, 1)];
        let _ = Layer::create(BaseStore::new(nodes()), &fresh, &edges_path).unwrap();
        assert_eq!(Layer::read_portable_edges(&edges_path).unwrap(), fresh);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_record_types_edge_blob_is_a_tag_error_from_the_read_only_paths_and_healed_by_open() {
        let (dir, edges_path) = scratch("symmetric_other_tag");
        let others: Vec<Other> = (1..=4).map(|id| Other { id }).collect();
        let _ = OtherLayer::create(BaseStore::new(others), &edges(), &edges_path).unwrap();

        // Acceptance criterion 4: same `Id`, same edges, same bytes in
        // the body — refused by name, never decoded (`SCHTAG-FR-001`).
        for result in [
            Layer::open_portable(BaseStore::new(nodes()), &edges_path).map(|_| ()),
            Layer::read_portable_edges(&edges_path).map(|_| ()),
        ] {
            match result {
                Err(DurabilityError::RecordBlobUnreadable { path, cause }) => {
                    assert_eq!(path, edges_path);
                    assert!(
                        cause.starts_with(
                            "schema tag mismatch: this store expects `store::tests::Node`"
                        ),
                        "{cause}"
                    );
                }
                Err(other) => panic!("expected RecordBlobUnreadable, got {other:?}"),
                Ok(()) => panic!("expected RecordBlobUnreadable, got a layer"),
            }
        }

        // `open` with the caller's edges treats the wrong-tag blob as
        // stale and rewrites it under `Node`'s tag (`SCHTAG-FR-004`).
        let healed = Layer::open(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&healed, 1), vec![2, 3]);
        assert_eq!(Layer::read_portable_edges(&edges_path).unwrap(), edges());
        let portable = Layer::open_portable(BaseStore::new(nodes()), &edges_path).unwrap();
        assert_eq!(all_neighbors(&portable), all_neighbors(&healed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_version_1_edge_blob_is_a_version_error_and_healed_by_open() {
        let (dir, edges_path) = scratch("symmetric_v1_blob");
        let _ = Layer::create(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        // The exact image STORAGE-016 v0.1.0 wrote: version 1, the shared
        // 20-byte header, no tag (`SCHTAG-FR-006`).
        let bytes = std::fs::read(&edges_path).unwrap();
        let mut v1 = bytes[..20].to_vec();
        v1.extend_from_slice(&bytes[28..]);
        v1[8..12].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&edges_path, &v1).unwrap();

        match Layer::read_portable_edges(&edges_path) {
            Err(DurabilityError::RecordBlobUnreadable { cause, .. }) => {
                assert!(
                    cause.starts_with("blob version mismatch: file has 1, this build expects 2"),
                    "{cause}"
                );
            }
            Err(other) => panic!("expected RecordBlobUnreadable, got {other:?}"),
            Ok(_) => panic!("expected RecordBlobUnreadable, got edges"),
        }

        let healed = Layer::open(BaseStore::new(nodes()), &edges(), &edges_path).unwrap();
        assert_eq!(Neighbors::<Node, Linked>::neighbors(&healed, 1), vec![2, 3]);
        assert_eq!(std::fs::read(&edges_path).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
