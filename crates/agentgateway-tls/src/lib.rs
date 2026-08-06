//! TLS termination, over [`rusty_tls`].
//!
//! [`rusty_tls`] is the ecosystem's one TLS implementation — a `rustls` 0.23
//! wrapper — so the gateway does not roll its own. Its async server adapter is
//! written against `rusty_tokio`'s I/O traits rather than `tokio`'s, which is
//! what [`bridge`] exists to reconcile; that adaptation is copy-free, so the
//! choice costs dependency weight rather than throughput.
//!
//! # What this does not do yet
//!
//! **SNI-based certificate selection.** One certificate is served per bind.
//! `rusty_tls` exposes a `TlsAcceptor` built from a single certificate chain
//! and does not surface `rustls`' `ResolvesServerCert`, so two listeners on
//! one port with different certificates cannot both be honoured — and quietly
//! serving the first one's certificate to the second one's clients would be a
//! misconfiguration nobody notices until a browser complains. [`TlsBinds`]
//! rejects it at startup instead.

mod bridge;

use std::sync::Once;

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use agentgateway_config::{Config, Protocol, TlsConfig};
use rusty_tls::TlsAcceptor;
use tokio::net::TcpStream;

pub use bridge::{ToRusty, ToTokio};

/// The decrypted stream handed back to `hyper`.
pub type TlsStream = ToTokio<rusty_tls::AsyncTlsServerStream<ToRusty<TcpStream>>>;

/// Protocols advertised over ALPN, most preferred first.
///
/// `hyper`'s auto builder serves either, and a client that negotiates `h2`
/// here gets HTTP/2 rather than falling back — which is the whole reason to
/// advertise it, since over TLS ALPN is how the version is chosen.
const ALPN: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// Select the crypto provider, once per process.
///
/// `rustls` picks a provider from crate features when none is installed, and
/// refuses to guess when more than one is present — which is this workspace,
/// since `reqwest` brings `aws-lc-rs` alongside `rusty_tls`' `ring`. Without
/// this it panics on the first handshake, not at startup.
///
/// Ignoring the error is deliberate: it means something already installed a
/// provider, and that choice is as good as this one.
fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Failure to set up TLS termination.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// A certificate or key file could not be read.
    #[error("{at}: reading {path}: {source}")]
    Io {
        /// Where in the configuration it came from.
        at: String,
        /// The file we tried to read.
        path: String,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The PEM held no certificates, or no private key.
    #[error("{at}: {path} contains no {kind}")]
    Empty {
        /// Where in the configuration it came from.
        at: String,
        /// The file we parsed.
        path: String,
        /// `certificates` or `private key`.
        kind: &'static str,
    },

    /// `rusty_tls` refused the certificate and key.
    #[error("{at}: building the TLS acceptor: {source}")]
    Acceptor {
        /// Where in the configuration it came from.
        at: String,
        /// Underlying failure.
        #[source]
        source: Box<rusty_tls::Error>,
    },

    /// A listener asks for TLS without naming a certificate.
    #[error("{at}: protocol is {protocol:?} but no `tls` certificate is configured")]
    Missing {
        /// Where in the configuration it came from.
        at: String,
        /// The protocol that needs a certificate.
        protocol: Protocol,
    },

    /// Two listeners on one port want different certificates.
    #[error(
        "bind on port {port} has listeners with different TLS certificates, which needs \
         SNI-based selection; this build serves one certificate per port"
    )]
    Sni {
        /// The port with conflicting certificates.
        port: u16,
    },
}

/// The TLS terminator for one port.
pub struct TlsTerminator {
    acceptor: TlsAcceptor,
}

impl std::fmt::Debug for TlsTerminator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsTerminator").finish_non_exhaustive()
    }
}

impl TlsTerminator {
    /// Load a certificate and key from PEM files.
    ///
    /// Read once at startup, so a missing or malformed certificate stops the
    /// gateway booting rather than failing every handshake later.
    pub fn new(tls: &TlsConfig, at: &str) -> Result<Self, TlsError> {
        install_crypto_provider();
        let certs = load_certs(&tls.cert, at)?;
        let key = load_key(&tls.key, at)?;

        let acceptor = TlsAcceptor::new_with_alpn(
            certs,
            key,
            ALPN.iter().map(|p| p.to_vec()).collect(),
        )
        .map_err(|source| TlsError::Acceptor {
            at: at.to_string(),
            source: Box::new(source),
        })?;

        Ok(TlsTerminator { acceptor })
    }

