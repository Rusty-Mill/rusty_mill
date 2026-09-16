//! Embedding pipeline and vector storage for semantic search — a Rust port
//! of `server/vectorstore/`.
//!
//! **Bootstrap stage — no implementation yet.** The embedding pipeline
//! itself (OpenAI-compatible `/v1/embeddings` client with adaptive
//! context-splitting and two-tier metadata/body embedding, capability
//! inventory §6.3) can build on `rusty_llama`/`rusty_provider`, both of
//! which already produce embedding vectors (confirmed by the sovereignty
//! audit — see `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md`).
//! The storage side is decided in `docs/decisions/
//! ADR-0002-search-indexing-engine-approach-proposal.md` (Accepted, folded
//! in as a sub-decision rather than its own ADR): a vendored-C build of
//! `sqlite-vec` (via the `cc` crate) for the SQLite deployment path,
//! `pgvector` for the Postgres path.
