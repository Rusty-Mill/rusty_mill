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
//! 4. [`RdpTransport::send_data`] / [`RdpTransport::recv_data`] — I/O-channel
//!    traffic once the session is up.
//!
//! Everything here stays on the standard library, so the crate remains
//! dependency-free. The later, security-dependent PDUs (Security Exchange,
//! Client Info, capabilities) are built with the [`crate::security`],
//! [`crate::client_info`], and [`crate::capabilities`] modules and sent with
//! [`RdpTransport::send_data`]; driving them end to end against a live server
//! is left to the caller.

use std::io::{self, Read, Write};

use crate::gcc::{self, UserDataBlock};
use crate::mcs::{ConnectInitial, ConnectResponse, DomainPdu, McsResult};
use crate::nego::{Negotiation, SecurityProtocols};
use crate::tpkt::{Tpkt, TPKT_HEADER_LEN};
use crate::x224::{ConnectionPdu, Cookie, X224};

/// Map a codec [`crate::Error`] into an [`io::Error`] for the transport layer.
fn to_io(e: crate::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

fn protocol_error(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// A TPKT-framed RDP transport over a byte stream.
pub struct RdpTransport<S> {
    stream: S,
}

impl<S: Read + Write> RdpTransport<S> {
    /// Wrap a connected stream.
    pub fn new(stream: S) -> Self {
        RdpTransport { stream }
    }

    /// Consume the transport and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    // --- TPKT framing -----------------------------------------------------

    /// Write `payload` as a single TPKT packet.
    pub fn write_tpkt(&mut self, payload: &[u8]) -> io::Result<()> {
        let packet = Tpkt::new(payload).to_vec().map_err(to_io)?;
        self.stream.write_all(&packet)?;
        self.stream.flush()
    }

    /// Read one complete TPKT packet, returning its payload (the X.224 TPDU).
    pub fn read_tpkt(&mut self) -> io::Result<Vec<u8>> {
        let mut header = [0u8; TPKT_HEADER_LEN];
        self.stream.read_exact(&mut header)?;
        let total = Tpkt::peek_total_len(&header)
            .map_err(to_io)?
            .ok_or_else(|| protocol_error("short TPKT header"))?;
        let mut packet = vec![0u8; total];
        packet[..TPKT_HEADER_LEN].copy_from_slice(&header);
        self.stream.read_exact(&mut packet[TPKT_HEADER_LEN..])?;
        let tpkt = Tpkt::decode(&packet).map_err(to_io)?;
        Ok(tpkt.payload.to_vec())
    }

    /// Wrap `payload` in an X.224 Data TPDU and send it.
    pub fn write_x224_data(&mut self, payload: &[u8]) -> io::Result<()> {
        let tpdu = X224::data(payload).to_vec().map_err(to_io)?;
        self.write_tpkt(&tpdu)
    }

    /// Read a TPKT packet and return the inner X.224 Data TPDU payload.
    pub fn read_x224_data(&mut self) -> io::Result<Vec<u8>> {
        let tpdu = self.read_tpkt()?;
        match X224::decode(&tpdu).map_err(to_io)? {
            X224::Data(payload) => Ok(payload.to_vec()),
            other => Err(protocol_error(format!("expected Data TPDU, got {other:?}"))),
        }
    }

    // --- Connection sequence ---------------------------------------------

    /// Perform the X.224 security negotiation and return the server's
    /// selected protocol.
    ///
    /// Sends a Connection Request advertising `requested` (with an optional
    /// `mstshash` cookie) and interprets the Connection Confirm. A server that
    /// omits the negotiation response is treated as selecting standard RDP
    /// security; a negotiation failure becomes an error.
    pub fn negotiate(
        &mut self,
        requested: SecurityProtocols,
        cookie: Option<&str>,
    ) -> io::Result<SecurityProtocols> {
        let request = ConnectionPdu {
            cookie: cookie.map(|c| Cookie::MsTsHash(c.to_string())),
            negotiation: Some(Negotiation::Request {
                flags: 0,
                protocols: requested,
            }),
            ..Default::default()
        };
        let cr = X224::ConnectionRequest(request).to_vec().map_err(to_io)?;
        self.write_tpkt(&cr)?;

        let response = self.read_tpkt()?;
        match X224::decode(&response).map_err(to_io)? {
            X224::ConnectionConfirm(pdu) => match pdu.negotiation {
                Some(Negotiation::Response { selected, .. }) => Ok(selected),
                Some(Negotiation::Failure { code }) => {
                    Err(protocol_error(format!("negotiation failed: {code:?}")))
                }
                _ => Ok(SecurityProtocols::RDP),
            },
            other => Err(protocol_error(format!(
                "expected Connection Confirm, got {other:?}"
            ))),
        }
    }

    /// Perform the GCC/MCS `Connect-Initial` / `Connect-Response` exchange.
    ///
    /// Wraps `client_blocks` in a Conference Create Request and returns the
    /// server's settings blocks parsed from the Conference Create Response.
    pub fn mcs_connect(
        &mut self,
        client_blocks: &[UserDataBlock],
    ) -> io::Result<Vec<UserDataBlock>> {
        let user_data = gcc::encode_user_data(client_blocks).map_err(to_io)?;
        let ccr = gcc::encode_conference_create_request(&user_data).map_err(to_io)?;
        let connect_initial = ConnectInitial::new(ccr).to_vec();
        self.write_x224_data(&connect_initial)?;

        let response = self.read_x224_data()?;
        let connect_response = ConnectResponse::decode(&response).map_err(to_io)?;
        let (_node_id, server_ud) =
            gcc::decode_conference_create_response(&connect_response.user_data).map_err(to_io)?;
        gcc::parse_user_data(&server_ud).map_err(to_io)
    }

    /// Send the MCS Erect Domain Request (no response expected).
    pub fn erect_domain(&mut self) -> io::Result<()> {
        let pdu = DomainPdu::ErectDomainRequest {
            sub_height: 0,
            sub_interval: 0,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&pdu)
    }

    /// Send an Attach User Request and return the assigned `UserId`.
    pub fn attach_user(&mut self) -> io::Result<u16> {
        let req = DomainPdu::AttachUserRequest.to_vec().map_err(to_io)?;
        self.write_x224_data(&req)?;

        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(user_id),
            } => Ok(user_id),
            DomainPdu::AttachUserConfirm { result, .. } => {
                Err(protocol_error(format!("attach user rejected: {result:?}")))
            }
            other => Err(protocol_error(format!(
                "expected Attach User Confirm, got {other:?}"
            ))),
        }
    }

    /// Join `channel_id` as `user_id`, waiting for the confirm.
    pub fn join_channel(&mut self, user_id: u16, channel_id: u16) -> io::Result<()> {
        let req = DomainPdu::ChannelJoinRequest {
            initiator: user_id,
            channel_id,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&req)?;

        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::ChannelJoinConfirm {
                result: McsResult::Successful,
                ..
            } => Ok(()),
            DomainPdu::ChannelJoinConfirm { result, .. } => Err(protocol_error(format!(
                "channel {channel_id} join rejected: {result:?}"
            ))),
            other => Err(protocol_error(format!(
                "expected Channel Join Confirm, got {other:?}"
            ))),
        }
    }

    // --- I/O channel traffic ---------------------------------------------

    /// Send `data` on `channel_id` as a Send Data Request from `user_id`.
    pub fn send_data(&mut self, user_id: u16, channel_id: u16, data: &[u8]) -> io::Result<()> {
        let pdu = DomainPdu::SendDataRequest {
            initiator: user_id,
            channel_id,
            user_data: data,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&pdu)
    }

    /// Receive one Send Data Indication, returning `(channel_id, data)`.
    pub fn recv_data(&mut self) -> io::Result<(u16, Vec<u8>)> {
        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::SendDataIndication {
                channel_id,
                user_data,
                ..
            } => Ok((channel_id, user_data.to_vec())),
            other => Err(protocol_error(format!(
                "expected Send Data Indication, got {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// An in-memory duplex stream: reads drain `inbound`, writes append to
    /// `outbound`.
    struct MockStream {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl MockStream {
        fn new(inbound: Vec<u8>) -> Self {
            MockStream {
                inbound: inbound.into(),
                outbound: Vec::new(),
            }
        }
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut n = 0;
            while n < buf.len() {
                match self.inbound.pop_front() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            if n == 0 && !buf.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            Ok(n)
        }
    }

    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.outbound.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Wrap an X.224 TPDU in TPKT the way a server would frame it.
    fn framed(tpdu: Vec<u8>) -> Vec<u8> {
        Tpkt::new(&tpdu).to_vec().unwrap()
    }

    #[test]
    fn tpkt_framing_roundtrip() {
        // A server that will echo one payload back to us.
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];
        let inbound = Tpkt::new(&payload).to_vec().unwrap();
        let mut t = RdpTransport::new(MockStream::new(inbound));
        assert_eq!(t.read_tpkt().unwrap(), payload);

        t.write_tpkt(&[0x01, 0x02]).unwrap();
        let out = t.into_inner().outbound;
        assert_eq!(Tpkt::decode(&out).unwrap().payload, &[0x01, 0x02]);
    }

    #[test]
    fn negotiate_returns_selected_protocol() {
        // Server replies with a Connection Confirm selecting TLS.
        let confirm = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Response {
                flags: 0,
                selected: SecurityProtocols::SSL,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));

        let selected = t
            .negotiate(
                SecurityProtocols::RDP | SecurityProtocols::SSL,
                Some("user"),
            )
            .unwrap();
        assert_eq!(selected, SecurityProtocols::SSL);

        // The client sent a Connection Request carrying the cookie.
        let out = t.into_inner().outbound;
        assert!(out.windows(9).any(|w| w == b"mstshash="));
    }

    #[test]
    fn negotiate_reports_failure() {
        use crate::nego::NegFailureCode;
        let failure = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Failure {
                code: NegFailureCode::HybridRequiredByServer,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(failure)));
        assert!(t.negotiate(SecurityProtocols::RDP, None).is_err());
    }

    #[test]
    fn attach_user_returns_user_id() {
        let confirm = X224::data(
            &DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(1007),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));
        assert_eq!(t.attach_user().unwrap(), 1007);
    }

    #[test]
    fn join_channel_accepts_confirm() {
        let confirm = X224::data(
            &DomainPdu::ChannelJoinConfirm {
                result: McsResult::Successful,
                initiator: 1007,
                requested: 1003,
                channel_id: Some(1003),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));
        assert!(t.join_channel(1007, 1003).is_ok());
    }

    #[test]
    fn recv_data_extracts_channel_payload() {
        let indication = X224::data(
            &DomainPdu::SendDataIndication {
                initiator: 1002,
                channel_id: 1003,
                user_data: &[0xAA, 0xBB, 0xCC],
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(indication)));
        let (channel, data) = t.recv_data().unwrap();
        assert_eq!(channel, 1003);
        assert_eq!(data, [0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn mcs_connect_parses_server_blocks() {
        use crate::gcc::{ServerCoreData, ServerNetworkData};

        // Server builds a Connect-Response wrapping SC_CORE + SC_NET.
        let server_blocks = vec![
            UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: Some(0),
                early_capability_flags: None,
            }),
            UserDataBlock::ServerNetwork(ServerNetworkData {
                io_channel_id: 1003,
                channel_ids: vec![],
            }),
        ];
        let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
        let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
        let response = ConnectResponse {
            result: McsResult::Successful,
            called_connect_id: 0,
            domain_parameters: crate::mcs::DomainParameters::client_target(),
            user_data: ccrsp,
        };
        let inbound = framed(X224::data(&response.to_vec()).to_vec().unwrap());

        let mut t = RdpTransport::new(MockStream::new(inbound));
        let blocks = t
            .mcs_connect(&[UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: None,
                early_capability_flags: None,
            })])
            .unwrap();
        assert_eq!(blocks, server_blocks);
    }
}
