//! Extractor SDK and the built-in per-site/format content extractors — a
//! Rust port of `server/extractor/`.
//!
//! **Bootstrap stage — no implementation yet.** See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §4 for the
//! full contract: the `Extractor` trait's `Enrich`/`Extract`/`Preview`
//! capability triad and its success/fallback/abort tri-state
//! chain-of-responsibility (§4.1), the ordered registry with
//! `RegisterBefore` insertion (§4.2), the semantically-significant default
//! chain order (§4.3), and the 20-entry extractor list with their
//! Go-test-fixture parity obligations (§4.5) — 9 of which have no existing
//! Go test coverage and need fresh Rust-side tests derived from reading the
//! Go source, not ported fixtures.
//!
//! Whether any Go test fixtures are copied verbatim, versus rewritten from
//! independent reading of the (AGPL-3.0) source, is an open licensing
//! question tracked in
//! `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md` — do not
//! copy Go test files into this crate before that's resolved.
