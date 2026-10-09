//! A blocking `Read + Write` client stream over the native engine.
//!
//! This is the shape a consumer such as `rleval-app`'s sign-in transport
//! already uses [`TlsStream`](crate::TlsStream) for: wrap a socket, hand it a
//! host name and a [`TrustPolicy`], read and write bytes. It exists so that
//! consumer could be pointed at the native engine without changing how it is
//! written, and so the engine can be exercised the way a consumer would use it.
//!
//! It is **not** the seam. [`TlsStream`](crate::TlsStream) is still rustls, this
//! type is reachable only under both gates, and nothing in this crate chooses it
//! for a caller (ADR-0002 section 3). Using it is a deliberate, local act in the
//! consumer's own code.
//!
//! # Behaviour
//!
//! - The handshake runs lazily on the first read or write, or on
//!   [`NativeTlsStream::complete_handshake`], and blocks on the socket.
//! - Both TLS versions are offered ([`ClientHandshakeBoth`]); the peer picks.
//! - Only [`TrustPolicy::System`] and [`TrustPolicy::PinnedAnchors`] are
//!   supported. Every other policy is refused at construction, because the
//!   engine has no way to skip or relax verification and this type will not
//!   grow one.
//! - A peer that closes the TCP connection without `close_notify` is a
//!   truncation: [`Read::read`] returns `UnexpectedEof`, not `Ok(0)`. Only an
//!   authenticated `close_notify` is a clean end of stream.
//! - Once the peer's `close_notify` has been read, nothing further is delivered
//!   (RFC 8446 section 6.1, RFC 5246 section 7.2.1).
//! - Handshake and control flights (a HelloRetryRequest answer, a KeyUpdate
//!   reply, `close_notify`) are flushed before the stream waits on the peer, so
//!   a buffering transport cannot sit on them.
//! - After any failure the stream stays failed. That includes a transport write
//!   or flush error: the record layer's sequence number has already moved, so
//!   lost ciphertext cannot be recovered, and later calls do no I/O. A
//!   `shutdown` that did not deliver `close_notify` keeps reporting the error.
//! - ALPN offers are checked before any I/O ([`NativeTlsStream::with_alpn`]).

use std::io::{self, Read, Write};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use super::client::{CipherSuite, ClientConfig, ClientError, Incoming};
use super::client12::{CipherSuite12, Incoming12};
use super::kx::NamedGroup;
use super::name::ServerName;
use super::negotiate::{ClientConfigBoth, ClientHandshakeBoth, Established};
use super::path::{PathOptions, TrustAnchor};
use super::record::MAX_FRAGMENT_LEN;
use super::x509::Certificate;
use crate::{Error, TrustPolicy};

/// The groups offered: the same three the rest of the engine's tests use.
const GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

/// The largest record body accepted: a TLS 1.2 ciphertext (2^14 + 2048).
/// Checked before allocating, so a hostile length cannot reserve 64 KiB at will.
const MAX_RECORD_BODY: usize = (1 << 14) + 2048;

/// The most ALPN data accepted: the sum over protocols of (1 + length).
///
/// RFC 7301 allows up to 2^16 - 1, but the ClientHello has to fit one record,
/// and real offers are a few dozen bytes. This is a generous, safe bound, not
/// the protocol's.
const MAX_ALPN_LIST: usize = 4096;

/// Where the stream is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No byte has been exchanged yet.
    Pending,
    /// Established and readable.
    Open,
    /// The peer's `close_notify` was read: end of stream.
    PeerClosed,
    /// A handshake or protocol failure. Permanent.
    Failed,
}

/// What one record turned out to be.
enum Step {
    Data(Vec<u8>),
    Reply(Vec<u8>),
    Closed,
    Nothing,
}

/// A blocking TLS client stream on the native engine.
pub struct NativeTlsStream<S> {
    sock: S,
    server_name: String,
    anchors: Vec<Vec<u8>>,
    alpn: Vec<Vec<u8>>,
    conn: Option<Established>,
    phase: Phase,
    /// This side has sent `close_notify`.
    sent_close: bool,
    /// Decrypted bytes not yet handed to the caller, from `taken` on.
    plain: Vec<u8>,
    taken: usize,
}

