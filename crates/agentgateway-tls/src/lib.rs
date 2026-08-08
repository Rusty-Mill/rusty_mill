//! TLS termination, over [`rusty_tls`].
//!
//! [`rusty_tls`] is the ecosystem's one TLS implementation — a `rustls` 0.23
//! wrapper — so the gateway does not roll its own. Its async server adapter is
//! written against `rusty_tokio`'s I/O traits rather than `tokio`'s, which is
//! what [`bridge`] exists to reconcile; that adaptation is copy-free, so the
//! choice costs dependency weight rather than throughput.
//!
//! # One certificate per hostname
//!
//! Two listeners on one port may hold different certificates, chosen by the
//! name the client asked for. `rusty_tls` exposes a `TlsAcceptor` built from a
//! single certificate chain and does not surface `rustls`' `ResolvesServerCert`
//! — so rather than reaching around it into `rustls`, the name is read off the
//! ClientHello before the handshake starts and the matching acceptor does the
//! rest. See [`hello`] for why that is the safe half of the trade.

mod bridge;
mod hello;

use std::sync::Once;

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use agentgateway_config::{Config, Protocol, TlsConfig};
use agentgateway_core::HostnamePattern;
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

    /// Two listeners on one port hold certificates and neither can be chosen.
    #[error(
        "bind on port {port} has {count} listeners with different TLS certificates and no \
         hostname to tell them apart; a client's SNI name is what chooses between them, so \
         each needs its own `hostname`"
    )]
    Sni {
        /// The port with conflicting certificates.
        port: u16,
        /// How many certificates are in play.
        count: usize,
    },

    /// Two listeners on one port claim the same hostname.
    #[error(
        "bind on port {port} has two listeners serving `{hostname}` with different \
         certificates; a client asking for that name could be given either"
    )]
    Duplicate {
        /// The port.
        port: u16,
        /// The name claimed twice.
        hostname: String,
    },
}

/// One certificate, and the name it answers to.
struct Certificate {
    /// `None` for a listener with no `hostname`, which answers to anything.
    hostname: Option<HostnamePattern>,
    acceptor: TlsAcceptor,
}

/// The TLS terminator for one port.
pub struct TlsTerminator {
    /// Most specific first, so the first match is the right one.
    certificates: Vec<Certificate>,
}

impl std::fmt::Debug for TlsTerminator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsTerminator")
            .field("certificates", &self.certificates.len())
            .finish_non_exhaustive()
    }
}

impl TlsTerminator {
    /// Load a certificate and key from PEM files.
    ///
    /// Read once at startup, so a missing or malformed certificate stops the
    /// gateway booting rather than failing every handshake later.
    pub fn new(tls: &TlsConfig, at: &str) -> Result<Self, TlsError> {
        Self::with_hostnames([(None, tls)], at)
    }

    /// The same, for a port serving several certificates by name.
    fn with_hostnames<'a>(
        listeners: impl IntoIterator<Item = (Option<&'a str>, &'a TlsConfig)>,
        at: &str,
    ) -> Result<Self, TlsError> {
        install_crypto_provider();

        let mut certificates = Vec::new();
        for (hostname, tls) in listeners {
            let certs = load_certs(&tls.cert, at)?;
            let key = load_key(&tls.key, at)?;
            let acceptor =
                TlsAcceptor::new_with_alpn(certs, key, ALPN.iter().map(|p| p.to_vec()).collect())
                    .map_err(|source| TlsError::Acceptor {
                    at: at.to_string(),
                    source: Box::new(source),
                })?;
            certificates.push(Certificate {
                hostname: hostname.map(HostnamePattern::parse),
                acceptor,
            });
        }

        // Exact names before wildcards before the catch-all, so `api.example`
        // wins over `*.example` for a client that asked for `api.example` --
        // the same precedence route hostnames follow.
        certificates.sort_by_key(|certificate| {
            std::cmp::Reverse(
                certificate
                    .hostname
                    .as_ref()
                    .map_or(0, HostnamePattern::specificity),
            )
        });

        Ok(TlsTerminator { certificates })
    }

    /// Complete a TLS handshake on an accepted socket.
    ///
    /// The name the client asked for is read from the ClientHello *before* the
    /// handshake begins, by peeking rather than reading: the bytes stay in the
    /// socket for `rustls` to parse itself a moment later. See [`hello`].
    pub async fn accept(&self, stream: TcpStream) -> Result<TlsStream, rusty_tls::Error> {
        let acceptor = match self.certificates.len() {
            // Nothing to choose between, so nothing to peek for.
            0 | 1 => self.first()?,
            _ => self.choose(peek_server_name(&stream).await.as_deref()),
        };

        let mut tls = acceptor.accept_async(ToRusty(stream))?;
        // Drive the handshake here rather than letting the first read do it,
        // so a failed handshake is reported as one instead of surfacing as a
        // confusing empty request.
        tls.complete_handshake().await?;
        Ok(ToTokio(tls))
    }

    /// The only certificate, when there is one to be had.
    ///
    /// A terminator is never built empty -- `TlsBinds` does not register a
    /// port without a certificate -- but returning the handshake's own error
    /// beats an index that could panic in the accept loop.
    fn first(&self) -> Result<&TlsAcceptor, rusty_tls::Error> {
        self.certificates
            .first()
            .map(|certificate| &certificate.acceptor)
            .ok_or_else(|| {
                rusty_tls::Error::Io(std::io::Error::other(
                    "no TLS certificate is configured for this port",
                ))
            })
    }

    /// The certificate for a name, or the fallback.
    ///
    /// A client that sent no name — every one addressing the gateway by IP —
    /// and one asking for a name nothing claims both get the first certificate
    /// rather than a refusal. Refusing would turn a working single-certificate
    /// deployment into a broken one the moment a second listener was added.
    fn choose(&self, name: Option<&str>) -> &TlsAcceptor {
        if let Some(name) = name
            && let Some(matched) = self.certificates.iter().find(|certificate| {
                certificate
                    .hostname
                    .as_ref()
                    .is_some_and(|pattern| pattern.matches(name))
            })
        {
            return &matched.acceptor;
        }
        self.certificates
            .first()
            .map(|certificate| &certificate.acceptor)
            .expect("`choose` is only reached with several certificates")
    }
}

