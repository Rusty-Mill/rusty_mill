//! Version negotiation: one endpoint that speaks TLS 1.3 and TLS 1.2 — stage
//! 4b-v.
//!
//! The two versions have separate state machines ([`super::client`],
//! [`super::client12`], [`super::server`], [`super::server12`]) because they
//! share almost nothing past the record header. This module is the seam between
//! them, and it is small on purpose: it chooses, and then it steps aside.
//!
//! ```text
//!   client                                        server
//!   ClientHello  (offers 1.3 and 1.2)  ------>    reads the whole hello,
//!                                                 chooses, replays it into
//!                                                 one machine
//!               <------  ServerHello              1.3 if it was offered,
//!   reads the whole ServerHello,                  otherwise 1.2 with the
//!   chooses, replays it into one machine          DOWNGRD sentinel
//! ```
//!
//! # Why a choice must be made on a whole message
//!
//! A hello can arrive in pieces, and its version is in an extension at the
//! end. Both sides therefore buffer records until the first handshake message
//! is complete, decide, and feed the buffered records to the machine chosen, so
//! that machine sees exactly the bytes a peer speaking only its version would
//! have sent. Nothing is parsed twice with different rules: the chosen machine
//! applies its own, strictly, to the same record bytes.
//!
//! # Downgrade protection
//!
//! This is the property the TLS 1.2 track gave up by learning to speak
//! something older, and what replaces it (RFC 8446 §4.1.3):
//!
//! - **A server that speaks both** and answers a client in 1.2 writes
//!   `DOWNGRD\x01` in the last eight octets of `ServerHello.random`
//!   ([`ServerHandshake12`]'s `downgraded` constructor, used here and nowhere
//!   else).
//! - **A client that offered both** and is answered in 1.2 refuses a
//!   `ServerHello` that carries it ([`ClientError::DowngradeDetected`]). The
//!   sentinel is signed in the `ServerKeyExchange`, so an attacker who strips
//!   1.3 from the hello cannot also strip the evidence.
//! - **A client that offers only 1.2** ([`ClientHandshake12`] alone) must *not*
//!   refuse it: a 1.2-only client of a 1.3-capable server is told the truth,
//!   and is not under attack.
//! - **`TLS_FALLBACK_SCSV`** (RFC 7507): a 1.2-only hello that signals a
//!   fallback is refused by this server with `inappropriate_fallback`. The
//!   sentinel protects clients that offer 1.3; the SCSV protects the ones that
//!   retry with less after a failure an attacker caused.
//!
//! # What the server decides, and how
//!
//! 1.3 if `supported_versions` lists it, 1.2 otherwise. A client that lists
//! 1.3 and has nothing else in common with this server fails; it is not
//! retried in 1.2, because by then the 1.3 machine has consumed the hello.
//!
//! Resumption works only in 1.3: a 1.3 hello carries the `pre_shared_key`, and
//! a 1.2 reply ignores it (the binder is computed over the combined hello).

use super::client::{plaintext_record, ClientConfig, ClientError, ClientHandshake, Connection};
use super::client12::{
    CipherSuite12, ClientConfig12, ClientHandshake12, Connection12, MAX_HANDSHAKE_BUFFER,
};
use super::handshake::{complete_prefix, extension, find, messages, HandshakeType};
use super::handshake12::{ClientHello12, ServerHello12};
use super::record::ContentType;
use super::record12::split;
use super::server::{ServerConfig, ServerError, ServerHandshake};
use super::server12::{ServerConfig12, ServerHandshake12};

/// `TLS_FALLBACK_SCSV`, RFC 7507 §3.
const FALLBACK_SCSV: u16 = 0x5600;
const TLS13: u16 = 0x0304;

/// The version a connection was established in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// TLS 1.2.
    Tls12,
    /// TLS 1.3.
    Tls13,
}

/// An established connection of either version.
#[derive(Debug)]
#[non_exhaustive]
pub enum Established {
    /// TLS 1.3.
    Tls13(Connection),
    /// TLS 1.2.
    Tls12(Connection12),
}

