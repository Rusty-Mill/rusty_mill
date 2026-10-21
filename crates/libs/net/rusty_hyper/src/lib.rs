//! HTTP/2-only client and server packaging over [`rusty_h2`]: `connect(url)`
//! for plain h2c or TLS with ALPN `h2`, and `serve(listener, tls, handler)`,
//! with connect and handshake timeouts, an optional keepalive, a connection
//! cap and graceful shutdown. No HTTP/1.1, no proxies, no redirects.
//!
//! The transport is behind the `rusty-tokio` feature, so a default build
//! pulls in nothing.
#![cfg(feature = "rusty-tokio")]

mod client;
mod error;
mod server;

pub use client::{connect, ClientIdentity, ConnectOptions, Connection, Keepalive};
pub use error::Error;
pub use server::{serve, ServeOptions, Server};

/// The HTTP/2 types a handler or caller works with.
pub use rusty_h2::driver::{
    Body, ClientHandle, Request, Respond, Response, ResponseFuture, SendStream,
};
/// Why an HTTP/2 call or connection failed.
pub use rusty_h2::driver::Error as H2Error;
/// A header field, as `rusty_h2` carries it.
pub use rusty_h2::hpack::HeaderField;
