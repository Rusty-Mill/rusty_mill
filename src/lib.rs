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
//! A **sans-IO core**: [`head::parse_request_head`]/
//! [`head::parse_response_head`] parse a request/response head from a byte
//! buffer without ever touching a socket, consuming *exactly* the head
//! ([`head::Outcome::Complete::consumed`]) so a caller mid-protocol-upgrade
//! (Noise, DERP, WebSocket-style flows) can take the connection over
//! byte-exact. [`body`] determines how a body's end is framed
//! (`Content-Length`, `Transfer-Encoding: chunked`, or close-delimited) and
//! provides [`body::ChunkedDecoder`], the same byte-in/byte-out shape, for
//! the incremental case.
//!
//! Sync and async I/O are thin adapters layered above the core:
//! [`sync::SyncTransport`] drives it over any `std::io::Read + Write`, and
//! [`async_tokio::AsyncTransport`] (behind the `rusty-tokio` feature) drives
//! it over [`rusty_tokio`](https://github.com/baileyrd/rusty_tokio)'s
//! `AsyncRead`/`AsyncWrite`, mirroring
//! [`rusty_tls`](https://github.com/baileyrd/rusty_tls)'s layout.
//!
//! # Scope
//!
//! In: request/response head parse + serialize (both directions); a header
//! map preserving order and case-insensitivity; the three body framings as
//! an incremental/streaming state machine; upgrade-safe head consumption;
//! [`Url`]; sync and async transport adapters; an optional `cookies` feature
//! (RFC 6265 jar, client-only, not yet built).
//!
//! Out: HTTP/2, TLS (that's `rusty_tls` -- the two compose, neither imports
//! the other), compression, multipart, routing frameworks.
//!
//! # Status
//!
//! `Url`, the sans-IO message core, and both transport adapters are built
//! and tested. Not yet built: the `cookies` feature, and any consumer
//! migration (`rusty_request`/`rusty_tail` still carry their own parsers).
//! See `ARCHITECTURE.md` for the boundary table and remaining sequencing.

mod error;
mod transport;
mod util;
mod version;

#[cfg(feature = "rusty-tokio")]
pub mod async_tokio;
pub mod body;
pub mod head;
pub mod header;
pub mod method;
pub mod status;
pub mod sync;
pub mod url;

pub use error::{Error, Result};
pub use header::HeaderMap;
pub use method::Method;
pub use status::StatusCode;
pub use transport::Error as TransportError;
pub use transport::Result as TransportResult;
pub use url::Url;
pub use version::Version;
