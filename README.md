# rusty_err

A `#![no_std]` + `alloc` sovereign error trait, context extension, and
`#[derive(Error)]` proc-macro for the **Rusty Mill** ecosystem — a
`no_std`-safe alternative to `thiserror` + `anyhow`.

## Status
Active — early (0.1.0), single maintainer (baileyrd).

## Getting started
```bash
git clone https://github.com/baileyrd/rusty_err
cd rusty_err
cargo build --workspace
```

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
```bash
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
MIT OR Apache-2.0
