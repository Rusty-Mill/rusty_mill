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

Every gap this document previously tracked as "still open" (segment manifest
persistence, the frame-size cap, graceful shutdown) is now closed, and — as
of `baileyrd/rusty_tokio#256` landing — `rusty_stream` has a real standalone
server binary (`src/main.rs`), not just an in-process API for tests. See
[RELEASE_NOTES.md](./RELEASE_NOTES.md) for the full history and each
change's own stated known limitations (smaller, ongoing ones like
`MAX_FRAME_LEN` not yet being configurable per deployment).

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

### Running the server
```bash
RUSTY_STREAM_ADDR=127.0.0.1:7420 RUSTY_STREAM_DATA_DIR=./data cargo run
```
Both environment variables are optional (defaults shown above). An existing
`RUSTY_STREAM_DATA_DIR` from a prior run is recovered on startup; a fresh one
is created only if none exists. `Ctrl-C` triggers graceful shutdown (drains
in-flight connections, doesn't cut them off — see `server::serve`'s docs).

### Container image
A `Dockerfile` exists (multi-stage build, ships just the release binary) —
**not verified with a real `docker build`/`docker run`**, stated plainly:
no Docker daemon was available in the environment that wrote it. Verify
before relying on it for a real deployment. `RUSTY_STREAM_ADDR` defaults to
`0.0.0.0:7420` inside the image (not `127.0.0.1`, so other containers can
reach it); `RUSTY_STREAM_DATA_DIR` defaults to `/data`, meant to be mounted
as a volume.

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
