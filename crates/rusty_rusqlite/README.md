# rusty_-rusqlite

A pure-Rust, from-scratch reimplementation of SQLite, aiming for API parity
with the [`rusqlite`](https://docs.rs/rusqlite) crate's public surface —
without depending on the C SQLite library. Tracked incrementally against
that reference surface via the platform's parity-loop process (see
`gap-analysis.md` once generated, and open issues labeled `parity-gap`).

## Status

Early development — greenfield. No engine code exists yet; this repo
currently holds governance scaffolding and (as of the next commit) a
minimal crate skeleton. Owner: baileyrd.

## Getting started
```bash
cargo build
cargo test
```

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
Internal — not for external distribution.