impl<S> std::fmt::Debug for NativeTlsStream<S> {
    /// Says nothing about key material, for the reason [`super::kx`] gives.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTlsStream")
            .field("server_name", &self.server_name)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl<S: Read + Write> NativeTlsStream<S> {
    /// Wrap `sock` in a client connection to `server_name`, trusted according to
    /// `policy`. Performs no I/O: the handshake runs on first use.
    ///
    /// Only [`TrustPolicy::System`] and [`TrustPolicy::PinnedAnchors`] are
    /// supported; anything else is an `Unsupported` I/O error.
    pub fn new(sock: S, server_name: &str, policy: &TrustPolicy) -> Result<Self, Error> {
        if server_name.is_empty() {
            return Err(Error::InvalidServerName(server_name.to_string()));
        }
        let anchors: Vec<Vec<u8>> =
            match policy {
                TrustPolicy::System => crate::trust::system_anchors()?,
                TrustPolicy::PinnedAnchors(roots) => {
                    roots.iter().map(|r| r.as_ref().to_vec()).collect()
                }
                _ => return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the native engine supports only the System and PinnedAnchors trust policies",
                ))),
            };
        if anchors.is_empty() {
            return Err(Error::NoTrustAnchors);
        }
        Ok(Self {
            sock,
            server_name: server_name.to_string(),
            anchors,
            alpn: Vec::new(),
            conn: None,
            phase: Phase::Pending,
            sent_close: false,
            plain: Vec::new(),
            taken: 0,
        })
    }

    /// Offer these ALPN protocols, most preferred first. Has no effect once the
    /// handshake has started.
    ///
    /// Each name must be 1 to 255 bytes and the whole offer at most
    /// [`MAX_ALPN_LIST`] bytes once framed (a length byte per name); otherwise
    /// `InvalidInput`, before any I/O. The engine's encoder trusts its caller,
    /// so this is where outside input is checked.
    pub fn with_alpn(mut self, protocols: Vec<Vec<u8>>) -> io::Result<Self> {
        let mut framed = 0usize;
        for protocol in &protocols {
            if protocol.is_empty() || protocol.len() > usize::from(u8::MAX) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an ALPN protocol name must be 1 to 255 bytes",
                ));
            }
            framed += 1 + protocol.len();
        }
        if framed > MAX_ALPN_LIST {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the ALPN offer is too large",
            ));
        }
        self.alpn = protocols;
        Ok(self)
    }

    /// Whether the handshake has not yet completed.
    pub fn is_handshaking(&self) -> bool {
        self.phase == Phase::Pending
    }

    /// Run the handshake now, if it has not run, and report its outcome.
    pub fn complete_handshake(&mut self) -> io::Result<()> {
        match self.phase {
            Phase::Pending => {}
            Phase::Failed => return Err(failed()),
            Phase::Open | Phase::PeerClosed => return Ok(()),
        }
        match self.run_handshake() {
            Ok(conn) => {
                self.conn = Some(conn);
                self.phase = Phase::Open;
                Ok(())
            }
            Err(err) => {
                self.phase = Phase::Failed;
                Err(err)
            }
        }
    }

    /// The protocol selected by ALPN, once the handshake has completed.
    pub fn negotiated_alpn_protocol(&self) -> Option<&[u8]> {
        self.conn.as_ref().and_then(Established::alpn_protocol)
    }

    /// The DER end-entity certificate the server presented, once the handshake
    /// has completed.
    pub fn peer_certificate_der(&self) -> Option<&[u8]> {
        match self.conn.as_ref()? {
            Established::Tls13(c) => c.peer_certificates().first().map(Vec::as_slice),
            Established::Tls12(c) => c.peer_certificates().first().map(Vec::as_slice),
            _ => None,
        }
    }

    /// Borrow the underlying stream. Does not touch TLS state.
    pub fn get_ref(&self) -> &S {
        &self.sock
    }

    /// Mutably borrow the underlying stream. Reading or writing it directly
    /// bypasses TLS and corrupts the session; this is for socket options.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.sock
    }

    /// Consume `self`, returning the underlying stream. The session is discarded.
    pub fn into_inner(self) -> S {
        self.sock
    }

    /// Send `close_notify`, the orderly end of this side's writing. The peer's
    /// own `close_notify` can still be read. Does nothing before the handshake
    /// or after it has been delivered once.
    ///
    /// If the close cannot be delivered the stream fails for good, and every
    /// later call, this one included, reports it: a retry never claims success
    /// for a `close_notify` that did not go out.
    pub fn shutdown(&mut self) -> io::Result<()> {
        if self.conn.is_none() || self.sent_close {
            return Ok(());
        }
        if self.phase == Phase::Failed {
            return Err(failed());
        }
        let sealed = match self.conn.as_mut() {
            Some(Established::Tls13(c)) => c.close(),
            Some(Established::Tls12(c)) => c.close(),
            _ => return Err(unsupported()),
        };
        let sealed = match sealed {
            Ok(sealed) => sealed,
            Err(err) => return Err(self.fail(&err)),
        };
        self.deliver(&sealed, true)?;
        self.sent_close = true;
        Ok(())
    }

    /// The handshake proper, with every borrow it needs kept local so the
    /// resulting [`Established`] owns its state.
    fn run_handshake(&mut self) -> io::Result<Established> {
        // An anchor the parser cannot read is skipped: a trust store with one odd
        // entry should not stop the rest from being used. None usable is an error.
        let certificates: Vec<Certificate<'_>> = self
            .anchors
            .iter()
            .filter_map(|der| Certificate::parse(der).ok())
            .collect();
        if certificates.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no usable trust anchors",
            ));
        }
        let anchors: Vec<TrustAnchor<'_>> = certificates
            .iter()
            .map(TrustAnchor::from_certificate)
            .collect();
        let name = match self.server_name.parse::<IpAddr>() {
            Ok(ip) => ServerName::Ip(ip),
            Err(_) => ServerName::Dns(&self.server_name),
        };
        let alpn: Vec<&[u8]> = self.alpn.iter().map(Vec::as_slice).collect();
        let tls13 = ClientConfig {
            server_name: name,
            anchors: &anchors,
            path: PathOptions {
                time: now()?,
                ..PathOptions::default()
            },
            groups: GROUPS,
            cipher_suites: CipherSuite::SUPPORTED,
            identity: None,
            resumption: None,
            alpn: &alpn,
        };
        let config = ClientConfigBoth::new(tls13, CipherSuite12::SUPPORTED);

        let (mut handshake, hello) = ClientHandshakeBoth::start(&config).map_err(invalid)?;
        send(&mut self.sock, &hello)?;
        while !handshake.is_finished() {
            let record = read_record(&mut self.sock)?;
            match handshake.read_record(&record) {
                Ok(reply) if reply.is_empty() => {}
                Ok(reply) => send(&mut self.sock, &reply)?,
                Err(err) => {
                    // Tell the peer why, best effort, then report it.
                    if let Some(alert) = handshake.alert_record(&err) {
                        let _ = send(&mut self.sock, &alert);
                    }
                    return Err(invalid(err));
                }
            }
        }
        handshake.into_connection().map_err(invalid)
    }

    /// Fail the stream for good, after telling the peer why where there is a way to.
    fn fail(&mut self, err: &ClientError) -> io::Error {
        self.phase = Phase::Failed;
        if let Some(alert) = self.conn.as_mut().and_then(|c| c.alert_record(err)) {
            let _ = send(&mut self.sock, &alert);
        }
        invalid(err)
    }

    /// Put sealed bytes on the wire. A failure is permanent: the record layer has
    /// already advanced its sequence number, so ciphertext that did not all go out
    /// cannot be recovered and anything sent after it would be out of step.
    /// Control flights (`flush`) are pushed through any buffering so the peer can
    /// answer before we block on it.
    fn deliver(&mut self, bytes: &[u8], flush: bool) -> io::Result<()> {
        let result = if flush {
            send(&mut self.sock, bytes)
        } else {
            self.sock.write_all(bytes)
        };
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }
}

