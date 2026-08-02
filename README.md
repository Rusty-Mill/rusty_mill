# rusty_tls

One TLS implementation, one trust policy, for the whole rusty ecosystem — so
no consumer (`rusty_request`, `rusty_rdp`, and eventually `rusty_tail`) ever
rolls its own TLS again. Wraps [rustls](https://docs.rs/rustls) behind a
seam: **consumers import `rusty_tls`, never `rustls`.**
OS trust anchors come from
[`platform::security::TrustAnchors`](https://github.com/baileyrd/rustils)
rather than a third-party crate — reading a trust store is OS
personality, not cryptography, so it lives in the crate this ecosystem
keeps OS personality in (rusty_tls#24). That seam is the
product — what sits behind it can be replaced piece by piece later without
any consumer changing a line. See `ARCHITECTURE.md` for the full design and
`docs/design-discussion-tls.md`'s upstream record (rustils#70) for why this
repo exists and what it deliberately leaves to rustils.

## Status
Early. Client side: both the sync adapter (`TlsStream`) and the async
adapter (`AsyncTlsStream`, behind the `rusty-tokio` feature) exist, backed
by the same `TrustPolicy`, with a hermetic rejection-test suite for each.
`rusty_request` (async) and `rusty_rdp` (sync, client-side only) are both
migrated onto it. Server side: `TlsAcceptor`/`TlsServerStream` (sync only —
no async server adapter yet); no consumer migrated onto it yet.

## Getting started
```bash
git clone https://github.com/baileyrd/rusty_tls
cd rusty_tls
cargo build
```

```rust
use std::net::TcpStream;
use std::io::Write;
use rusty_tls::{TlsStream, TrustPolicy};

let sock = TcpStream::connect("example.com:443")?;
let mut tls = TlsStream::new(sock, "example.com", &TrustPolicy::System)?;
tls.write_all(b"GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")?;
```

With the `rusty-tokio` feature, `AsyncTlsStream` is the same thing over
`rusty_tokio`'s `AsyncRead`/`AsyncWrite` instead of blocking `Read`/`Write`.

Server side:

```rust
use std::net::TcpListener;
use rusty_tls::TlsAcceptor;

let acceptor = TlsAcceptor::new(vec![cert_chain_der], private_key_der)?;
let (sock, _) = TcpListener::bind("0.0.0.0:8443")?.accept()?;
let mut tls = acceptor.accept(sock)?;
```

## Architecture
See [ARCHITECTURE.md](./ARCHITECTURE.md) for boundaries, key decisions, and data flow.

## Development
```bash
cargo build
cargo test
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security
See [SECURITY.md](./SECURITY.md) to report a vulnerability.

## License
Internal — not for external distribution
