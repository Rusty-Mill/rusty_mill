# ADR-0104: The `Busy` Frame Is Written Under TLS Too, From a Bounded Refusal Pool

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-22, "2" — the fork `ADR-0103` held open). No wire
  change; protocol stays 30.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0103`/`docs/design/SERVER-WIRE-CLAMP-BUSY-DESIGN.md`
  (`WCB-FR-002`, the plaintext-only `Busy` frame and its option (b)),
  `ADR-0093` (`LIM-FR-002`, the connection cap), `ADR-0032` (TLS in
  `ServeOptions`), `ADR-0010` (thread-per-connection).
- Supersedes/Superseded by: amends `WCB-FR-002` for a TLS listener.
  Additive: `ServeOptions::busy_refusals`, `serve::MAX_BUSY_REFUSALS`,
  `serve::BUSY_HANDSHAKE_TIMEOUT`, the refusal thread in
  `serve::refuse_busy`; `SERVER-002` 0.19.1.

## Context

`ADR-0103` wrote `Err { Busy }` to a refused connection on plaintext
listeners only, because under TLS the frame needs a completed
handshake and a handshake on the accept thread would let one slow peer
stall every accept — while a thread per refusal is the unbounded pool
the connection cap exists to prevent. The recommended deployment is
TLS, so the frame was absent exactly where it matters.

## Decision

Implement the small pool `ADR-0103` named, for every listener: a
refused socket gets its read and write timeouts set to
`BUSY_REFUSAL_TIMEOUT` (2 s) and is handed to a refusal thread that
completes the TLS handshake when there is one, writes the one frame,
shuts down its write side, and waits for the peer to close before
dropping the socket. That last step is not a nicety: the client's
`Hello` is unread in the socket when the frame goes out, a socket
closed with unread data is reset, and a reset lets the peer's kernel
discard the `Busy` bytes it already holds — both refusal tests were
flaky with `BrokenPipe`/`ConnectionReset` until the drain, so plaintext
refusals moved off the accept thread into the same pool. At most
`MAX_BUSY_REFUSALS` (16) such threads exist at once, counted in
`ServeOptions::busy_refusals` and released by the same guard a
connection slot uses; a refusal past that is closed silently, exactly
as before. The accept thread never blocks. The counters and the wire
are unchanged; `SERVER-002` 0.19.1 records that a close without the frame
is still a failed connect, never "not full". Proven over TLS at a cap
of one: the second connect reads `Busy` then EOF, the first still
answers, and the third is admitted once the first ends.

## Consequences

- Positive: a TLS client can tell a full server from a dead one; the
  frame reliably reaches a plaintext client too; a flood of refused
  connects holds at most sixteen threads, each alive for a bounded few
  seconds.
- Negative / tradeoffs: sixteen and two seconds are constants, not
  settings — named as the fork (an operator who wants a larger
  courtesy pool). A refusal past the pool is indistinguishable, to the
  client, from a pre-`ADR-0103` server.
- Named, not hidden: the refusal thread runs the handshake against the
  server's full acceptor, so a peer that fails client-certificate
  admission on an mTLS listener is also closed without the frame — the
  handshake fails first, and nothing is audited for a refusal.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.85.0 / `FR-097`, `SERVER-002` 0.19.1.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 967 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
