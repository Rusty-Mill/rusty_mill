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

Skeleton stage — `Cargo.toml` and crate docs exist; no parsing/serialization
code has landed yet. See `ARCHITECTURE.md` for the boundary table and build
sequencing (`Url` and the sans-IO core next, then adapters, then the
migration PRs against `rusty_request`/`rusty_tail`).

## Getting Started

Nothing to run yet — `cargo build` succeeds against an empty crate.

## Security note

A shared HTTP parser is a shared attack surface — this is the one real cost
of consolidating six hand-rolled implementations into one. The head parser
and chunked-body decoder are fuzz targets once they exist (see
`rustils`' fuzz setup for the ecosystem's convention), and this crate accepts
untrusted server input (donor 6's LocalAPI server proves the core must
parse a request, not just a response) — bounding header/line size against a
malicious or slow peer is part of the core's scope, not a later hardening
pass.
