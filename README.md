# rusty_stream

Single-node durable log for RustyMill, built on `rusty_wire`. Append-only segment
storage with a sparse offset index, in the spirit of Kafka's `.log`/`.index` model —
scoped deliberately to avoid re-deriving Kafka wholesale. See
[docs/phase1-scope.md](./docs/phase1-scope.md) for the full research brief.

## Status
Phase 1 scaffold: a single append-only `Segment` (framing, recovery, offset
tracking) exists and is tested, built directly on `rusty_tokio`'s `thread-per-core`
+ `io-uring-fs` (ADR-0002 D3/D4 — see [docs/adr/](./docs/adr/) for how that was
decided, including two full rounds of build/test/strace/ASAN verification, not
just documentation review). Retention (segment rolling/deletion) and per-consumer
offset tracking are stubbed (`src/retention.rs`, `src/consumer.rs`) but not yet
implemented. The wire protocol (`rusty_wire`, per ADR-0002 D1) isn't wired in yet.

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
