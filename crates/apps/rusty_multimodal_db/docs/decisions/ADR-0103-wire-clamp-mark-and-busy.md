# ADR-0103: The Wire Says When a Reply Was Clamped and When a Server Is Full

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-22, "2" — the two wire-level forks `ADR-0093` and
  `ADR-0102` held open). Protocol 29 → 30.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0102`/`CLP-FR-001` (the clamp this marks),
  `ADR-0093`/`LIM-FR-002` (the refused accept this names),
  `ADR-0022`/`PROTO-FR-005` (the four compatibility rules),
  `ADR-0043` (`SERVER-002`, the fixture, the Python client),
  `docs/design/SERVER-WIRE-CLAMP-BUSY-DESIGN.md`.
- Supersedes/Superseded by: none. Additive, append-only:
  `Response::RowsClamped` (24), `ErrorCode::Busy` (15),
  `serve::mark_clamped`, `serve::refuse_busy`, one
  `downgrade_for_version` arm, `SchemaDrivenClient::last_clamp`, the
  Python client's `RowsClamped`/`Busy`/`last_clamp`; `SERVER-002`
  0.19.0.

## Context

Two things the server knew and did not say. Under `ADR-0102` a
`Query` with no `limit` is answered with the first `cap` rows, and
the client cannot tell that answer from a complete one; only the
operator's counter moves. Under `ADR-0093` a connection past the
connection cap is closed with nothing written, and the client cannot
tell a full server from a dead one. Both were named as wire changes
and held for the owner, since a wire change is a public API change.

## Decision

Implement, as protocol 30:

- `Response::RowsClamped { rows, cap: u64 }` — `Rows` plus the cap —
  answers a clamped `Query` on a connection negotiated at 30 or
  above; `Rows` below (rule 3, one `downgrade_for_version` arm that
  re-enters the function so a connection below 11 also loses its
  `StrList` fields). The mark is applied after dispatch, before the
  downgrade, only when the clamp step rewrote the request and the
  answer is rows; an error, and an explicit `limit` at or under the
  cap, are never marked.
- `ErrorCode::Busy` — written as one `Err { Busy }` frame on the
  accept thread, on a plaintext listener, to a connection refused at
  the cap, then the socket is closed. The one frame ever sent before
  negotiation. Under TLS nothing is written, as before: a handshake
  would cost the thread the cap exists to save. A client below 30
  cannot decode index 15 and fails the connect as it failed on the
  silent close.
- The Rust client keeps the cap in `last_clamp()` and surfaces `Busy`
  from `connect` as `ClientError::Server(Busy, ..)`; the Python client
  the same (`last_clamp`, `ServerError`). Two golden vectors; the
  fixture at 84; `SERVER-002` 0.19.0.

## Consequences

- Positive: a short answer is visible to the client that received it;
  a full server is distinguishable from a dead one on a plaintext
  listener; every older client sees exactly what it saw before.
- Negative / tradeoffs: under TLS a refused connect is still a silent
  close — named, not hidden. The mark is a new response variant
  rather than a field on `Rows` (rule 1 forbids the field). The SQL
  client's `QueryResult` is unchanged; the mark is an accessor beside
  it, so no caller's match breaks.
- Named, not hidden: a `Busy` frame under TLS would need a handshake
  on a thread, which is the cap's own cost; a per-connection
  "busy" thread pool is a later round if wanted.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.84.0 / `FR-096`, `SERVER-002` 0.19.0.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 966 tests across 44 targets, 0 failed; the Python conformance suite green at 84 vectors. Builder: Claude; independent Codex inspection owed.
