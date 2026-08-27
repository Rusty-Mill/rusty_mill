//! [`TlsConnector`]: the client-side mirror of [`TlsAcceptor`](crate::TlsAcceptor)
//! — builds a `rustls::ClientConfig` once and reuses it (`Arc`-backed)
//! across every connection, the way a server built once and reused is
//! naturally what session resumption needs.
//!
//! [`TlsStream::new`](crate::TlsStream::new) and
//! [`AsyncTlsStream::new`](crate::AsyncTlsStream::new) each build a fresh
//! `ClientConfig` — and thus a fresh, empty resumption cache — on every
//! call, so rustls' own default resumption support (a real 256-entry
//! session cache, on by default) never actually gets a chance to trigger
//! through them alone. `TlsConnector` exists for a caller that reconnects
//! to the same host repeatedly and wants that resumption to actually
//! happen.

use std::io::{Read, Write};
use std::sync::Arc;

use rustls::ClientConfig;

use crate::client::TlsStream;
use crate::error::Error;
use crate::trust::{build_client_config, TrustPolicy};

/// A [`TrustPolicy`] built into a `rustls::ClientConfig` once, and reused
/// across every connection made through it. Cheap to clone (an `Arc`
/// underneath).
#[derive(Clone)]
pub struct TlsConnector {
    config: Arc<ClientConfig>,
}

impl TlsConnector {
    /// Build the client config for `policy` once, ready to be reused by
    /// every subsequent [`TlsConnector::connect`]/
    /// [`TlsConnector::connect_async`] call.
    pub fn new(policy: &TrustPolicy) -> Result<Self, Error> {
        Ok(Self {
            config: build_client_config(policy)?,
        })
    }

    /// Wrap `sock` in a TLS client connection to `server_name`, sharing
    /// this connector's config (and resumption cache) with every other
    /// connection made through it. Otherwise identical to
    /// [`TlsStream::new`].
    pub fn connect<S: Read + Write>(
        &self,
        sock: S,
        server_name: &str,
    ) -> Result<TlsStream<S>, Error> {
        TlsStream::from_config(self.config.clone(), sock, server_name)
    }

    /// Like [`TlsConnector::connect`], but over `rusty_tokio`'s
    /// `AsyncRead + AsyncWrite` — the async counterpart, behind the
    /// `rusty-tokio` feature.
    #[cfg(feature = "rusty-tokio")]
    pub fn connect_async<S: rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin>(
        &self,
        io: S,
        server_name: &str,
    ) -> Result<crate::async_client::AsyncTlsStream<S>, Error> {
        crate::async_client::AsyncTlsStream::from_config(self.config.clone(), io, server_name)
    }
}
