# rusty_stream

Single-node durable log for RustyMill, built on `rusty_wire`. Append-only segment
storage with a sparse offset index, in the spirit of Kafka's `.log`/`.index` model —
scoped deliberately to avoid re-deriving Kafka wholesale. See
[docs/phase1-scope.md](./docs/phase1-scope.md) for the full research brief.

## Status
Phase 1's full `docs/phase1-scope.md` §2 scope list is now implemented: an
append-only `Segment` (framing, recovery, offset tracking) rolled and retained
by `retention::Log`, per-consumer offset tracking (`ConsumerOffsets`), a wire
protocol built on `rusty_wire` (`protocol.rs`), `server::serve` — a real
`rusty_tokio` TCP server dispatching `Produce`/`Fetch` (against the log) and
`Commit`/`LastCommitted` (against consumer offsets) requests against shared
state — and `client::Client`, the Rust client SDK driving that same protocol
from the other end. All built directly on `rusty_tokio`'s `thread-per-core` +
`io-uring-fs` (ADR-0002 D3/D4 — see [docs/adr/](./docs/adr/) for how that was
decided, including multiple rounds of build/test/strace/ASAN verification, not
just documentation review).

Still open: no graceful shutdown or frame-size cap on the server. See
[RELEASE_NOTES.md](./RELEASE_NOTES.md) for the full history.

## Getting started
```bash
git clone https://github.com/baileyrd/rusty_stream
cd rusty_stream
cargo build
cargo test
```
`rusty_tokio` is pulled as a pinned `git` dependency (see `Cargo.toml`'s comment
for why it's a `rev`, not a branch) — no local sibling checkout needed as of
`baileyrd/rusty_tokio#254`.

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
```bash
cargo build
cargo test --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```
Matches exactly what CI runs (`.github/workflows/ci.yml`) — if these pass
locally, CI passes.

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
Internal — not for external distribution
