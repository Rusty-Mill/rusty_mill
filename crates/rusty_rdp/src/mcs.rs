//! MCS — Multipoint Communication Service (ITU-T T.125) as RDP uses it.
//!
//! MCS multiplexes the RDP session into channels over the single X.224
//! transport. It has two distinct encodings:
//!
//! * The **connection PDUs** `Connect-Initial` (client) and
//!   `Connect-Response` (server) are BER-encoded ([`crate::ber`]). Each wraps
//!   an opaque block of GCC (T.124) user data — the client/server core,
//!   security, and network settings — which this crate carries verbatim for
//!   now and will decode in a dedicated `gcc` layer.
//! * The **domain PDUs** — erect domain, attach user, channel join, send
//!   data — are PER-encoded ([`crate::per`]) and far more compact. Each is a
//!   single header byte (`domainMCSPDU << 2`, optionally OR'd with a
//!   SEQUENCE presence bit) followed by its fields.
//!
//! ```text
//! Connect-Initial  (BER, [APPLICATION 101])  ── client ──▶
//!                              ◀── Connect-Response (BER, [APPLICATION 102])
//! ErectDomainRequest / AttachUserRequest      ── client ──▶
//!                              ◀── AttachUserConfirm (assigns UserId)
//! ChannelJoinRequest (per channel)            ── client ──▶
//!                              ◀── ChannelJoinConfirm
//! SendDataRequest  ⇄  SendDataIndication      (all later RDP PDUs)
//! ```

use crate::ber;
use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::per;

/// Base ID of the MCS user channels; a client's `UserId` is assigned at or
/// above this value (`MCS_USERCHANNEL_BASE`).
pub const MCS_BASE_CHANNEL_ID: u16 = 1001;

/// The well-known MCS I/O channel that carries the main RDP data
/// (`MCS_GLOBAL_CHANNEL`).
pub const MCS_GLOBAL_CHANNEL_ID: u16 = 1003;

// DomainMCSPDU CHOICE indices (T.125). Only the ones RDP uses.
const PDU_ERECT_DOMAIN_REQUEST: u8 = 1;
const PDU_DISCONNECT_PROVIDER_ULTIMATUM: u8 = 8;
const PDU_ATTACH_USER_REQUEST: u8 = 10;
const PDU_ATTACH_USER_CONFIRM: u8 = 11;
const PDU_CHANNEL_JOIN_REQUEST: u8 = 14;
const PDU_CHANNEL_JOIN_CONFIRM: u8 = 15;
const PDU_SEND_DATA_REQUEST: u8 = 25;
const PDU_SEND_DATA_INDICATION: u8 = 26;

/// `dataPriority` (high) + fully-segmented flags byte in a Send Data PDU.
const SEND_DATA_SEGMENTATION: u8 = 0x70;

/// MCS `Result` enumeration (T.125). `rt-successful` is zero; other codes
/// signal a rejected request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McsResult {
    /// `rt-successful` — the request was accepted.
    Successful,
    /// Any non-zero result code, kept verbatim.
    Other(u8),
}

impl McsResult {
    fn from_u8(v: u8) -> McsResult {
        if v == 0 {
            McsResult::Successful
        } else {
            McsResult::Other(v)
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            McsResult::Successful => 0,
            McsResult::Other(v) => v,
        }
    }
}

/// MCS `DomainParameters` (T.125): the negotiated limits of a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainParameters {
    /// Maximum number of channels.
    pub max_channel_ids: u32,
    /// Maximum number of user IDs.
    pub max_user_ids: u32,
    /// Maximum number of token IDs.
    pub max_token_ids: u32,
    /// Number of data priorities.
    pub num_priorities: u32,
    /// Minimum throughput interval.
    pub min_throughput: u32,
    /// Maximum domain height.
    pub max_height: u32,
    /// Maximum MCS PDU size in bytes.
    pub max_mcs_pdu_size: u32,
    /// MCS protocol version.
    pub protocol_version: u32,
}

impl DomainParameters {
    /// The `targetParameters` an RDP client conventionally requests.
    pub fn client_target() -> Self {
        DomainParameters {
            max_channel_ids: 34,
            max_user_ids: 2,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        }
    }

