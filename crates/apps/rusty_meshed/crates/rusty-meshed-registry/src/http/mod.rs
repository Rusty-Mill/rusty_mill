//! A minimal HTTP routing layer for the registry app -- the Rust
//! equivalent of the routing/dispatch machinery FastAPI provides for
//! free in the Python source. `rusty_http` (this crate's transport
//! dependency) is deliberately scoped to the wire-level message layer
//! only (see its own module doc: "Out: ... routing frameworks"), so
//! there is no in-workspace router to build on; everything here --
//! path-pattern matching with `{param}` extraction, query-string
//! parsing, a `Request`/`Response` pair, and the accept-loop server --
//! is new.
//!
//! Deliberately *not* built: HTTP/2, keep-alive connection reuse
//! (every response sets `Connection: close`), streaming bodies,
//! WebSockets, or a general-purpose middleware chain -- [`cors`] is
//! the one cross-cutting concern the source app actually needs
//! (REG-005), so it's wired in directly rather than generalized ahead
//! of a second use case.

pub mod cors;
pub mod query;
pub mod request;
pub mod response;
pub mod router;
pub mod server;

pub use request::Request;
pub use response::Response;
pub use router::Router;
