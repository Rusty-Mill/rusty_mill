# rusty_time

A `#![no_std]` + `alloc` sovereign `Date`, `Time`, `DateTime`, RFC 3339/ISO-8601
parser/formatter, and timezone-offset calculation crate for the **Rusty Mill**
ecosystem — a dependency-free alternative to crates like `time` and `chrono` for
consumers that don't want them.

## Status
Active — @baileyrd.

## Getting started
```bash
# rusty_time depends on rusty_std via a path dependency, so it must be
# checked out as a sibling directory:
git clone https://github.com/baileyrd/rusty_time
git clone https://github.com/baileyrd/rusty_std ../rusty_std   # from inside rusty_time/

cargo build
cargo test
```

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
```bash
cargo test              # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
MIT OR Apache-2.0 (see `Cargo.toml`).