impl Established {
    /// The negotiated version.
    pub const fn version(&self) -> Version {
        match self {
            Self::Tls13(_) => Version::Tls13,
            Self::Tls12(_) => Version::Tls12,
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A client that offers TLS 1.3 and TLS 1.2 in one hello.
///
/// Holds the TLS 1.2 configuration derived from the TLS 1.3 one, so the two
/// cannot disagree about the server's name, the trust anchors, the time, or
/// the groups: a hello that offered a group the 1.2 half would then refuse
/// would be a handshake that fails only against some servers.
pub struct ClientConfigBoth<'a> {
    tls13: ClientConfig<'a>,
    tls12: ClientConfig12<'a>,
}

impl<'a> ClientConfigBoth<'a> {
    /// Combine a TLS 1.3 configuration with the TLS 1.2 suites to also offer.
    pub fn new(tls13: ClientConfig<'a>, suites12: &'a [CipherSuite12]) -> Self {
        let tls12 = ClientConfig12 {
            server_name: tls13.server_name,
            anchors: tls13.anchors,
            path: tls13.path,
            groups: tls13.groups,
            cipher_suites: suites12,
        };
        Self { tls13, tls12 }
    }
}

enum ClientState<'a> {
    /// The hello is sent and no ServerHello is complete yet. Records are kept
    /// so the machine that is chosen sees them all.
    Pending {
        tls13: Box<ClientHandshake<'a>>,
        buffer: Vec<u8>,
        records: Vec<Vec<u8>>,
    },
    Tls13(Box<ClientHandshake<'a>>),
    Tls12(Box<ClientHandshake12<'a>>),
    Failed,
}

/// A client handshake that has not yet learned which version it is in.
pub struct ClientHandshakeBoth<'a> {
    config: &'a ClientConfigBoth<'a>,
    state: ClientState<'a>,
}

impl<'a> ClientHandshakeBoth<'a> {
    /// Start, returning the handshake and the ClientHello record to send.
    pub fn start(config: &'a ClientConfigBoth<'a>) -> Result<(Self, Vec<u8>), ClientError> {
        let (tls13, hello) =
            ClientHandshake::start_offering_tls12(&config.tls13, config.tls12.cipher_suites)?;
        Ok((
            Self {
                config,
                state: ClientState::Pending {
                    tls13: Box::new(tls13),
                    buffer: Vec::new(),
                    records: Vec::new(),
                },
            },
            hello,
        ))
    }

    /// True once the handshake has completed in either version.
    pub fn is_finished(&self) -> bool {
        match &self.state {
            ClientState::Tls13(h) => h.is_finished(),
            ClientState::Tls12(h) => h.is_finished(),
            _ => false,
        }
    }

    /// The version chosen, once the ServerHello has been read.
    pub fn version(&self) -> Option<Version> {
        match &self.state {
            ClientState::Tls13(_) => Some(Version::Tls13),
            ClientState::Tls12(_) => Some(Version::Tls12),
            _ => None,
        }
    }

    /// Take the established connection.
    pub fn into_connection(self) -> Result<Established, ClientError> {
        match self.state {
            ClientState::Tls13(h) => Ok(Established::Tls13((*h).into_connection()?)),
            ClientState::Tls12(h) => Ok(Established::Tls12((*h).into_connection()?)),
            _ => Err(ClientError::Failed),
        }
    }

    /// Feed one whole TLS record, and get back the bytes to send in reply.
    /// A failure is permanent.
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>, ClientError> {
        let result = match &mut self.state {
            ClientState::Tls13(h) => return h.read_record(record),
            ClientState::Tls12(h) => return h.read_record(record),
            ClientState::Failed => return Err(ClientError::Failed),
            ClientState::Pending { .. } => self.read_pending(record),
        };
        if result.is_err() {
            self.state = ClientState::Failed;
        }
        result
    }

    fn read_pending(&mut self, record: &[u8]) -> Result<Vec<u8>, ClientError> {
        let ClientState::Pending {
            buffer, records, ..
        } = &mut self.state
        else {
            return Err(ClientError::Failed);
        };
        let (typ, _, fragment) = split(record)?;
        records.push(record.to_vec());
        // Anything but handshake bytes goes straight to the TLS 1.3 machine,
        // which owns the pre-ServerHello rules for them: it reports an alert
        // by name and drops a middlebox ChangeCipherSpec.
        if typ != ContentType::Handshake {
            return self.decide(Version::Tls13);
        }
        if buffer.len() + fragment.len() > MAX_HANDSHAKE_BUFFER {
            return Err(ClientError::HandshakeTooLarge);
        }
        buffer.extend_from_slice(fragment);
        if complete_prefix(buffer) == 0 {
            return Ok(Vec::new());
        }
        let version = server_hello_version(buffer);
        self.decide(version)
    }

    /// Commit to a version and replay every buffered record into its machine.
    fn decide(&mut self, version: Version) -> Result<Vec<u8>, ClientError> {
        let ClientState::Pending { tls13, records, .. } =
            core::mem::replace(&mut self.state, ClientState::Failed)
        else {
            return Err(ClientError::Failed);
        };
        let mut reply = Vec::new();
        match version {
            Version::Tls13 => {
                let mut machine = tls13;
                for record in &records {
                    reply.extend(machine.read_record(record)?);
                }
                self.state = ClientState::Tls13(machine);
            }
            Version::Tls12 => {
                let (hello, random) = (*tls13).into_tls12_hello()?;
                let mut machine =
                    ClientHandshake12::continue_from(&self.config.tls12, hello, &random)?;
                for record in &records {
                    reply.extend(machine.read_record(record)?);
                }
                self.state = ClientState::Tls12(Box::new(machine));
            }
        }
        Ok(reply)
    }
}

/// Which machine should read a ServerHello that is complete in `buffer`.
///
/// 1.3 if it carries `supported_versions` (a 1.3 ServerHello and a
/// HelloRetryRequest always do), 1.2 if it does not. Anything that does not
/// parse as a ServerHello is sent to the 1.3 machine, whose errors are the
/// more specific ones for a hello in the wrong place.
fn server_hello_version(buffer: &[u8]) -> Version {
    // Only the complete prefix: a record boundary may fall inside the next
    // message, and its partial tail is not this message's to be judged by.
    let Ok(parsed) = messages(&buffer[..complete_prefix(buffer)]) else {
        return Version::Tls13;
    };
    let Some(first) = parsed.first() else {
        return Version::Tls13;
    };
    if first.typ != HandshakeType::ServerHello {
        return Version::Tls13;
    }
    match ServerHello12::parse(first.body) {
        Ok(hello) if find(&hello.extensions, extension::SUPPORTED_VERSIONS).is_none() => {
            Version::Tls12
        }
        _ => Version::Tls13,
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// A server that speaks TLS 1.3 and TLS 1.2.
pub struct ServerConfigBoth<'a> {
    /// The TLS 1.3 half.
    pub tls13: &'a ServerConfig<'a>,
    /// The TLS 1.2 half.
    pub tls12: &'a ServerConfig12<'a>,
}

enum ServerState<'a> {
    Pending {
        buffer: Vec<u8>,
        records: Vec<Vec<u8>>,
    },
    Tls13(Box<ServerHandshake<'a>>),
    Tls12(Box<ServerHandshake12<'a>>),
    Failed,
}

/// A server handshake that has not yet read a ClientHello.
pub struct ServerHandshakeBoth<'a> {
    config: &'a ServerConfigBoth<'a>,
    state: ServerState<'a>,
}

impl<'a> ServerHandshakeBoth<'a> {
    /// A server waiting for a ClientHello.
    pub fn new(config: &'a ServerConfigBoth<'a>) -> Self {
        Self {
            config,
            state: ServerState::Pending {
                buffer: Vec::new(),
                records: Vec::new(),
            },
        }
    }

    /// True once the handshake has completed in either version.
    pub fn is_finished(&self) -> bool {
        match &self.state {
            ServerState::Tls13(h) => h.is_finished(),
            ServerState::Tls12(h) => h.is_finished(),
            _ => false,
        }
    }

    /// The version chosen, once the ClientHello has been read.
    pub fn version(&self) -> Option<Version> {
        match &self.state {
            ServerState::Tls13(_) => Some(Version::Tls13),
            ServerState::Tls12(_) => Some(Version::Tls12),
            _ => None,
        }
    }

    /// Take the established connection.
    pub fn into_connection(self) -> Result<Established, ServerError> {
        match self.state {
            ServerState::Tls13(h) => Ok(Established::Tls13((*h).into_connection()?)),
            ServerState::Tls12(h) => Ok(Established::Tls12((*h).into_connection()?)),
            _ => Err(ServerError::Failed),
        }
    }

    /// The fatal alert record to send for `error`, if the peer should be told.
    pub fn alert_record(&self, error: &ServerError) -> Option<Vec<u8>> {
        let description = error.alert()?;
        plaintext_record(ContentType::Alert, 0x0303, &[2, description.0]).ok()
    }

    /// Feed one whole TLS record, and get back the bytes to send in reply.
    /// A failure is permanent.
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>, ServerError> {
        let result = match &mut self.state {
            ServerState::Tls13(h) => return h.read_record(record),
            ServerState::Tls12(h) => return h.read_record(record),
            ServerState::Failed => return Err(ServerError::Failed),
            ServerState::Pending { .. } => self.read_pending(record),
        };
        if result.is_err() {
            self.state = ServerState::Failed;
        }
        result
    }

    fn read_pending(&mut self, record: &[u8]) -> Result<Vec<u8>, ServerError> {
        let ServerState::Pending { buffer, records } = &mut self.state else {
            return Err(ServerError::Failed);
        };
        let (typ, _, fragment) = split(record)?;
        records.push(record.to_vec());
        // A record that is not handshake data cannot be a ClientHello. The 1.2
        // server is the stricter of the two about what may open a connection.
        if typ != ContentType::Handshake {
            return self.decide(Version::Tls12);
        }
        if buffer.len() + fragment.len() > MAX_HANDSHAKE_BUFFER {
            return Err(ServerError::HandshakeTooLarge);
        }
        buffer.extend_from_slice(fragment);
        if complete_prefix(buffer) == 0 {
            return Ok(Vec::new());
        }
        let version = client_hello_version(buffer)?;
        self.decide(version)
    }

    fn decide(&mut self, version: Version) -> Result<Vec<u8>, ServerError> {
        let ServerState::Pending { records, .. } =
            core::mem::replace(&mut self.state, ServerState::Failed)
        else {
            return Err(ServerError::Failed);
        };
        let mut reply = Vec::new();
        match version {
            Version::Tls13 => {
                let mut machine = ServerHandshake::new(self.config.tls13);
                for record in &records {
                    reply.extend(machine.read_record(record)?);
                }
                self.state = ServerState::Tls13(Box::new(machine));
            }
            Version::Tls12 => {
                let mut machine = ServerHandshake12::downgraded(self.config.tls12)?;
                for record in &records {
                    reply.extend(machine.read_record(record)?);
                }
                self.state = ServerState::Tls12(Box::new(machine));
            }
        }
        Ok(reply)
    }
}

/// Which machine should read the ClientHello that is complete in `buffer`.
///
/// Errors only for what the choice itself must refuse: a hello that cannot be
/// read, one that offers neither version, and a TLS 1.2 fallback.
fn client_hello_version(buffer: &[u8]) -> Result<Version, ServerError> {
    let parsed = messages(&buffer[..complete_prefix(buffer)])?;
    let Some(first) = parsed
        .first()
        .filter(|m| m.typ == HandshakeType::ClientHello)
    else {
        // Not a ClientHello: the 1.2 machine names what it got instead.
        return Ok(Version::Tls12);
    };
    let hello = ClientHello12::parse(first.body)?;
    if hello
        .supported_versions()?
        .is_some_and(|versions| versions.contains(&TLS13))
    {
        return Ok(Version::Tls13);
    }
    // Everything else is the 1.2 server's to judge, including a client that
    // offers neither version: it names that precisely, and a second copy of the
    // rule here would only be a second place for it to drift.
    // A hello whose best is 1.2 and that signals a fallback: refuse.
    if hello.cipher_suites.contains(&FALLBACK_SCSV) {
        return Err(ServerError::InappropriateFallback);
    }
    Ok(Version::Tls12)
}
