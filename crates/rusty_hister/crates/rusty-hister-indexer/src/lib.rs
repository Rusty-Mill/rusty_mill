//! Hister's query language and full-text indexing — a Rust port of
//! `server/indexer/`, `server/indexer/querybuilder/`, and
//! `server/indexer/searchschema/`.
//!
//! **Bootstrap stage — no implementation yet, blocked on a decision.** See
//! `docs/decisions/ADR-0002-search-indexing-engine-approach-proposal.md`
//! (open, awaiting sign-off) before writing any indexing logic here: the
//! choice of backend engine (`rusty_search` + Tantivy vs. SQLite-FTS5 vs.
//! something else), the multi-language `IndexAlias` federation equivalent,
//! the `url_re:` custom-filter-after-retrieval pattern, and the three
//! highlight-rendering styles all need resolving first. The query
//! grammar/lexer itself (capability-inventory §5.2-§5.4) is *not* blocked —
//! it is backend-independent and can be built as a parser that compiles to
//! `rusty_search`'s `Query` tree once the crate exists, but hasn't been
//! started yet since it depends on this crate's dependency on the chosen
//! backend.
//!
//! See `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §5 for
//! the full grammar, field-filter table, sort/facet options, and
//! fingerprinting mechanism this crate must reproduce.