    /// The `minimumParameters` an RDP client conventionally accepts.
    pub fn client_minimum() -> Self {
        DomainParameters {
            max_channel_ids: 1,
            max_user_ids: 1,
            max_token_ids: 1,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 1056,
            protocol_version: 2,
        }
    }

    /// The `maximumParameters` an RDP client conventionally accepts.
    pub fn client_maximum() -> Self {
        DomainParameters {
            max_channel_ids: 65535,
            max_user_ids: 64535,
            max_token_ids: 65535,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        }
    }

    fn encode(&self, w: &mut Writer) {
        let mut inner = Writer::new();
        for v in [
            self.max_channel_ids,
            self.max_user_ids,
            self.max_token_ids,
            self.num_priorities,
            self.min_throughput,
            self.max_height,
            self.max_mcs_pdu_size,
            self.protocol_version,
        ] {
            ber::write_integer(&mut inner, v);
        }
        ber::write_tlv(w, ber::TAG_SEQUENCE, inner.as_slice());
    }

    fn decode(r: &mut Reader<'_>) -> Result<DomainParameters> {
        let len = ber::expect_tag(r, ber::TAG_SEQUENCE)?;
        let body = r.read_bytes(len)?;
        let mut b = Reader::new(body);
        Ok(DomainParameters {
            max_channel_ids: ber::read_integer(&mut b)?,
            max_user_ids: ber::read_integer(&mut b)?,
            max_token_ids: ber::read_integer(&mut b)?,
            num_priorities: ber::read_integer(&mut b)?,
            min_throughput: ber::read_integer(&mut b)?,
            max_height: ber::read_integer(&mut b)?,
            max_mcs_pdu_size: ber::read_integer(&mut b)?,
            protocol_version: ber::read_integer(&mut b)?,
        })
    }
}

/// MCS `Connect-Initial` — the client's first MCS PDU, wrapping the GCC
/// Conference Create Request in [`user_data`](ConnectInitial::user_data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectInitial {
    /// `callingDomainSelector` (RDP uses a single `0x01` byte).
    pub calling_domain: Vec<u8>,
    /// `calledDomainSelector` (RDP uses a single `0x01` byte).
    pub called_domain: Vec<u8>,
    /// `upwardFlag` — always true for an RDP client.
    pub upward_flag: bool,
    /// Requested domain parameters.
    pub target_parameters: DomainParameters,
    /// Minimum acceptable domain parameters.
    pub minimum_parameters: DomainParameters,
    /// Maximum acceptable domain parameters.
    pub maximum_parameters: DomainParameters,
    /// Opaque GCC Conference Create Request user data.
    pub user_data: Vec<u8>,
}

impl ConnectInitial {
    /// Build a `Connect-Initial` with the conventional RDP client defaults,
    /// wrapping the supplied GCC user data.
    pub fn new(user_data: Vec<u8>) -> Self {
        ConnectInitial {
            calling_domain: vec![0x01],
            called_domain: vec![0x01],
            upward_flag: true,
            target_parameters: DomainParameters::client_target(),
            minimum_parameters: DomainParameters::client_minimum(),
            maximum_parameters: DomainParameters::client_maximum(),
            user_data,
        }
    }

    /// Encode to BER bytes (an X.224 Data TPDU payload).
    pub fn to_vec(&self) -> Vec<u8> {
        let mut inner = Writer::new();
        ber::write_octet_string(&mut inner, &self.calling_domain);
        ber::write_octet_string(&mut inner, &self.called_domain);
        ber::write_boolean(&mut inner, self.upward_flag);
        self.target_parameters.encode(&mut inner);
        self.minimum_parameters.encode(&mut inner);
        self.maximum_parameters.encode(&mut inner);
        ber::write_octet_string(&mut inner, &self.user_data);

        let mut w = Writer::new();
        ber::write_tlv(&mut w, ber::TAG_CONNECT_INITIAL, inner.as_slice());
        w.into_vec()
    }

