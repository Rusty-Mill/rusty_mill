//! A generic record/schema/query library: any domain that implements the
//! trait set in [`crate::generic::traits`] gets equality-indexed lookup, scannable-field
//! access, and symmetric/directed relationship traversal, composed from
//! reusable store layers ([`crate::generic::store`]) — including real durability via mmap
//! ([`crate::generic::mmap_store`]) and concurrent access via `RwLock` ([`crate::generic::production`]),
//! the same recipe `crate::production::ProductionStore` uses for `Dog`,
//! generalized.
//!
//! # From design, to spike, to library
//!
//! `docs/design/GENERIC-SCHEMA-DESIGN.md` and ADR-0009 proposed this
//! design; four validation spikes (`src/generic_spike/`, kept around as
//! historical record, not deleted) tested it against real code and real
//! benchmarks before any of it was treated as accepted:
//!
//! 1. **Dog overhead** — does genericizing `Dog`'s schema/query surface
//!    cost anything relative to `CanonicalCachedStore`? Negligible — but
//!    surfaced a real ambiguous-associated-type compile error.
//! 2. **Ambiguity diagnosis** (`Order`/`Customer`) — the ambiguity is
//!    worse than round 1 suggested: a same-trait multi-marker case hits a
//!    real Rust coherence limit (`E0119`), not just a naming collision.
//!    `IndexedField`/`ScannableField`'s associated types are renamed
//!    `IndexValue`/`ScanValue` (fixes the cross-trait case).
//! 3. **Macro-generated forwarding** — `forward_scannable_pairs!`
//!    (`store.rs`) generates the O(pairs) concrete forwarding impls the
//!    coherence limit requires, from a field list, so the human-maintained
//!    surface is one macro invocation, not hand-written impls per pair.
//! 4. **Directed-relation generalization** — does the adjacency-index
//!    pattern that made `littermate_of` traversal ~100,000× faster than a
//!    linear scan generalize to a *directed* relation
//!    (`Order belongs_to Customer`)? Yes, same order of magnitude,
//!    measured and explained.
//!
//! Every risk `docs/design/GENERIC-SCHEMA-DESIGN.md` §4 named has now been
//! individually resolved with real data. This module is that validated
//! design, promoted: [`crate::generic::traits`]/[`crate::generic::query`]/[`crate::generic::store`] are the same trait/
//! wrapper shapes the spikes proved out (not a rewrite), and
//! [`crate::generic::order_customer`] is the `Order`/`Customer` domain promoted from
//! prototype status to this library's real reference implementation.
//! [`crate::generic::mmap_store`]/[`crate::generic::production`] are new this round — the actual point of
//! the whole arc: a real generalized *production* store, not just traits
//! that compile in isolation. See ADR-0009 (now Accepted) for the full
//! acceptance record.
//!
//! # `Dog`/`ProductionStore` are not touched or replaced
//!
//! This module adds new, parallel capability. `crate::production::ProductionStore`,
//! `crate::store::DogStore`, and every benchmarked backend remain exactly
//! as they were — still the empirically-validated recommendation for the
//! `Dog` benchmark work this crate started as. Nothing in `src/generic/`
//! is wired into any of them, and nothing in `src/production.rs` or
//! `src/store/**` changed to build this module.
//!
//! # Write-through consistency, in both the durable and in-memory paths
//!
//! `GetById::get` reflects a field `UpdateField::update` just wrote, on
//! both the durable core ([`crate::generic::mmap_store::GenericMmapStore`]) and the
//! purely in-memory [`crate::generic::store::BaseStore`]/[`crate::generic::store::Indexed`]/[`crate::generic::store::Scanned`]
//! composition — the same guarantee every hand-written backend in this
//! crate has (`CanonicalCachedStore::update_age` mutates both its
//! canonical record and its cache). The two paths get there by different
//! mechanisms, since they have structurally different shapes:
//!
//! - [`crate::generic::mmap_store::GenericMmapStore`] is a single hand-fused struct that
//!   owns both the (possibly stale) constructed-from record and the live
//!   mmap value directly — its `get` merges the two on read, via
//!   [`crate::generic::traits::ScannableField::set_scannable_value`].
//! - [`crate::generic::store::Scanned`] is a separate struct layered *on top of* whatever
//!   owns the record (typically [`crate::generic::store::BaseStore`], several layers
//!   down) — it has no way to reach down and mutate that owner's storage.
//!   Its `GetById` forwarding impl instead patches the record it gets
//!   back from its inner store with its own cached value, using the same
//!   `set_scannable_value`, before returning it. When multiple `Scanned`
//!   layers stack (e.g. `Order`'s `Amount`/`CreatedAt`/`DiscountCents`),
//!   each one patches only its own field as `get` unwinds back up through
//!   the stack, so the record is fully consistent by the time it reaches
//!   the caller — no change needed in `Indexed`/`Symmetric`/`Reversed`,
//!   none of which own any `ScannableField` data to patch.
//! - [`crate::generic::mmap_scanned::MmapScanned`] is `Scanned`'s durable twin: the same
//!   layered shape, but its one field lives in its own slot file (the same
//!   engine `GenericMmapStore` uses) rather than a `HashMap`, so a stack
//!   can hold more than one mutable-and-durable field without
//!   `GenericMmapStore` itself growing past its one. Its `GetById` patches
//!   the record the same way `Scanned` does.
//!
//! **A real, measured cost, not free**: unlike the durable core (where the
//! merge replaces work that already had to happen), the in-memory fix adds
//! one `HashMap` lookup per `Scanned` layer on every `get` call. Measured
//! directly (same-session, back-to-back, `benches/generic_spike.rs`'s
//! `generic_get` on `Dog`'s single-`Scanned`-layer stack): roughly 43–88%
//! slower across 1K/100K/1M records than before this fix. `scan`/`scan_ages`
//! and every other capability are untouched and unaffected — see
//! `RESULTS.md`'s `## Generic schema library` section for the full
//! numbers. This is the accepted cost of the correctness guarantee, not
//! an unexamined regression.
//!
//! # Where the machinery lives
//!
//! Everything generic here (the traits, the store layers, `GenericMmapStore`,
//! `GenericProductionStore`, the error types, and the `Order` fixture) moved
//! to `rusty_multimodal_db_engine` (ADR-0124) so another product can embed it.
//! It is re-exported below under the same paths, so `crate::generic::store`,
//! `crate::generic::GenericMmapStore` and the rest resolve as before. What
//! stays in this module is this crate's own domains.

pub use rusty_multimodal_db_engine::generic::*;

/// `Entity` — this library's second front-door domain (`ENT-FR-001`,
/// ADR-0037), and its first with a `SymmetricRelation`. See this
/// module's own doc comment for the full account.
pub mod entity;

/// `Memory` — this library's third front-door domain (`MEM-FR-001`,
/// ADR-0048): the consumer's own `memories` table, bounded to the eleven
/// fields a memory is. See the module's own doc comment.
pub mod memory;

/// `Reminder` — this library's first front-door domain, not `research`-
/// gated (`RMD-FR-001`, ADR-0036): unlike `order_customer`, this is not
/// reference material validating the design, but real, deployable
/// capability. See this module's own doc comment for the full account.
pub mod relation;

pub mod reminder;