impl<S: Read + Write> Read for NativeTlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.complete_handshake()?;
        loop {
            if self.taken < self.plain.len() {
                let n = buf.len().min(self.plain.len() - self.taken);
                buf[..n].copy_from_slice(&self.plain[self.taken..self.taken + n]);
                self.taken += n;
                return Ok(n);
            }
            self.plain.clear();
            self.taken = 0;
            match self.phase {
                Phase::PeerClosed => return Ok(0),
                Phase::Failed => return Err(failed()),
                Phase::Pending | Phase::Open => {}
            }

            // EOF here, without a close_notify, is a truncation and surfaces as
            // `UnexpectedEof` from `read_exact`.
            let record = match read_record(&mut self.sock) {
                Ok(record) => record,
                Err(err) => {
                    self.phase = Phase::Failed;
                    return Err(err);
                }
            };
            let conn = self.conn.as_mut().ok_or_else(failed)?;
            match step(conn, &record) {
                Ok(Step::Data(data)) => self.plain = data,
                Ok(Step::Reply(reply)) => self.deliver(&reply, true)?,
                Ok(Step::Closed) => self.phase = Phase::PeerClosed,
                Ok(Step::Nothing) => {}
                Err(StepError::Unsupported) => {
                    self.phase = Phase::Failed;
                    return Err(unsupported());
                }
                Err(StepError::Engine(err)) => return Err(self.fail(&err)),
            }
        }
    }
}

