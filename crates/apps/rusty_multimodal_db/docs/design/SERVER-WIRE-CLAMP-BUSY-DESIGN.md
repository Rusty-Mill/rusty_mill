# Wire Marks for a Clamped Reply and a Full Server (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-22, `ADR-0103`, "2"). The two wire-level forks
  `ADR-0093` and `ADR-0102` held open.
- Date: 2026-09-22
- Related: `docs/design/SERVER-CONNECTION-LIMITS-DESIGN.md`
  (`LIM-FR-002`, option (c)), `ADR-0102` (`CLP-FR-001`, "a
  'truncated' mark on the reply"), `ADR-0022` (the four
  compatibility rules), `ADR-0043` (`SERVER-002`, the fixture, the
  reference client), `SERVER-002` §6.3 (close-without-reply).
- Supersedes/Superseded by: none. Additive: two appended variants,
  two pure functions in `serve.rs`, one downgrade arm, one client
  accessor per client.

## Purpose and scope

Say on the wire the two things the server knew and kept to itself:
that a `Query`'s rows were cut at the row cap, and that a connection
was refused because the server is full. Exactly two variants, both
appended; protocol 29 → 30.

Scope: the mark (`WCB-FR-001`); the busy frame (`WCB-FR-002`); the
downgrade (`WCB-FR-003`); the clients (`WCB-FR-004`); the
specification and fixture (`WCB-FR-005`); everything else unchanged
(`WCB-FR-006`).

## Non-goals

- **A `Busy` frame under TLS.** The accept thread would have to run
  the handshake to write one encrypted frame — the thread the cap
  exists to save. Nothing is written; the close stands.
- **Marking a page.** A page's `limit` is the client's own; above the
  cap it is refused, never clamped, so there is nothing to mark.
- **A `truncated: bool` field on `Rows`.** Rule 1: no field is added
  to a shipped variant.
- **A retry-after hint in `Busy`.** The message is fixed text; the
  client decides its own backoff.

## Context and terminology

`ADR-0102`'s clamp rewrites `Query { limit: None }` to `Some(cap)`
before dispatch and counts it; the answer is `Rows`. `ADR-0093`'s
accept check drops a socket past `max_connections` before a thread
exists. `downgrade_for_version` rewrites a response to the nearest
older shape for the connection's negotiated version (rule 3).
`SERVER-002` §6.3 lists the cases in which the server closes with no
reply; a refused accept was one in practice though not in the list.

## Requirements

- `WCB-FR-001` **The mark.** On a connection negotiated at 30 or
  above, a `Query` the clamp step rewrote and the server answered with
  rows is answered `RowsClamped { rows, cap }` where `rows` is exactly
  what `Rows` would have carried and `cap` the row cap. An error, an
  explicit `limit` at or under the cap, a page, an aggregate, a join:
  never marked.
- `WCB-FR-002` **The busy frame.** On a plaintext listener, a
  connection refused at the cap is written one `Err { Busy, message }`
  frame and closed; the write is bounded by a short timeout and its
  failure is not an error. Under TLS nothing is written. The counter
  moves as before.
- `WCB-FR-003` **The downgrade.** Below 30 a `RowsClamped` goes as
  `Rows`, and through the rest of the downgrade (so `StrList` is
  stripped below 11). `Busy` never answers a request, so it needs no
  downgrade; a client below 30 fails to decode it and fails the
  connect as it did on the close.
- `WCB-FR-004` **The clients.** `SchemaDrivenClient::query` accepts
  `RowsClamped` as rows and keeps the cap in `last_clamp()`, reset by
  the next rows answer; `connect` surfaces `Err { Busy }` as
  `ClientError::Server(Busy, ..)`. The Python client the same
  (`last_clamp`, `ServerError`), and declares 30.
- `WCB-FR-005` **The specification.** `SERVER-002` 0.19.0: §4, §5
  header, §5.2, §5.7, §6.3, §7 item 27, §8 row 30 and rule 3; the
  fixture gains `Response/RowsClamped` and `Response/Err(Busy)` at
  30; the Python conformance suite pins 30.
- `WCB-FR-006` **Everything else unchanged.** Every vector at ≤ 29
  byte-identical; every request's answer on a connection below 30
  byte-identical; the counters unchanged.

