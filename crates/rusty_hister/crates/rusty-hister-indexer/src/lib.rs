//! Hister's query language and full-text indexing — a Rust port of
//! `server/indexer/`, `server/indexer/querybuilder/`, and
//! `server/indexer/searchschema/`.
//!
//! **Bootstrap stage — no implementation yet, decision made.** See
//! `docs/decisions/ADR-0002-search-indexing-engine-approach-proposal.md`
//! (Accepted): `rusty_search` + `rusty-search-sqlite-fts5` is the chosen
//! backend; the multi-language `IndexAlias` federation equivalent, the
//! `url_re:` custom-filter-after-retrieval pattern, and the three
//! highlight-rendering styles are all this crate's own composition over
//! `rusty_search`, not backend features. The query grammar/lexer itself
//! (capability-inventory §5.2-§5.4) is backend-independent and can be built
//! as a parser that compiles to `rusty-search-core`'s `Query` tree.
//!
//! See `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §5 for
//! the full grammar, field-filter table, sort/facet options, and
//! fingerprinting mechanism this crate must reproduce.
