# rusty_stream

Single-node durable log for RustyMill, built on `rusty_wire`. Append-only segment
storage with a sparse offset index, in the spirit of Kafka's `.log`/`.index` model —
scoped deliberately to avoid re-deriving Kafka wholesale. See
[docs/phase1-scope.md](./docs/phase1-scope.md) for the full research brief.

## Status
Pre-RFC / research phase. No implementation has landed yet — the open questions in
`docs/phase1-scope.md` §6 need answers before an ADR is drafted and code starts.

## Getting started
<!-- No dev loop yet — nothing has been implemented. This section gets filled in
     once the storage engine and Rust SDK exist. -->

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
<!-- Test/lint commands land once a Cargo.toml exists. -->

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
Internal — not for external distribution