    /// Decode from BER bytes.
    pub fn decode(buf: &[u8]) -> Result<ConnectInitial> {
        let mut r = Reader::new(buf);
        let len = ber::expect_tag(&mut r, ber::TAG_CONNECT_INITIAL)?;
        let body = r.read_bytes(len)?;
        let mut b = Reader::new(body);
        let calling_domain = ber::read_octet_string(&mut b)?.to_vec();
        let called_domain = ber::read_octet_string(&mut b)?.to_vec();
        let upward_flag = ber::read_boolean(&mut b)?;
        let target_parameters = DomainParameters::decode(&mut b)?;
        let minimum_parameters = DomainParameters::decode(&mut b)?;
        let maximum_parameters = DomainParameters::decode(&mut b)?;
        let user_data = ber::read_octet_string(&mut b)?.to_vec();
        Ok(ConnectInitial {
            calling_domain,
            called_domain,
            upward_flag,
            target_parameters,
            minimum_parameters,
            maximum_parameters,
            user_data,
        })
    }
}

/// MCS `Connect-Response` — the server's answer, wrapping the GCC Conference
/// Create Response in [`user_data`](ConnectResponse::user_data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectResponse {
    /// Overall result of the connect.
    pub result: McsResult,
    /// `calledConnectId` assigned by the server.
    pub called_connect_id: u32,
    /// Domain parameters the server selected.
    pub domain_parameters: DomainParameters,
    /// Opaque GCC Conference Create Response user data.
    pub user_data: Vec<u8>,
}

impl ConnectResponse {
    /// Encode to BER bytes.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut inner = Writer::new();
        ber::write_enumerated(&mut inner, self.result.to_u8());
        ber::write_integer(&mut inner, self.called_connect_id);
        self.domain_parameters.encode(&mut inner);
        ber::write_octet_string(&mut inner, &self.user_data);

        let mut w = Writer::new();
        ber::write_tlv(&mut w, ber::TAG_CONNECT_RESPONSE, inner.as_slice());
        w.into_vec()
    }

    /// Decode from BER bytes.
    pub fn decode(buf: &[u8]) -> Result<ConnectResponse> {
        let mut r = Reader::new(buf);
        let len = ber::expect_tag(&mut r, ber::TAG_CONNECT_RESPONSE)?;
        let body = r.read_bytes(len)?;
        let mut b = Reader::new(body);
        let result = McsResult::from_u8(ber::read_enumerated(&mut b)?);
        let called_connect_id = ber::read_integer(&mut b)?;
        let domain_parameters = DomainParameters::decode(&mut b)?;
        let user_data = ber::read_octet_string(&mut b)?.to_vec();
        Ok(ConnectResponse {
            result,
            called_connect_id,
            domain_parameters,
            user_data,
        })
    }
}

/// A PER-encoded MCS domain PDU (everything after the connection handshake).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainPdu<'a> {
    /// `ErectDomainRequest` — client establishes the domain hierarchy.
    ErectDomainRequest {
        /// `subHeight`.
        sub_height: u32,
        /// `subInterval`.
        sub_interval: u32,
    },
    /// `AttachUserRequest` — client asks for a user channel.
    AttachUserRequest,
    /// `AttachUserConfirm` — server assigns the client's `UserId`.
    AttachUserConfirm {
        /// Result of the attach.
        result: McsResult,
        /// The assigned `UserId`, if present.
        initiator: Option<u16>,
    },
    /// `ChannelJoinRequest` — client joins one channel.
    ChannelJoinRequest {
        /// The requesting user's `UserId`.
        initiator: u16,
        /// The channel to join.
        channel_id: u16,
    },
    /// `ChannelJoinConfirm` — server confirms a channel join.
    ChannelJoinConfirm {
        /// Result of the join.
        result: McsResult,
        /// The requesting user's `UserId`.
        initiator: u16,
        /// The channel that was requested.
        requested: u16,
        /// The channel actually joined, if present.
        channel_id: Option<u16>,
    },
    /// `SendDataRequest` — client → server channel data.
    SendDataRequest {
        /// Sending user's `UserId`.
        initiator: u16,
        /// Target channel.
        channel_id: u16,
        /// Opaque channel payload (a higher-layer RDP PDU).
        user_data: &'a [u8],
    },
    /// `SendDataIndication` — server → client channel data.
    SendDataIndication {
        /// Originating user's `UserId`.
        initiator: u16,
        /// Source channel.
        channel_id: u16,
        /// Opaque channel payload (a higher-layer RDP PDU).
        user_data: &'a [u8],
    },
    /// `DisconnectProviderUltimatum` — the connection is being torn down.
    DisconnectProviderUltimatum {
        /// `reason` enumeration value.
        reason: u8,
    },
}

