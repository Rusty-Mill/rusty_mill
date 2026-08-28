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
//! [`sync::SyncTransport`] drives it over any `std::io::Read + Write`;
//! [`async_tokio::AsyncTransport`] (behind the `rusty-tokio` feature) drives
//! it over [`rusty_tokio`](https://github.com/baileyrd/rusty_tokio)'s
//! `AsyncRead`/`AsyncWrite`, mirroring
//! [`rusty_tls`](https://github.com/baileyrd/rusty_tls)'s layout; and
//! [`tokio_native::AsyncTransport`] (behind the `tokio` feature) drives it
//! over real crates.io `tokio`'s `AsyncRead`/`AsyncWrite`, for a consumer
//! (`rusty_tail`) built on that runtime instead.
//!
//! # Scope
//!
//! In: request/response head parse + serialize (both directions); a header
//! map preserving order and case-insensitivity; the three body framings as
//! an incremental/streaming state machine; upgrade-safe head consumption;
//! [`Url`]; sync and async (over either `rusty_tokio` or real `tokio`)
//! transport adapters; an optional `cookies` feature ([`cookie::CookieJar`],
//! RFC 6265, client-only).
//!
//! Out: HTTP/2, TLS (that's `rusty_tls` -- the two compose, neither imports
//! the other), compression, multipart, routing frameworks.
//!
//! # Status
//!
//! `Url`, the sans-IO message core, all three transport adapters, and the
//! `cookies` feature are built and tested. Not yet built: any consumer
//! migration -- `rusty_request` migrated onto this crate in its own repo;
//! `rusty_tail`'s migration (which needed the `tokio` adapter added here
//! first) is still pending. See `ARCHITECTURE.md` for the boundary table
//! and remaining sequencing.

mod error;
mod transport;
mod util;
mod version;

#[cfg(feature = "rusty-tokio")]
pub mod async_tokio;
pub mod body;
#[cfg(feature = "cookies")]
pub mod cookie;
pub mod head;
pub mod header;
pub mod method;
pub mod status;
pub mod sync;
#[cfg(feature = "tokio")]
pub mod tokio_native;
pub mod url;

pub use error::{Error, Result};
pub use header::HeaderMap;
pub use method::Method;
pub use status::StatusCode;
pub use transport::Error as TransportError;
pub use transport::Result as TransportResult;
pub use url::Url;
pub use version::Version;