impl<S: Read + Write> Write for NativeTlsStream<S> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        self.complete_handshake()?;
        if self.sent_close || self.phase == Phase::PeerClosed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        // One record per call: `write` may be partial, and the TLS 1.3 connection
        // seals a single record at a time. `write_all` loops over the rest.
        let chunk = &data[..data.len().min(MAX_FRAGMENT_LEN)];
        let sealed = match self.conn.as_mut() {
            Some(Established::Tls13(c)) => c.write(chunk),
            Some(Established::Tls12(c)) => c.write(chunk),
            _ => return Err(unsupported()),
        };
        let sealed = match sealed {
            Ok(sealed) => sealed,
            Err(err) => return Err(self.fail(&err)),
        };
        self.deliver(&sealed, false)?;
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.phase == Phase::Failed {
            return Err(failed());
        }
        let result = self.sock.flush();
        if result.is_err() {
            // Whatever was buffered below us may be gone.
            self.phase = Phase::Failed;
        }
        result
    }
}

/// Why one record could not be handled.
enum StepError {
    Engine(ClientError),
    Unsupported,
}

/// Unprotect one record and say what it was.
fn step(conn: &mut Established, record: &[u8]) -> Result<Step, StepError> {
    let outcome = match conn {
        Established::Tls13(c) => c.read(record).map(|incoming| match incoming {
            Incoming::Application(data) => Step::Data(data),
            Incoming::Reply(reply) => Step::Reply(reply),
            Incoming::Closed => Step::Closed,
            // Resumption tickets are not kept by this stream.
            _ => Step::Nothing,
        }),
        Established::Tls12(c) => c.read(record).map(|incoming| match incoming {
            Incoming12::Application(data) => Step::Data(data),
            Incoming12::Reply(reply) => Step::Reply(reply),
            Incoming12::Closed => Step::Closed,
            _ => Step::Nothing,
        }),
        _ => return Err(StepError::Unsupported),
    };
    outcome.map_err(StepError::Engine)
}

/// Write all of `bytes` and flush, so a buffering transport cannot hold a
/// handshake or control flight while we wait for the answer.
fn send<S: Write>(sock: &mut S, bytes: &[u8]) -> io::Result<()> {
    sock.write_all(bytes)?;
    sock.flush()
}

/// Read one whole record, header included.
fn read_record<S: Read>(sock: &mut S) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 5];
    sock.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    if length > MAX_RECORD_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a record larger than any TLS version allows",
        ));
    }
    let mut record = header.to_vec();
    record.resize(5 + length, 0);
    sock.read_exact(&mut record[5..])?;
    Ok(record)
}

fn now() -> io::Result<i64> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::other("the system clock is before 1970"))?
        .as_secs();
    i64::try_from(secs).map_err(|_| io::Error::other("the system clock is out of range"))
}

fn invalid(err: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}

fn failed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "the TLS connection has failed")
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "a connection kind this stream does not know",
    )
}
