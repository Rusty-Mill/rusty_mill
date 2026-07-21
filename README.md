# rusty_http

One sans-IO HTTP/1.1 message layer and `Url` type for the rusty ecosystem —
replacing the hand-rolled parsers duplicated today across `rusty_request`
(client-side `http1.rs`/`url.rs`/`cookie.rs`) and four sites in `rusty_tail`.

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
above the core — the async adapter feature-gated on `rusty_tokio`,
mirroring [`rusty_tls`](https://github.com/baileyrd/rusty_tls)'s layout.

## Dependencies

Target: **zero runtime dependencies** in the core. The sans-IO parser needs
none, and even the optional adapters stay behind features so a sync-only
consumer never pulls in `rusty_tokio`. See `Cargo.toml` for the running
justification of anything added.

## Status

**Built and tested:** `Url` (`url`), the header map (`header`), `Method`/
`StatusCode`/`Version`, and the sans-IO message core — request/response
head parsing + serialization (`head`), and body framing including the
incremental chunked decoder (`body`). **Not yet built:** the sync/async
transport adapters and the `cookies` feature — see `ARCHITECTURE.md` for
the boundary table and remaining sequencing (adapters, then the migration
PRs against `rusty_request`/`rusty_tail`).

## Getting Started

```rust
use rusty_http::head::{parse_request_head, Outcome};

let buf = b"GET /a HTTP/1.1\r\nHost: example.com\r\n\r\n";
if let Outcome::Complete { head, consumed } = parse_request_head(buf, 8192).unwrap() {
    assert_eq!(head.target, "/a");
    assert_eq!(consumed, buf.len());
}
```

`cargo test --all-features` runs 57 unit tests plus a doc test.

## Security note

A shared HTTP parser is a shared attack surface — this is the one real cost
of consolidating six hand-rolled implementations into one. Head parsing and
chunked-body-framing lines are bounded against a malicious or slow peer
(`max_len` parameters, 8 KiB default) rather than allowed to grow a
caller's buffer forever — donor 6's LocalAPI server proves the core must
parse untrusted requests, not just trusted responses, so this landed with
the core itself rather than as a later hardening pass. The head parser and
chunked-body decoder are fuzz targets once an adapter exists to drive them
end-to-end (see `rustils`' fuzz setup for the ecosystem's convention).