/// Read the ClientHello's server name without consuming it.
///
/// `peek` leaves the bytes in the socket, which is what makes this safe to do
/// in front of a handshake: `rustls` reads the same ClientHello afterwards and
/// is still the thing that decides whether it is acceptable.
///
/// A peer that sends nothing, or never sends a whole ClientHello, ends this
/// with `None` and gets the default certificate — the same outcome as before
/// any of this existed.
///
/// The wait is bounded because `peek` returns whatever is buffered *now*: a
/// ClientHello split across two segments arrives in two peeks, and a peer that
/// sends the first half and stops would otherwise be waited on forever. The
/// budget is the one this spends looking, not a handshake timeout.
async fn peek_server_name(stream: &TcpStream) -> Option<String> {
    tokio::time::timeout(PEEK_BUDGET, async {
        let mut buffer = vec![0u8; hello::MAX_HELLO];
        let mut filled = 0;

        loop {
            let read = stream.peek(&mut buffer[..]).await.ok()?;
            if read == 0 {
                // The peer closed. Nothing more is coming.
                return None;
            }
            if read > filled {
                filled = read;
                if let Some(name) = hello::server_name(&buffer[..filled]) {
                    return Some(name);
                }
                if filled >= buffer.len() {
                    return None;
                }
                continue;
            }
            // No new bytes. `peek` is readable-triggered and the buffered
            // bytes keep it readable, so polling again immediately would spin
            // rather than wait -- hence a pause between looks.
            tokio::time::sleep(PEEK_POLL).await;
        }
    })
    .await
    .ok()
    .flatten()
}

/// How long to spend looking for the server name before giving up on it.
const PEEK_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// How long to wait when a peek brought nothing new.
const PEEK_POLL: std::time::Duration = std::time::Duration::from_millis(2);

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
            let mut chosen: Vec<(Option<&str>, &TlsConfig)> = Vec::new();

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

                let hostname = listener.hostname.as_deref();
                // The same certificate twice is one certificate, whatever the
                // hostnames: there is nothing for a name to choose between.
                if chosen.iter().any(|(_, existing)| *existing == tls) {
                    continue;
                }
                // Two certificates and the same name is a coin toss at
                // handshake time, which is exactly the misconfiguration this
                // used to refuse wholesale.
                if let Some(hostname) = hostname
                    && chosen.iter().any(|(name, _)| *name == Some(hostname))
                {
                    return Err(TlsError::Duplicate {
                        port: bind.port,
                        hostname: hostname.to_string(),
                    });
                }
                chosen.push((hostname, tls));
            }

            // More than one certificate needs names to choose between them,
            // and more than one *unnamed* certificate has none: whichever
            // sorted first would answer for both.
            if chosen.len() > 1 && chosen.iter().filter(|(name, _)| name.is_none()).count() > 1 {
                return Err(TlsError::Sni {
                    port: bind.port,
                    count: chosen.len(),
                });
            }

            if !chosen.is_empty() {
                let at = format!("binds[{b}]");
                ports.insert(
                    bind.port,
                    Arc::new(TlsTerminator::with_hostnames(chosen, &at)?),
                );
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
    let key =
        rustls_pemfile::private_key(&mut BufReader::new(file)).map_err(|source| TlsError::Io {
            at: at.to_string(),
            path: path.to_string(),
            source,
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