impl<'a> DomainPdu<'a> {
    /// Encode this PDU to PER bytes (an X.224 Data TPDU payload).
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        self.encode(&mut w)?;
        Ok(w.into_vec())
    }

    /// Encode this PDU into `w`.
    pub fn encode(&self, w: &mut Writer) -> Result<()> {
        match *self {
            DomainPdu::ErectDomainRequest {
                sub_height,
                sub_interval,
            } => {
                per::write_choice(w, PDU_ERECT_DOMAIN_REQUEST << 2);
                per::write_integer(w, sub_height)?;
                per::write_integer(w, sub_interval)?;
            }
            DomainPdu::AttachUserRequest => {
                per::write_choice(w, PDU_ATTACH_USER_REQUEST << 2);
            }
            DomainPdu::AttachUserConfirm { result, initiator } => {
                // Low bit flags presence of the optional `initiator`.
                let presence = if initiator.is_some() { 0x02 } else { 0x00 };
                per::write_choice(w, (PDU_ATTACH_USER_CONFIRM << 2) | presence);
                per::write_enumerated(w, result.to_u8());
                if let Some(id) = initiator {
                    per::write_integer16(w, id, MCS_BASE_CHANNEL_ID)?;
                }
            }
            DomainPdu::ChannelJoinRequest {
                initiator,
                channel_id,
            } => {
                per::write_choice(w, PDU_CHANNEL_JOIN_REQUEST << 2);
                per::write_integer16(w, initiator, MCS_BASE_CHANNEL_ID)?;
                per::write_integer16(w, channel_id, 0)?;
            }
            DomainPdu::ChannelJoinConfirm {
                result,
                initiator,
                requested,
                channel_id,
            } => {
                let presence = if channel_id.is_some() { 0x02 } else { 0x00 };
                per::write_choice(w, (PDU_CHANNEL_JOIN_CONFIRM << 2) | presence);
                per::write_enumerated(w, result.to_u8());
                per::write_integer16(w, initiator, MCS_BASE_CHANNEL_ID)?;
                per::write_integer16(w, requested, 0)?;
                if let Some(id) = channel_id {
                    per::write_integer16(w, id, 0)?;
                }
            }
            DomainPdu::SendDataRequest {
                initiator,
                channel_id,
                user_data,
            } => {
                Self::encode_send_data(w, PDU_SEND_DATA_REQUEST, initiator, channel_id, user_data)?
            }
            DomainPdu::SendDataIndication {
                initiator,
                channel_id,
                user_data,
            } => Self::encode_send_data(
                w,
                PDU_SEND_DATA_INDICATION,
                initiator,
                channel_id,
                user_data,
            )?,
            DomainPdu::DisconnectProviderUltimatum { reason } => {
                // The reason ENUMERATED is extensible; RDP encodes it with the
                // low presence bit set and the value in the high bits.
                per::write_choice(w, (PDU_DISCONNECT_PROVIDER_ULTIMATUM << 2) | 0x01);
                per::write_enumerated(w, reason << 1);
            }
        }
        Ok(())
    }

    fn encode_send_data(
        w: &mut Writer,
        pdu: u8,
        initiator: u16,
        channel_id: u16,
        user_data: &[u8],
    ) -> Result<()> {
        per::write_choice(w, pdu << 2);
        per::write_integer16(w, initiator, MCS_BASE_CHANNEL_ID)?;
        per::write_integer16(w, channel_id, 0)?;
        w.write_u8(SEND_DATA_SEGMENTATION);
        per::write_length(w, user_data.len())?;
        w.write_bytes(user_data);
        Ok(())
    }

    /// Decode a single domain PDU from `buf`.
    pub fn decode(buf: &'a [u8]) -> Result<DomainPdu<'a>> {
        let mut r = Reader::new(buf);
        let choice = per::read_choice(&mut r)?;
        let pdu = choice >> 2;
        let presence = choice & 0x03;
        match pdu {
            PDU_ERECT_DOMAIN_REQUEST => Ok(DomainPdu::ErectDomainRequest {
                sub_height: per::read_integer(&mut r)?,
                sub_interval: per::read_integer(&mut r)?,
            }),
            PDU_ATTACH_USER_REQUEST => Ok(DomainPdu::AttachUserRequest),
            PDU_ATTACH_USER_CONFIRM => {
                let result = McsResult::from_u8(per::read_enumerated(&mut r)?);
                let initiator = if presence & 0x02 != 0 {
                    Some(per::read_integer16(&mut r, MCS_BASE_CHANNEL_ID)?)
                } else {
                    None
                };
                Ok(DomainPdu::AttachUserConfirm { result, initiator })
            }
            PDU_CHANNEL_JOIN_REQUEST => Ok(DomainPdu::ChannelJoinRequest {
                initiator: per::read_integer16(&mut r, MCS_BASE_CHANNEL_ID)?,
                channel_id: per::read_integer16(&mut r, 0)?,
            }),
            PDU_CHANNEL_JOIN_CONFIRM => {
                let result = McsResult::from_u8(per::read_enumerated(&mut r)?);
                let initiator = per::read_integer16(&mut r, MCS_BASE_CHANNEL_ID)?;
                let requested = per::read_integer16(&mut r, 0)?;
                let channel_id = if presence & 0x02 != 0 {
                    Some(per::read_integer16(&mut r, 0)?)
                } else {
                    None
                };
                Ok(DomainPdu::ChannelJoinConfirm {
                    result,
                    initiator,
                    requested,
                    channel_id,
                })
            }
            PDU_SEND_DATA_REQUEST => {
                let (initiator, channel_id, user_data) = Self::decode_send_data(&mut r)?;
                Ok(DomainPdu::SendDataRequest {
                    initiator,
                    channel_id,
                    user_data,
                })
            }
            PDU_SEND_DATA_INDICATION => {
                let (initiator, channel_id, user_data) = Self::decode_send_data(&mut r)?;
                Ok(DomainPdu::SendDataIndication {
                    initiator,
                    channel_id,
                    user_data,
                })
            }
            PDU_DISCONNECT_PROVIDER_ULTIMATUM => Ok(DomainPdu::DisconnectProviderUltimatum {
                reason: per::read_enumerated(&mut r)? >> 1,
            }),
            other => Err(Error::InvalidValue {
                field: "MCS domain PDU",
                value: other.to_string(),
            }),
        }
    }

    fn decode_send_data(r: &mut Reader<'a>) -> Result<(u16, u16, &'a [u8])> {
        let initiator = per::read_integer16(r, MCS_BASE_CHANNEL_ID)?;
        let channel_id = per::read_integer16(r, 0)?;
        let _segmentation = r.read_u8()?;
        let len = per::read_length(r)?;
        let user_data = r.read_bytes(len)?;
        Ok((initiator, channel_id, user_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erect_domain_request_matches_wire() {
        let pdu = DomainPdu::ErectDomainRequest {
            sub_height: 0,
            sub_interval: 0,
        };
        // FreeRDP-compatible bytes: 04 (1<<2), then two `01 00` integers.
        assert_eq!(pdu.to_vec().unwrap(), [0x04, 0x01, 0x00, 0x01, 0x00]);
        assert_eq!(DomainPdu::decode(&pdu.to_vec().unwrap()).unwrap(), pdu);
    }

    #[test]
    fn attach_user_request_is_single_byte() {
        let pdu = DomainPdu::AttachUserRequest;
        assert_eq!(pdu.to_vec().unwrap(), [0x28]); // 10 << 2
        assert_eq!(DomainPdu::decode(&[0x28]).unwrap(), pdu);
    }

    #[test]
    fn attach_user_confirm_with_initiator() {
        let pdu = DomainPdu::AttachUserConfirm {
            result: McsResult::Successful,
            initiator: Some(1007),
        };
        // 0x2E = (11<<2)|2, result 0x00, initiator 1007-1001 = 0x0006.
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes, [0x2E, 0x00, 0x00, 0x06]);
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn attach_user_confirm_without_initiator() {
        let pdu = DomainPdu::AttachUserConfirm {
            result: McsResult::Other(1),
            initiator: None,
        };
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes, [0x2C, 0x01]); // 11<<2, no presence bit
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn channel_join_request_matches_wire() {
        let pdu = DomainPdu::ChannelJoinRequest {
            initiator: 1007,
            channel_id: MCS_GLOBAL_CHANNEL_ID,
        };
        // 0x38 = 14<<2, initiator 0x0006, channel 1003 = 0x03EB.
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes, [0x38, 0x00, 0x06, 0x03, 0xEB]);
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn channel_join_confirm_roundtrip() {
        let pdu = DomainPdu::ChannelJoinConfirm {
            result: McsResult::Successful,
            initiator: 1007,
            requested: MCS_GLOBAL_CHANNEL_ID,
            channel_id: Some(MCS_GLOBAL_CHANNEL_ID),
        };
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes[0], 0x3E); // (15<<2)|2
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn send_data_request_roundtrip() {
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];
        let pdu = DomainPdu::SendDataRequest {
            initiator: 1007,
            channel_id: MCS_GLOBAL_CHANNEL_ID,
            user_data: &payload,
        };
        let bytes = pdu.to_vec().unwrap();
        // 0x64 (25<<2), initiator, channel, 0x70 seg, 0x04 len, payload.
        assert_eq!(bytes[0], 0x64);
        assert_eq!(&bytes[1..3], &[0x00, 0x06]);
        assert_eq!(&bytes[3..5], &[0x03, 0xEB]);
        assert_eq!(bytes[5], 0x70);
        assert_eq!(bytes[6], 0x04);
        assert_eq!(&bytes[7..], &payload);
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn send_data_indication_long_payload() {
        let payload = vec![0xAB; 300];
        let pdu = DomainPdu::SendDataIndication {
            initiator: 1002,
            channel_id: MCS_GLOBAL_CHANNEL_ID,
            user_data: &payload,
        };
        let bytes = pdu.to_vec().unwrap();
        assert_eq!(bytes[0], 0x68); // 26 << 2
                                    // The 300-byte length uses the two-byte PER length form.
        assert_eq!(&bytes[5..8], &[0x70, 0x81, 0x2C]);
        assert_eq!(DomainPdu::decode(&bytes).unwrap(), pdu);
    }

    #[test]
    fn domain_parameters_roundtrip() {
        let dp = DomainParameters::client_target();
        let mut w = Writer::new();
        dp.encode(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(DomainParameters::decode(&mut r).unwrap(), dp);
    }

    #[test]
    fn connect_initial_roundtrip() {
        let gcc = vec![0x01, 0x02, 0x03, 0x04];
        let ci = ConnectInitial::new(gcc.clone());
        let bytes = ci.to_vec();
        // Outer tag is [APPLICATION 101] = 0x7F 0x65.
        assert_eq!(&bytes[..2], &[0x7F, 0x65]);
        let decoded = ConnectInitial::decode(&bytes).unwrap();
        assert_eq!(decoded, ci);
        assert_eq!(decoded.user_data, gcc);
    }

    #[test]
    fn connect_response_roundtrip() {
        let cr = ConnectResponse {
            result: McsResult::Successful,
            called_connect_id: 0,
            domain_parameters: DomainParameters::client_target(),
            user_data: vec![0xAA, 0xBB, 0xCC],
        };
        let bytes = cr.to_vec();
        assert_eq!(&bytes[..2], &[0x7F, 0x66]);
        assert_eq!(ConnectResponse::decode(&bytes).unwrap(), cr);
    }

    #[test]
    fn rejects_unknown_domain_pdu() {
        // choice >> 2 == 2 is not a PDU RDP uses.
        assert!(matches!(
            DomainPdu::decode(&[0x08]).unwrap_err(),
            Error::InvalidValue {
                field: "MCS domain PDU",
                ..
            }
        ));
    }
}
