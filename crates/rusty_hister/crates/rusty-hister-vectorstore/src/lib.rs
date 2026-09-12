//! Embedding pipeline and vector storage for semantic search — a Rust port
//! of `server/vectorstore/`.
//!
//! **Bootstrap stage — no implementation yet.** The embedding pipeline
//! itself (OpenAI-compatible `/v1/embeddings` client with adaptive
//! context-splitting and two-tier metadata/body embedding, capability
//! inventory §6.3) can build on `rusty_llama`/`rusty_provider`, both of
//! which already produce embedding vectors (confirmed by the sovereignty
//! audit — see `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md`).
//! The storage side is a genuine, separately-tracked gap: Hister's SQLite
//! backend vendors the `sqlite-vec` C extension via cgo (§6.2), which has no
//! settled Rust-side answer yet (a vendored-C build via the `cc` crate, a
//! pure-Rust vector index, or `rusty_search`'s designed-for-but-unimplemented
//! `VectorQuery` path). This is folded into the search-engine
//! decision-request (`ADR-0002`) rather than decided here, since
//! `rusty_search`'s own roadmap already anticipates hybrid search.
