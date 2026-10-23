//! One TLS implementation, one trust policy, for the whole rusty ecosystem.
//!
//! `rusty_tls` wraps [rustls](https://docs.rs/rustls) behind a seam: callers
//! import this crate, never `rustls` directly. That's the whole point —
//! what sits behind the seam can change later (a different backend, native
//! OS verification on some platforms) without any consumer changing a line.
//!
//! # Scope
//!
//! Client side: [`TlsStream`], a sync adapter layered over any
//! `Read + Write` stream, plus [`TrustPolicy`], the one place trust
//! decisions are made — including
//! [`PinnedAnchorsWithRevocation`](TrustPolicy::PinnedAnchorsWithRevocation)
//! for CRL-based revocation checking. `new_with_client_identity` presents a
//! client certificate (mTLS) to a server that requests one; `new_with_alpn`
//! offers ALPN protocols (read back via `negotiated_alpn_protocol`).
//! [`TlsConnector`] builds a config once for reuse across many connections,
//! the way real session resumption (`resumed_session`) needs. Behind the
//! `rusty-tokio` feature, [`AsyncTlsStream`] is the same thing over
//! `rusty_tokio`'s `AsyncRead + AsyncWrite`.
//!
//! Server side: [`TlsAcceptor`] (a certificate + private key, built once)
//! and [`TlsServerStream`], the sync per-connection wrapper it produces.
//! `new_with_client_auth` additionally requires and verifies a client
//! certificate against caller-supplied client-CA roots, pairing with the
//! client side's `new_with_client_identity` for full mTLS; `new_with_alpn`
//! mirrors the client side's ALPN support. Behind the `rusty-tokio`
//! feature, [`AsyncTlsServerStream`] is the async counterpart, produced by
//! [`TlsAcceptor::accept_async`] — see the crate's `ARCHITECTURE.md` for
//! the full roadmap and what's deliberately not built.
//!
//! # Example
//!
//! ```no_run
//! use std::net::TcpStream;
//! use std::io::Write;
//! use rusty_tls::{TlsStream, TrustPolicy};
//!
//! let sock = TcpStream::connect("example.com:443")?;
//! let mut tls = TlsStream::new(sock, "example.com", &TrustPolicy::System)?;
//! tls.write_all(b"GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg(feature = "rusty-tokio")]
mod async_client;
#[cfg(feature = "rusty-tokio")]
mod async_server;
mod client;
mod connector;
mod danger;
mod error;
mod provider;
mod server;
mod trust;

// The engine compiles with `--cfg rusty_tls_handrolled` and the feature for
// its backend: `handrolled-engine` (`ring`) normally, `native-crypto`
// (first-party primitives, ADR-0005) under `--cfg rusty_tls_native_crypto`.
// The native backend does not need `handrolled-engine`, so a native build can
// have no direct `ring` dependency at all, which makes "the engine calls no
// `ring`" something the compiler checks rather than something a reader trusts.
#[cfg(all(
    rusty_tls_handrolled,
    any(
        all(feature = "handrolled-engine", not(rusty_tls_native_crypto)),
        all(feature = "native-crypto", rusty_tls_native_crypto)
    )
))]
pub mod handrolled;

// The native backend's cfg is meaningless without the crates it selects. Unlike
// a feature without its cfg (ADR-0002 §2), this cannot arise from
// `--all-features`: only the person running the build sets a cfg, so an error
// here reaches exactly the person who can fix it.
#[cfg(all(rusty_tls_native_crypto, not(feature = "native-crypto")))]
compile_error!("--cfg rusty_tls_native_crypto needs the `native-crypto` feature (ADR-0005)");

/// The hand-rolled engine is *not* compiled into this build.
///
/// You are seeing this stub because the `handrolled-engine` (or
/// `native-crypto`) cargo feature is enabled but `--cfg rusty_tls_handrolled`
/// is not set, or because only `native-crypto` is enabled without
/// `--cfg rusty_tls_native_crypto` (the `ring` backend then has no `ring` to
/// use). A feature and its cfg are both required,
/// and the cfg is the one that carries the guarantee: cargo features are
/// unified across a dependency graph, so a feature alone would let any crate
/// in a consumer's tree enable a hand-rolled TLS implementation for every
/// other crate in that build. A `--cfg` flag comes from `RUSTFLAGS` and
/// cannot be set by a dependency.
///
/// To actually build it:
///
/// ```text
/// RUSTFLAGS='--cfg rusty_tls_handrolled' \
///     cargo test --features handrolled-engine
/// ```
///
/// Before you do: this is an opt-in experiment, permanently non-default, and
/// `rustls` remains the engine behind every type this crate exports. See
/// `docs/adr/0002-handrolled-engine-behind-a-permanently-non-default-seam.md`.
#[cfg(all(
    any(feature = "handrolled-engine", feature = "native-crypto"),
    not(all(
        rusty_tls_handrolled,
        any(
            all(feature = "handrolled-engine", not(rusty_tls_native_crypto)),
            all(feature = "native-crypto", rusty_tls_native_crypto)
        )
    ))
))]
pub mod handrolled {}

#[cfg(feature = "rusty-tokio")]
pub use async_client::AsyncTlsStream;
#[cfg(feature = "rusty-tokio")]
pub use async_server::AsyncTlsServerStream;
pub use client::TlsStream;
pub use connector::TlsConnector;
pub use error::Error;
pub use server::{TlsAcceptor, TlsServerStream};
pub use trust::TrustPolicy;