## Considered options

- **(a) `RowsClamped` as a new response variant plus a `Busy` error
  code, plaintext only — implemented.**
- **(b) (a) plus the `Busy` frame under TLS**, the accept thread
  running the handshake for it. Costs the thread the cap saves; a
  refused peer could hold the accept loop through a slow handshake.
- **(c) A `truncated` flag on `Rows`.** Rule 1 forbids it.
- **(d) Decline** — keep the counter and the close.

The owner's shorthand: **(a)** as implemented; **(b)** busy under
TLS; **(d)** decline and revert.

## Proposed shape

`src/server/protocol.rs`: `ErrorCode::Busy` (15),
`Response::RowsClamped` (24), the table row, `introduced_at`, two
golden vectors, the pin. `src/server/serve.rs`: the clamp step keeps
its `clamped` bool; `mark_clamped(resp, cap)` after dispatch;
`downgrade_for_version`'s first arm; `refuse_busy(stream, options)`
at accept; `error_message`/`outcome_of` arms.
`src/server/client.rs`: `last_clamp`, the `Query` match, the connect
match. `clients/python/`: `Busy`, `RowsClamped`, `last_clamp`, the
connect check, the version. `tests/server_limits_integration.rs`:
both proofs, at 30 and at 29. `tests/server_client_only.rs`: the
hello bytes. `tests/fixtures/wire-vectors.txt`: 84 lines.

## Data/state and invariants

- `RowsClamped { rows, .. }.rows == Rows { rows }.rows` for the same
  request on the same state.
- A connection's response stream never contains index 24 below 30.
- `Busy` is written at most once per accepted socket, only before any
  byte is read from it, and only when `try_admit` refused.

## Errors, failure, recovery, and observability

A refused accept: `Err { Busy }` then EOF on plaintext, EOF on TLS;
`connections_refused_total` either way. A clamped read: the mark;
`query_rows_clamped_total` as before. A failed busy write: the close
the client would have seen anyway.

## Security, privacy, and compatibility

A wire change, append-only (rules 1–2). `Busy` tells an unauthenticated
peer the server is at its cap — one bit an attacker measuring refusals
already had. Nothing else is revealed before authentication.

## Acceptance criteria

1. Golden vectors: `RowsClamped`, `Err(Busy)` (`protocol.rs`).
2. Integration (`tests/server_limits_integration.rs`): the mark at 30,
   `Rows` at 29, the unmarked explicit limit, `last_clamp` through the
   client; `Busy` on the refused socket and through `connect`.
3. Conformance: the Python suite at 84 vectors; the hello bytes.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
`python3 -m unittest discover -s clients/python/tests`. Independent
review owed.

## Traceability

- Roadmap: `SERVER-WIRE-CLAMP-BUSY`.
- Decision: `ADR-0103`.
- Specification: `SERVER-001` v0.84.0 / `FR-096`; `SERVER-002` 0.19.0.
- Requirements: `WCB-FR-001`–`006`.

## Open questions

- **`Busy` under TLS** — option (b), if a TLS deployment wants it and
  accepts the handshake on the accept thread (or a small pool for
  refusals). *Taken: `ADR-0104` (2026-09-22) — the small pool: one
  refusal thread per refused TLS socket under a 2 s timeout, at most
  sixteen alive, past which the silent close.*
- **A retry-after hint** — a `Busy { after_ms }` variant would be a
  protocol round of its own. *Declined (2026-09-23, the owner's
  "Decline"): the server has no honest number to put in it. A slot
  frees when an active connection ends, which nothing bounds; the
  idle timeout is a ceiling on a stalled connection, not an estimate,
  and a client backing off for it would wait minutes on a server
  that frees a slot in milliseconds; a configured constant is one the
  client can hold itself. Building it would also have meant the
  refusal thread reading the client's `Hello` before choosing the
  frame, since a new variant cannot be gated on a version the server
  has not yet read. Reopen only with a real estimate to carry.*

## Change history

- 2026-09-22: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word
  ("2"), as `SERVER-001` v0.84.0 / `FR-096`, `SERVER-002` 0.19.0.
