//! One HTTP/1.1 message layer and [`Url`] type for the rusty ecosystem,
//! replacing the hand-rolled parsers duplicated across `rusty_request` and
//! `rusty_tail` today.
//!
//! # Seam
//!
//! Consumers import `rusty_http` and never hand-parse or hand-serialize
//! HTTP again -- the parsing logic exists in exactly one place. What sits
//! behind this seam (the framing state machine, its edge cases) can change
//! without any consumer changing a line.
//!
//! # Shape
//!
//! A **sans-IO core**: parsing and serializing HTTP/1.1 request/response
//! heads, and driving body framing (`Content-Length`, `Transfer-Encoding:
//! chunked`, close-delimited) as a byte-in/byte-out state machine that never
//! touches a socket. Head parsing consumes exactly the head and no further,
//! so a caller mid-protocol-upgrade (Noise, DERP, WebSocket-style flows) can
//! take the underlying connection over byte-exact.
//!
//! Sync and async I/O are thin adapters layered above the core -- the async
//! adapter feature-gated on `rusty_tokio`, mirroring
//! [`rusty_tls`](https://github.com/baileyrd/rusty_tls)'s layout. Neither
//! adapter exists yet; see the crate's `ARCHITECTURE.md` for the planned
//! shape and current status.
//!
//! # Scope
//!
//! In: request/response head parse + serialize (both directions); a header
//! map preserving order and case-insensitivity; the three body framings as
//! an incremental/streaming state machine; upgrade-safe head consumption;
//! [`Url`]; an optional `cookies` feature (RFC 6265 jar, client-only).
//!
//! Out: HTTP/2, TLS (that's `rusty_tls` -- the two compose, neither imports
//! the other), compression, multipart, routing frameworks.
//!
//! Nothing is implemented yet -- this crate is at the skeleton stage. See
//! `ARCHITECTURE.md` for the boundary table and build sequencing.