    /// Complete a TLS handshake on an accepted socket.
    pub async fn accept(&self, stream: TcpStream) -> Result<TlsStream, rusty_tls::Error> {
        let mut tls = self.acceptor.accept_async(ToRusty(stream))?;
        // Drive the handshake here rather than letting the first read do it,
        // so a failed handshake is reported as one instead of surfacing as a
        // confusing empty request.
        tls.complete_handshake().await?;
        Ok(ToTokio(tls))
    }
}

/// The terminators a configuration needs, by port.
#[derive(Debug, Default)]
pub struct TlsBinds {
    ports: BTreeMap<u16, Arc<TlsTerminator>>,
}

impl TlsBinds {
    /// Build a terminator for every bind that has a TLS listener.
    pub fn build(config: &Config) -> Result<Self, TlsError> {
        let mut ports: BTreeMap<u16, Arc<TlsTerminator>> = BTreeMap::new();

        for (b, bind) in config.binds.iter().enumerate() {
            let mut chosen: Option<&TlsConfig> = None;

            for (l, listener) in bind.listeners.iter().enumerate() {
                if !listener.protocol.is_tls() {
                    continue;
                }
                let at = format!("binds[{b}].listeners[{l}]");
                let Some(tls) = listener.tls.as_ref() else {
                    return Err(TlsError::Missing {
                        at,
                        protocol: listener.protocol,
                    });
                };

                match chosen {
                    // Same certificate twice is fine; two different ones on
                    // one port would need SNI to tell apart.
                    Some(existing) if existing != tls => {
                        return Err(TlsError::Sni { port: bind.port });
                    }
                    Some(_) => {}
                    None => chosen = Some(tls),
                }
            }

            if let Some(tls) = chosen {
                let at = format!("binds[{b}]");
                ports.insert(bind.port, Arc::new(TlsTerminator::new(tls, &at)?));
            }
        }

        Ok(TlsBinds { ports })
    }

    /// The terminator serving `port`, if it terminates TLS.
    pub fn get(&self, port: u16) -> Option<Arc<TlsTerminator>> {
        self.ports.get(&port).cloned()
    }

    /// Ports that terminate TLS.
    pub fn ports(&self) -> impl Iterator<Item = u16> + '_ {
        self.ports.keys().copied()
    }

    /// Whether any bind terminates TLS.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }
}

fn load_certs(path: &str, at: &str) -> Result<Vec<Vec<u8>>, TlsError> {
    let file = open(path, at)?;
    let certs: Vec<Vec<u8>> = rustls_pemfile::certs(&mut BufReader::new(file))
        .filter_map(Result::ok)
        .map(|der| der.to_vec())
        .collect();

    if certs.is_empty() {
        return Err(TlsError::Empty {
            at: at.to_string(),
            path: path.to_string(),
            kind: "certificates",
        });
    }
    Ok(certs)
}

fn load_key(path: &str, at: &str) -> Result<Vec<u8>, TlsError> {
    let file = open(path, at)?;
    // `private_key` accepts PKCS#8, PKCS#1 and SEC1 alike, so an operator does
    // not have to know which one their tooling emitted.
    let key = rustls_pemfile::private_key(&mut BufReader::new(file)).map_err(|source| {
        TlsError::Io {
            at: at.to_string(),
            path: path.to_string(),
            source,
        }
    })?;

    key.map(|key| key.secret_der().to_vec())
        .ok_or_else(|| TlsError::Empty {
            at: at.to_string(),
            path: path.to_string(),
            kind: "private key",
        })
}

fn open(path: &str, at: &str) -> Result<std::fs::File, TlsError> {
    std::fs::File::open(Path::new(path)).map_err(|source| TlsError::Io {
        at: at.to_string(),
        path: path.to_string(),
        source,
    })
}
