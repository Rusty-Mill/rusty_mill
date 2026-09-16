//! Blocking TCP driver — the I/O boundary around the pure codec.
//!
//! Every other module in this crate is I/O-free: it turns bytes into types
//! and back. This module is the one place that touches a socket. It wraps any
//! [`Read`] + [`Write`] (a [`std::net::TcpStream`] in practice) and drives the
//! deterministic part of the RDP connection sequence:
//!
//! 1. [`RdpTransport::negotiate`] — the X.224 security negotiation.
//! 2. [`RdpTransport::mcs_connect`] — the GCC/MCS `Connect-Initial` /
//!    `Connect-Response` exchange.
//! 3. [`RdpTransport::erect_domain`], [`RdpTransport::attach_user`],
//!    [`RdpTransport::join_channel`] — MCS channel setup.
//! 4. [`RdpTransport::security_exchange`] — RSA-encrypt the client random for
//!    standard RDP security, then [`RdpTransport::send_client_info`] and
//!    [`RdpTransport::send_secure`] / [`RdpTransport::recv_secure`] carry the
//!    encrypted, MAC'd PDUs.
//! 5. [`RdpTransport::send_data`] / [`RdpTransport::recv_data`] — raw
//!    I/O-channel traffic.
//!
//! [`RdpTransport::establish`] chains the whole standard-RDP sequence —
//! negotiation, MCS connect, channel setup, security exchange, encrypted
//! Client Info, licensing, capability exchange, and connection finalization —
//! into one call and returns an active [`RdpSession`]. [`server_crypto`] pulls
//! the server's RSA key and random out of the Connect-Response for it.
//!
//! For the enhanced-security (TLS/CredSSP) path, the X.224 negotiation runs on
//! the raw TCP connection, the stream is then upgraded to TLS, and
//! [`RdpTransport::new_enhanced`] + [`RdpTransport::establish_enhanced`] drive
//! the rest of the sequence with the RDP security layer switched off — no
//! Security Exchange and no RC4, since TLS provides confidentiality. This
//! module stays dependency-free by being generic over the stream: bring any
//! TLS implementation (or, with the optional `tls` feature, use
//! `crate::tls::connect_tls`).
//!
//! Once active, [`RdpTransport::recv_event`] reads server updates — accepting
//! both slow-path (TPKT) and fast-path framing transparently — and
//! [`RdpTransport::send_input`] sends keyboard/mouse events over the compact
//! fast-path input path. Everything here stays on the standard library, so the
//! crate remains dependency-free.
//!
//! Static virtual channels beyond the required I/O channel — e.g. `"DRDYNVC"`,
//! which carries [`crate::dvc`]'s dynamic-channel traffic (RDPGFX,
//! redirection protocols) — are opt in: list them in
//! [`EstablishConfig::extra_channels`], look up the id the server granted with
//! [`RdpSession::channel_id`], and [`RdpTransport::recv_event`] reassembles
//! their chunked traffic (MS-RDPBCGR 2.2.6.1, [`crate::vchan`]) into
//! [`RdpEvent::ChannelData`] alongside the usual display/input events;
//! [`RdpTransport::send_channel_data`] is the outbound counterpart.
//!
//! ## Server side
//!
//! [`RdpTransport::accept`] drives the same connection sequence in reverse,
//! as a server: X.224 Connection Confirm, the GCC/MCS `Connect-Response`
//! (building the server's settings blocks), channel-join confirmation, the
//! Client Info PDU, the "no license required" response, Demand Active /
//! Confirm Active, and the server's connection-finalization sequence —
//! returning an [`AcceptedClient`]. Every codec type it uses is the same
//! bidirectional type [`RdpTransport::establish`] uses on the other side;
//! `accept` is what supplies the missing server-role driving logic and
//! defaults (a fixed share id and MCS identity, since there is only ever one
//! client per `accept` call).
//!
//! By default `accept` speaks **unencrypted** standard RDP security
//! (`encryptionLevel = 0`): no RSA key exchange, no RC4. Supplying
//! [`AcceptConfig::encryption`] drives real encrypted standard security
//! instead (RSA key exchange, a signed proprietary certificate, RC4). TLS/
//! CredSSP server support needs a certificate and a TLS server
//! implementation, which doesn't exist in this crate yet, so treat `accept`
//! as a building block for trusted-network or testing use, not a
//! production-ready server.

mod client;
mod framing;
mod server;
mod session;
#[cfg(test)]
mod tests;

pub use client::{server_crypto, EstablishConfig, RdpSession, ServerCrypto};
pub use framing::RdpTransport;
pub use server::{AcceptConfig, AcceptEncryption, AcceptedClient};
pub use session::RdpEvent;

use std::collections::HashMap;
use std::io::{self, Read, Write};

use crate::capabilities::{
    client_capability_sets, server_capability_sets, ConfirmActive, DemandActive,
};
use crate::client_info::{ClientInfo, INFO_UNICODE};
use crate::finalization::FinalizationPdu;
use crate::finalization::{client_finalization_sequence, server_finalization_sequence};
use crate::gcc::{
    self, ChannelDef, ClientClusterData, ClientCoreData, ClientNetworkData, ClientSecurityData,
    ServerCoreData, ServerNetworkData, ServerSecurityData, UserDataBlock,
    ENCRYPTION_LEVEL_CLIENT_COMPATIBLE, ENCRYPTION_METHOD_128BIT, ENCRYPTION_METHOD_40BIT,
    ENCRYPTION_METHOD_56BIT, RDP_VERSION_5_PLUS,
};
use crate::input::InputEvent;
use crate::license::{LicenseErrorMessage, LicensePdu};
use crate::mcs::{
    ConnectInitial, ConnectResponse, DomainParameters, DomainPdu, McsResult, MCS_BASE_CHANNEL_ID,
    MCS_GLOBAL_CHANNEL_ID,
};
#[cfg(feature = "tls")]
use crate::nego::NegFailureCode;
use crate::nego::{Negotiation, SecurityProtocols};
use crate::output::{BitmapData, PaletteUpdate, UpdatePdu};
use crate::pdu::{
    ShareControlHeader, ShareDataHeader, PDUTYPE2_CONTROL, PDUTYPE2_FONTMAP, PDUTYPE2_POINTER,
    PDUTYPE2_SYNCHRONIZE, PDUTYPE2_UPDATE, PDUTYPE_DEACTIVATEALLPDU, PDUTYPE_DEMANDACTIVEPDU,
};
use crate::pointer::PointerUpdate;
use crate::security::{
    self, derive_session_keys, Rc4Session, RsaPrivateKey, RsaPublicKey, RANDOM_LEN, SEC_INFO_PKT,
    SEC_LICENSE_PKT,
};
use crate::tpkt::{Tpkt, TPKT_HEADER_LEN};
use crate::x224::{ConnectionPdu, Cookie, X224};

/// Map a codec [`crate::Error`] into an [`io::Error`] for the transport layer.
fn to_io(e: crate::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

fn protocol_error(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
