# rusty_http

One sans-IO HTTP/1.1 message layer and `Url` type for the rusty ecosystem.
[`rusty_request`](https://github.com/baileyrd/rusty_request) has migrated
onto it, deleting its own hand-rolled `http1.rs`/`url.rs`/`cookie.rs`/
`header.rs`/`method.rs`/`status.rs`. `rusty_tail`'s four sites (still
duplicating this logic today) are next.

## Seam

Consumers import `rusty_http` and never hand-parse or hand-serialize HTTP
again — the parsing logic exists in exactly one place. What sits behind the
seam (the framing state machine, its edge cases) can change later without
any consumer changing a line.

## Shape

A **sans-IO core**: parse and serialize HTTP/1.1 request/response heads,
and drive body framing (`Content-Length`, `Transfer-Encoding: chunked`,
close-delimited) as a byte-in/byte-out state machine that never touches a
socket. Head parsing consumes exactly the head and no further, so a caller
mid-protocol-upgrade (Noise, DERP, WebSocket-style flows) can take the
underlying connection over byte-exact. Sync and async I/O are thin adapters
above the core — one async adapter feature-gated on `rusty_tokio`,
mirroring [`rusty_tls`](https://github.com/baileyrd/rusty_tls)'s layout,
and a second feature-gated on real crates.io `tokio` for a consumer built
on that runtime instead.

## Dependencies

Target: **zero runtime dependencies** in the core. The sans-IO parser needs
none, and even the optional adapters stay behind features so a consumer
never pulls in a runtime it doesn't use. See `Cargo.toml` for the running
justification of anything added.

## Status

**Built and tested:** `Url` (`url`), the header map (`header`), `Method`/
`StatusCode`/`Version`, the sans-IO message core (`head`/`body`), three
transport adapters — `sync::SyncTransport` over any `std::io::Read +
Write`, `async_tokio::AsyncTransport` over `rusty_tokio`'s
`AsyncRead`/`AsyncWrite` (behind `rusty-tokio`), and
`tokio_native::AsyncTransport` over real tokio's `AsyncRead`/`AsyncWrite`
(behind `tokio`) — and `cookie::CookieJar` (behind the `cookies` feature,
RFC 6265, client-only). All three adapters support eager (`read_body`,
whole body in memory) and incremental (`into_body_reader`/
`BodyReader::next_chunk`, one chunk at a time) body reads.

**Migrated:** `rusty_request` (deleted its own `http1`/`url`/`cookie`/
`header`/`method`/`status`, in its own repo). **Not yet done:**
`rusty_tail`'s migration — the `tokio` adapter above was built specifically
to unblock it (its call sites are async over real tokio, which fit neither
of the first two adapters). See `ARCHITECTURE.md` for the boundary table
and that finding's full record.

## Getting Started

```rust
use rusty_http::sync::SyncTransport;
use rusty_http::body;
use std::net::TcpStream;

let stream = TcpStream::connect("example.com:80")?;
let mut t = SyncTransport::new(stream);
// ...write a request head via `t.write_request_head(&head)`, then:
let head = t.read_response_head(8192)?;
let framing = body::response_framing(&head.headers, &method, head.status)?;
let response_body = t.read_body(framing)?;
```

`cargo test --all-features` runs 107 unit tests plus a doc test.

## Security note

A shared HTTP parser is a shared attack surface — this is the one real cost
of consolidating six hand-rolled implementations into one. Head parsing and
chunked-body-framing lines are bounded against a malicious or slow peer
(`max_len` parameters, 8 KiB default) rather than allowed to grow a
caller's buffer forever — donor 6's LocalAPI server proves the core must
parse untrusted requests, not just trusted responses, so this landed with
the core itself rather than as a later hardening pass. `rusty_request` now
exercises the head parser and chunked-body decoder end-to-end in
production over real sockets; fuzzing them is still open work (see
`rustils`' fuzz setup for the ecosystem's convention).
