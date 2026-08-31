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
fn security_exchange_sends_encrypted_random() {
    // No server response needed; we only inspect what the client writes.
    let mut t = RdpTransport::new(MockStream::new(Vec::new()));
    // Tiny RSA key so the encrypt succeeds on a short "random".
    let key = RsaPublicKey {
        modulus_le: vec![0xA1, 0x0C],
        exponent: 17,
    };
    t.security_exchange(1007, 1003, &key, &[42]).unwrap();

    let out = t.into_inner().outbound;
    // Peel TPKT → X.224 Data → MCS Send Data Request → Security Exchange.
    let payload = Tpkt::decode(&out).unwrap().payload.to_vec();
    let X224::Data(mcs) = X224::decode(&payload).unwrap() else {
        panic!("expected Data TPDU");
    };
    let DomainPdu::SendDataRequest { user_data, .. } = DomainPdu::decode(mcs).unwrap() else {
        panic!("expected Send Data Request");
    };
    // Security Exchange header flags = SEC_EXCHANGE_PKT.
    assert_eq!(u16::from_le_bytes([user_data[0], user_data[1]]), 0x0001);
}

#[test]
fn client_info_is_encrypted_when_session_present() {
    use crate::security::{derive_session_keys, RANDOM_LEN, SEC_ENCRYPT};

    let keys = derive_session_keys(&[1u8; RANDOM_LEN], &[2u8; RANDOM_LEN], 0x02);
    let mut session = Rc4Session::new(&keys);
    let mut t = RdpTransport::new(MockStream::new(Vec::new()));
    let info = ClientInfo::new("CORP", "alice", "secret");
    t.send_client_info(Some(&mut session), 1007, 1003, &info)
        .unwrap();

    let out = t.into_inner().outbound;
    let payload = Tpkt::decode(&out).unwrap().payload.to_vec();
    let X224::Data(mcs) = X224::decode(&payload).unwrap() else {
        panic!("expected Data TPDU");
    };
    let DomainPdu::SendDataRequest { user_data, .. } = DomainPdu::decode(mcs).unwrap() else {
        panic!("expected Send Data Request");
    };
    let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
    assert!(flags & SEC_INFO_PKT != 0);
    assert!(flags & SEC_ENCRYPT != 0);
}

#[test]
fn recv_secure_decrypts_indication() {
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    // Server-side keys are the mirror of the client's.
    let client_keys = derive_session_keys(&[3u8; RANDOM_LEN], &[4u8; RANDOM_LEN], 0x02);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);
    let mut client_session = Rc4Session::new(&client_keys);

    // Server wraps a licensing-ish payload encrypted and frames it.
    let wrapped = security::wrap_pdu(Some(&mut server_session), 0, b"server-payload");
    let indication = X224::data(
        &DomainPdu::SendDataIndication {
            initiator: 1002,
            channel_id: 1003,
            user_data: &wrapped,
        }
        .to_vec()
        .unwrap(),
    )
    .to_vec()
    .unwrap();

    let mut t = RdpTransport::new(MockStream::new(framed(indication)));
    let (channel, _flags, body) = t.recv_secure(Some(&mut client_session)).unwrap();
    assert_eq!(channel, 1003);
    assert_eq!(body, b"server-payload");
}

#[test]
fn server_crypto_extracts_key() {
    use crate::gcc::{ServerSecurityData, ENCRYPTION_METHOD_128BIT};
    use crate::security::RsaPublicKey;

    // Build a minimal proprietary certificate for a 64-bit key.
    let modulus = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let mut cert = crate::cursor::Writer::new();
    cert.write_u32_le(1); // CERT_TYPE_PROPRIETARY
    cert.write_u32_le(1);
    cert.write_u32_le(1);
    cert.write_u16_le(0x0006);
    cert.write_u16_le(0);
    cert.write_u32_le(0x3141_5352); // "RSA1"
    cert.write_u32_le(modulus.len() as u32 + 8);
    cert.write_u32_le(modulus.len() as u32 * 8);
    cert.write_u32_le(modulus.len() as u32 - 1);
    cert.write_u32_le(65537);
    cert.write_bytes(&modulus);
    cert.write_bytes(&[0u8; 8]);
    cert.write_u16_le(0x0008);
    cert.write_u16_le(0);

    let blocks = vec![UserDataBlock::ServerSecurity(ServerSecurityData {
        encryption_method: ENCRYPTION_METHOD_128BIT,
        encryption_level: 2,
        server_random: vec![0xAB; 32],
        server_certificate: cert.into_vec(),
    })];
    let crypto = server_crypto(&blocks).unwrap().unwrap();
    assert_eq!(crypto.encryption_method, ENCRYPTION_METHOD_128BIT);
    assert_eq!(crypto.server_random, vec![0xAB; 32]);
    assert_eq!(
        crypto.public_key,
        RsaPublicKey {
            modulus_le: modulus.to_vec(),
            exponent: 65537,
        }
    );
}

#[test]
fn server_crypto_none_when_no_encryption() {
    use crate::gcc::ServerSecurityData;
    let blocks = vec![UserDataBlock::ServerSecurity(ServerSecurityData {
        encryption_method: 0,
        encryption_level: 0,
        server_random: Vec::new(),
        server_certificate: Vec::new(),
    })];
    assert_eq!(server_crypto(&blocks).unwrap(), None);
}

#[test]
fn recv_event_decodes_encrypted_bitmap_update() {
    use crate::output::{BitmapData, UpdatePdu};
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    // Mirror sessions, as after a security exchange.
    let client_keys = derive_session_keys(&[3u8; RANDOM_LEN], &[4u8; RANDOM_LEN], 0x02);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);

    // Server encodes a bitmap update, encrypts it, frames it.
    let update = UpdatePdu::Bitmap(vec![BitmapData::uncompressed(
        2,
        3,
        1,
        1,
        16,
        vec![0x00, 0xF8],
    )]);
    let share = update.encode(0x1234, 1002).unwrap();
    let wrapped = security::wrap_pdu(Some(&mut server_session), 0, &share);
    let indication = X224::data(
        &DomainPdu::SendDataIndication {
            initiator: 1002,
            channel_id: 1003,
            user_data: &wrapped,
        }
        .to_vec()
        .unwrap(),
    )
    .to_vec()
    .unwrap();

    let mut t = RdpTransport::new(MockStream::new(framed(indication)));
    t.session = Some(Rc4Session::new(&client_keys));

    match t.recv_event().unwrap() {
        RdpEvent::Bitmap(rects) => {
            assert_eq!(rects.len(), 1);
            assert_eq!(rects[0].dest_left, 2);
            assert_eq!(rects[0].dest_top, 3);
        }
        other => panic!("expected Bitmap, got {other:?}"),
    }
}

#[test]
fn recv_event_decodes_encrypted_pointer_position() {
    use crate::pointer::PointerUpdate;
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    let client_keys = derive_session_keys(&[5u8; RANDOM_LEN], &[6u8; RANDOM_LEN], 0x02);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);

    let share = PointerUpdate::Position { x: 100, y: 200 }
        .encode(0x1234, 1002)
        .unwrap();
    let wrapped = security::wrap_pdu(Some(&mut server_session), 0, &share);
    let indication = X224::data(
        &DomainPdu::SendDataIndication {
            initiator: 1002,
            channel_id: 1003,
            user_data: &wrapped,
        }
        .to_vec()
        .unwrap(),
    )
    .to_vec()
    .unwrap();

    let mut t = RdpTransport::new(MockStream::new(framed(indication)));
    t.session = Some(Rc4Session::new(&client_keys));
    match t.recv_event().unwrap() {
        RdpEvent::Pointer(PointerUpdate::Position { x, y }) => {
            assert_eq!((x, y), (100, 200));
        }
        other => panic!("expected Pointer position, got {other:?}"),
    }
}

#[test]
fn recv_event_decodes_fastpath_pointer_position() {
    use crate::fastpath::{write_length, FASTPATH_ACTION, FASTPATH_UPDATETYPE_PTR_POSITION};

    // Build a plaintext fast-path output PDU carrying a pointer position.
    let mut updates = crate::cursor::Writer::new();
    updates.write_u8(FASTPATH_UPDATETYPE_PTR_POSITION);
    updates.write_u16_le(4); // size
    updates.write_u16_le(50);
    updates.write_u16_le(60);
    let body = updates.into_vec();

    let mut pdu = crate::cursor::Writer::new();
    pdu.write_u8(FASTPATH_ACTION); // action 0, not encrypted
    let total = 1 + 1 + body.len(); // header + 1-byte length + body
    write_length(&mut pdu, total).unwrap();
    pdu.write_bytes(&body);

    let mut t = RdpTransport::new(MockStream::new(pdu.into_vec()));
    match t.recv_event().unwrap() {
        RdpEvent::Pointer(crate::pointer::PointerUpdate::Position { x, y }) => {
            assert_eq!((x, y), (50, 60));
        }
        other => panic!("expected fast-path pointer, got {other:?}"),
    }
}

#[test]
fn recv_event_decodes_encrypted_fastpath_bitmap() {
    use crate::fastpath::{
        write_length, FASTPATH_ACTION, FASTPATH_ENCRYPTED, FASTPATH_SECURE_CHECKSUM,
        FASTPATH_UPDATETYPE_BITMAP,
    };
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    let client_keys = derive_session_keys(&[9u8; RANDOM_LEN], &[8u8; RANDOM_LEN], 0x02);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);

    // Update payload: one bitmap update (numberRectangles + a 1x1 rect).
    let mut updates = crate::cursor::Writer::new();
    updates.write_u8(FASTPATH_UPDATETYPE_BITMAP);
    let mut rect = crate::cursor::Writer::new();
    rect.write_u16_le(1); // numberRectangles
    for v in [7u16, 8, 7, 8, 1, 1, 16, 0, 2] {
        rect.write_u16_le(v);
    }
    rect.write_bytes(&[0x00, 0xF8]);
    let rect = rect.into_vec();
    updates.write_u16_le(rect.len() as u16);
    updates.write_bytes(&rect);
    let plaintext = updates.into_vec();

    let (signature, ciphertext) = server_session.encrypt(&plaintext);
    let mut body = signature.to_vec();
    body.extend_from_slice(&ciphertext);

    let mut pdu = crate::cursor::Writer::new();
    let flags = FASTPATH_ENCRYPTED | FASTPATH_SECURE_CHECKSUM;
    pdu.write_u8(FASTPATH_ACTION | (flags << 6));
    // Length includes the header byte and the length field itself.
    let base = 1 + body.len();
    let total = if base < 0x7F { base + 1 } else { base + 2 };
    write_length(&mut pdu, total).unwrap();
    pdu.write_bytes(&body);

    let mut t = RdpTransport::new(MockStream::new(pdu.into_vec()));
    t.session = Some(Rc4Session::new(&client_keys));
    match t.recv_event().unwrap() {
        RdpEvent::Bitmap(rects) => {
            assert_eq!(rects[0].dest_left, 7);
            assert_eq!(rects[0].dest_top, 8);
        }
        other => panic!("expected fast-path bitmap, got {other:?}"),
    }
}

#[test]
fn send_input_writes_fastpath_pdu() {
    use crate::input::InputEvent;

    let mut t = RdpTransport::new(MockStream::new(Vec::new()));
    // Unencrypted (no session): key press + release.
    t.send_input(&[InputEvent::key_press(0x1E), InputEvent::key_release(0x1E)])
        .unwrap();
    let out = t.into_inner().outbound;
    // Header: action 0, numberEvents 2 in bits 2-5 → 0x08.
    assert_eq!(out[0], 0x08);
    // length byte = total = 1 (header) + 1 (length) + 4 (two 2-byte events).
    assert_eq!(out[1], 6);
    // First event: scancode header 0x00, keycode 0x1E.
    assert_eq!(&out[2..4], &[0x00, 0x1E]);
    // Second event: RELEASE flag (0x01), keycode 0x1E.
    assert_eq!(&out[4..6], &[0x01, 0x1E]);
}

#[test]
fn establish_drives_full_standard_rdp_handshake() {
    use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
    use crate::gcc::{
        ServerCoreData, ServerNetworkData, ServerSecurityData, ENCRYPTION_METHOD_128BIT,
    };
    use crate::license::{LicenseErrorMessage, LicensePdu};
    use crate::mcs::DomainParameters;
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    let client_random = [0x5Au8; RANDOM_LEN];
    let server_random = [0xA5u8; RANDOM_LEN];

    // The server's session is the mirror of the client's derived keys.
    let client_keys = derive_session_keys(&client_random, &server_random, ENCRYPTION_METHOD_128BIT);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);

    // A minimal proprietary certificate (tiny RSA key; the mock never
    // decrypts, it only needs the client's encrypt step to succeed).
    let modulus = [0xA1u8, 0x0C];
    let mut cert = crate::cursor::Writer::new();
    cert.write_u32_le(1);
    cert.write_u32_le(1);
    cert.write_u32_le(1);
    cert.write_u16_le(0x0006);
    cert.write_u16_le(0);
    cert.write_u32_le(0x3141_5352);
    cert.write_u32_le(modulus.len() as u32 + 8);
    cert.write_u32_le(modulus.len() as u32 * 8);
    cert.write_u32_le(modulus.len() as u32 - 1);
    cert.write_u32_le(17);
    cert.write_bytes(&modulus);
    cert.write_bytes(&[0u8; 8]);
    cert.write_u16_le(0x0008);
    cert.write_u16_le(0);

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
        UserDataBlock::ServerSecurity(ServerSecurityData {
            encryption_method: ENCRYPTION_METHOD_128BIT,
            encryption_level: 2,
            server_random: server_random.to_vec(),
            server_certificate: cert.into_vec(),
        }),
    ];
    let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
    let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
    let connect_response = ConnectResponse {
        result: McsResult::Successful,
        called_connect_id: 0,
        domain_parameters: DomainParameters::client_target(),
        user_data: ccrsp,
    };

    let data_ind = |payload: &[u8]| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::SendDataIndication {
                    initiator: 1002,
                    channel_id: 1003,
                    user_data: payload,
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };

    // Licensing: encrypted valid-client alert.
    let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
        .to_vec()
        .unwrap();
    let license_wrapped = security::wrap_pdu(Some(&mut server_session), SEC_LICENSE_PKT, &license);

    // Demand Active from the server's channel (1002), encrypted.
    let demand = DemandActive {
        share_id: 0x0000_1234,
        source_descriptor: b"RDP\0".to_vec(),
        capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
        session_id: 0,
    };
    let demand_bytes = demand.encode(1002).unwrap();
    let demand_wrapped = security::wrap_pdu(Some(&mut server_session), 0, &demand_bytes);

    // Assemble the server's byte stream in the exact order the client reads.
    let confirm = |requested: u16, channel: u16| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::ChannelJoinConfirm {
                    result: McsResult::Successful,
                    initiator: 1007,
                    requested,
                    channel_id: Some(channel),
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };
    let mut inbound = Vec::new();
    // 1. Connection Confirm (RDP selected).
    inbound.extend(framed(
        X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Response {
                flags: 0,
                selected: SecurityProtocols::RDP,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap(),
    ));
    // 2. MCS Connect-Response.
    inbound.extend(framed(
        X224::data(&connect_response.to_vec()).to_vec().unwrap(),
    ));
    // 3. Attach User Confirm.
    inbound.extend(framed(
        X224::data(
            &DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(1007),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap(),
    ));
    // 4/5. Channel join confirms (user channel, then I/O channel).
    inbound.extend(confirm(1007, 1007));
    inbound.extend(confirm(1003, 1003));
    // 6. Licensing, then 7. Demand Active.
    inbound.extend(data_ind(&license_wrapped));
    inbound.extend(data_ind(&demand_wrapped));

    let mut t = RdpTransport::new(MockStream::new(inbound));
    let config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
    let session = t.establish(&config, &client_random).unwrap();

    assert_eq!(session.user_id, 1007);
    assert_eq!(session.io_channel, 1003);
    assert_eq!(session.share_id, 0x0000_1234);
    assert_eq!(session.server_channel, 1002);
}

#[test]
fn establish_requests_and_maps_extra_channel() {
    // Same handshake as above, but the client also asks for DRDYNVC and
    // the server grants it a channel id; establish() should surface that
    // mapping on the returned session.
    use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
    use crate::gcc::{
        ServerCoreData, ServerNetworkData, ServerSecurityData, ENCRYPTION_METHOD_128BIT,
    };
    use crate::license::{LicenseErrorMessage, LicensePdu};
    use crate::mcs::DomainParameters;
    use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

    let client_random = [0x5Au8; RANDOM_LEN];
    let server_random = [0xA5u8; RANDOM_LEN];
    let client_keys = derive_session_keys(&client_random, &server_random, ENCRYPTION_METHOD_128BIT);
    let server_keys = SessionKeys {
        mac_key: client_keys.mac_key.clone(),
        encrypt_key: client_keys.decrypt_key.clone(),
        decrypt_key: client_keys.encrypt_key.clone(),
    };
    let mut server_session = Rc4Session::new(&server_keys);

    let modulus = [0xA1u8, 0x0C];
    let mut cert = crate::cursor::Writer::new();
    cert.write_u32_le(1);
    cert.write_u32_le(1);
    cert.write_u32_le(1);
    cert.write_u16_le(0x0006);
    cert.write_u16_le(0);
    cert.write_u32_le(0x3141_5352);
    cert.write_u32_le(modulus.len() as u32 + 8);
    cert.write_u32_le(modulus.len() as u32 * 8);
    cert.write_u32_le(modulus.len() as u32 - 1);
    cert.write_u32_le(17);
    cert.write_bytes(&modulus);
    cert.write_bytes(&[0u8; 8]);
    cert.write_u16_le(0x0008);
    cert.write_u16_le(0);

    // The server grants the one requested extra channel as id 1004.
    let server_blocks = vec![
        UserDataBlock::ServerCore(ServerCoreData {
            version: 0x0008_0004,
            client_requested_protocols: Some(0),
            early_capability_flags: None,
        }),
        UserDataBlock::ServerNetwork(ServerNetworkData {
            io_channel_id: 1003,
            channel_ids: vec![1004],
        }),
        UserDataBlock::ServerSecurity(ServerSecurityData {
            encryption_method: ENCRYPTION_METHOD_128BIT,
            encryption_level: 2,
            server_random: server_random.to_vec(),
            server_certificate: cert.into_vec(),
        }),
    ];
    let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
    let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
    let connect_response = ConnectResponse {
        result: McsResult::Successful,
        called_connect_id: 0,
        domain_parameters: DomainParameters::client_target(),
        user_data: ccrsp,
    };

    let data_ind = |payload: &[u8]| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::SendDataIndication {
                    initiator: 1002,
                    channel_id: 1003,
                    user_data: payload,
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };
    let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
        .to_vec()
        .unwrap();
    let license_wrapped = security::wrap_pdu(Some(&mut server_session), SEC_LICENSE_PKT, &license);
    let demand = DemandActive {
        share_id: 0x0000_1234,
        source_descriptor: b"RDP\0".to_vec(),
        capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
        session_id: 0,
    };
    let demand_bytes = demand.encode(1002).unwrap();
    let demand_wrapped = security::wrap_pdu(Some(&mut server_session), 0, &demand_bytes);

    let confirm = |requested: u16, channel: u16| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::ChannelJoinConfirm {
                    result: McsResult::Successful,
                    initiator: 1007,
                    requested,
                    channel_id: Some(channel),
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };
    let mut inbound = Vec::new();
    inbound.extend(framed(
        X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Response {
                flags: 0,
                selected: SecurityProtocols::RDP,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap(),
    ));
    inbound.extend(framed(
        X224::data(&connect_response.to_vec()).to_vec().unwrap(),
    ));
    inbound.extend(framed(
        X224::data(
            &DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(1007),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap(),
    ));
    // User channel, I/O channel, then the extra DRDYNVC channel.
    inbound.extend(confirm(1007, 1007));
    inbound.extend(confirm(1003, 1003));
    inbound.extend(confirm(1004, 1004));
    inbound.extend(data_ind(&license_wrapped));
    inbound.extend(data_ind(&demand_wrapped));

    let mut t = RdpTransport::new(MockStream::new(inbound));
    let mut config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
    config.extra_channels.push(ChannelDef {
        name: crate::dvc::DRDYNVC_CHANNEL_NAME.to_string(),
        options: 0,
    });
    let session = t.establish(&config, &client_random).unwrap();

    assert_eq!(
        session.channel_id(crate::dvc::DRDYNVC_CHANNEL_NAME),
        Some(1004)
    );
    assert_eq!(session.channel_id("not-requested"), None);
}

#[test]
fn recv_event_reassembles_channel_data_across_chunks() {
    // Once the I/O channel is known, traffic on any other channel is
    // virtual-channel data (MS-RDPBCGR 2.2.6.1), reassembled and surfaced
    // as RdpEvent::ChannelData only once the last chunk arrives.
    let message = b"a dynamic-channel payload that needs no encryption";
    let chunks = crate::vchan::chunk(message, 16);
    assert!(chunks.len() > 1, "test needs a fragmented message");

    let data_ind = |payload: &[u8]| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::SendDataIndication {
                    initiator: 1002,
                    channel_id: 1004, // not the I/O channel
                    user_data: payload,
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };

    // Every Send Data Indication carries a Basic Security Header, even
    // with no session active (flags = 0, no SEC_ENCRYPT).
    let mut inbound = Vec::new();
    for c in &chunks {
        let wrapped = security::wrap_pdu(None, 0, c);
        inbound.extend(data_ind(&wrapped));
    }
    let mut t = RdpTransport::new(MockStream::new(inbound));
    t.io_channel = Some(1003);

    match t.recv_event().unwrap() {
        RdpEvent::ChannelData { channel_id, data } => {
            assert_eq!(channel_id, 1004);
            assert_eq!(data, message);
        }
        other => panic!("expected ChannelData, got {other:?}"),
    }
}

#[test]
fn establish_enhanced_drives_tls_handshake() {
    // The TLS path: no Security Exchange, no RC4. Licensing carries a
    // SEC_LICENSE_PKT header; the Demand Active and everything after are
    // bare Share Control PDUs.
    use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
    use crate::gcc::{ServerCoreData, ServerNetworkData};
    use crate::license::{LicenseErrorMessage, LicensePdu};
    use crate::mcs::DomainParameters;

    // Server security block advertises no RDP encryption (TLS handles it).
    let server_blocks = vec![
        UserDataBlock::ServerCore(ServerCoreData {
            version: 0x0008_0004,
            client_requested_protocols: Some(SecurityProtocols::SSL.0),
            early_capability_flags: None,
        }),
        UserDataBlock::ServerNetwork(ServerNetworkData {
            io_channel_id: 1003,
            channel_ids: vec![],
        }),
    ];
    let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
    let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
    let connect_response = ConnectResponse {
        result: McsResult::Successful,
        called_connect_id: 0,
        domain_parameters: DomainParameters::client_target(),
        user_data: ccrsp,
    };

    let data_ind = |payload: &[u8]| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::SendDataIndication {
                    initiator: 1002,
                    channel_id: 1003,
                    user_data: payload,
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };
    let confirm = |requested: u16, channel: u16| -> Vec<u8> {
        framed(
            X224::data(
                &DomainPdu::ChannelJoinConfirm {
                    result: McsResult::Successful,
                    initiator: 1007,
                    requested,
                    channel_id: Some(channel),
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        )
    };

    // Licensing PDU: valid-client alert under a plaintext SEC_LICENSE_PKT
    // header (no encryption).
    let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
        .to_vec()
        .unwrap();
    let license_wrapped = security::wrap_pdu(None, SEC_LICENSE_PKT, &license);

    // Demand Active: a bare Share Control PDU (no security header).
    let demand = DemandActive {
        share_id: 0x0000_1234,
        source_descriptor: b"RDP\0".to_vec(),
        capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
        session_id: 0,
    };
    let demand_bytes = demand.encode(1002).unwrap();

    let mut inbound = Vec::new();
    // No Connection Confirm here: negotiation already happened on raw TCP.
    // 1. MCS Connect-Response.
    inbound.extend(framed(
        X224::data(&connect_response.to_vec()).to_vec().unwrap(),
    ));
    // 2. Attach User Confirm.
    inbound.extend(framed(
        X224::data(
            &DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(1007),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap(),
    ));
    // 3/4. Channel join confirms.
    inbound.extend(confirm(1007, 1007));
    inbound.extend(confirm(1003, 1003));
    // 5. Licensing, then 6. Demand Active (headerless).
    inbound.extend(data_ind(&license_wrapped));
    inbound.extend(data_ind(&demand_bytes));

    let mut t = RdpTransport::new_enhanced(MockStream::new(inbound));
    let config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
    let session = t
        .establish_enhanced(&config, SecurityProtocols::SSL)
        .unwrap();

    assert_eq!(session.user_id, 1007);
    assert_eq!(session.io_channel, 1003);
    assert_eq!(session.share_id, 0x0000_1234);
    assert_eq!(session.server_channel, 1002);

    // The client sent no Security Exchange PDU; the Client Info rode under
    // a plaintext SEC_INFO_PKT header (no SEC_ENCRYPT).
    let out = t.into_inner().outbound;
    let mut info_seen = false;
    for user_data in split_send_data_requests(&out) {
        if user_data.len() < 4 {
            continue;
        }
        let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
        // No PDU carries a Security Exchange header under TLS.
        assert_eq!(flags & crate::security::SEC_EXCHANGE_PKT, 0);
        if flags & SEC_INFO_PKT != 0 {
            info_seen = true;
            // Client Info is in the clear under TLS.
            assert_eq!(flags & crate::security::SEC_ENCRYPT, 0);
        }
    }
    assert!(info_seen, "Client Info PDU was not sent");
}

/// Split a client's outbound byte stream into the user-data payloads of
/// each MCS Send Data Request it carries.
fn split_send_data_requests(mut out: &[u8]) -> Vec<Vec<u8>> {
    let mut payloads = Vec::new();
    while out.len() >= TPKT_HEADER_LEN {
        let Some(total) = Tpkt::peek_total_len(&out[..TPKT_HEADER_LEN]).unwrap() else {
            break;
        };
        if total > out.len() {
            break;
        }
        let (packet, rest) = out.split_at(total);
        out = rest;
        let tpkt = Tpkt::decode(packet).unwrap();
        if let Ok(X224::Data(mcs)) = X224::decode(tpkt.payload) {
            if let Ok(DomainPdu::SendDataRequest { user_data, .. }) = DomainPdu::decode(mcs) {
                payloads.push(user_data.to_vec());
            }
        }
    }
    payloads
}

#[test]
fn establish_enhanced_bitmap_has_no_security_header() {
    // After an enhanced-mode session, a bitmap update arrives as a bare
    // Share Data PDU with no Basic Security Header.
    use crate::output::{BitmapData, UpdatePdu};

    let update = UpdatePdu::Bitmap(vec![BitmapData::uncompressed(
        2,
        3,
        1,
        1,
        16,
        vec![0x00, 0xF8],
    )]);
    let share = update.encode(0x1234, 1002).unwrap();
    let indication = X224::data(
        &DomainPdu::SendDataIndication {
            initiator: 1002,
            channel_id: 1003,
            user_data: &share,
        }
        .to_vec()
        .unwrap(),
    )
    .to_vec()
    .unwrap();

    let mut t = RdpTransport::new_enhanced(MockStream::new(framed(indication)));
    match t.recv_event().unwrap() {
        RdpEvent::Bitmap(rects) => {
            assert_eq!(rects.len(), 1);
            assert_eq!(rects[0].dest_left, 2);
            assert_eq!(rects[0].dest_top, 3);
        }
        other => panic!("expected Bitmap, got {other:?}"),
    }
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

/// End-to-end: [`RdpTransport::accept`] against a hand-driven client that
/// speaks the same sequence [`RdpTransport::establish`] would (negotiate,
/// MCS connect, channel setup, unencrypted Client Info, licensing,
/// capability exchange, finalization) over a real TCP loopback
/// connection — exercising the full wire protocol both directions, not
/// just one side's framing in isolation.
///
/// A hand-driven client rather than `establish()` itself: `establish()`
/// requires the server to select encryption (real standard RDP security
/// needs RSA, which this crate's server side does not implement yet),
/// so it cannot speak `accept`'s unencrypted `encryptionLevel = 0` mode.
/// Any real RDP client can, though — `accept` only has to speak the wire
/// protocol correctly, which is what this test checks.
#[test]
fn accept_completes_full_connection_sequence_with_a_real_client() {
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut t = RdpTransport::new(stream);
        t.accept(&AcceptConfig::new(1024, 768)).unwrap()
    });

    let stream = TcpStream::connect(addr).unwrap();
    let mut client = RdpTransport::new(stream);

    // 1. X.224 negotiation.
    let selected = client
        .negotiate(SecurityProtocols::RDP, Some("alice"))
        .unwrap();
    assert_eq!(selected, SecurityProtocols::RDP);

    // 2. GCC/MCS connect, requesting one virtual channel.
    let mut core = ClientCoreData::new(1024, 768, "test-client");
    core.server_selected_protocol = Some(0);
    let client_blocks = vec![
        UserDataBlock::ClientCore(core),
        UserDataBlock::ClientSecurity(ClientSecurityData {
            encryption_methods: 0,
            ext_encryption_methods: 0,
        }),
        UserDataBlock::ClientNetwork(ClientNetworkData {
            channels: vec![ChannelDef {
                name: "rdpdr".to_string(),
                options: 0,
            }],
        }),
        UserDataBlock::ClientCluster(ClientClusterData {
            flags: 0x0D,
            redirected_session_id: 0,
        }),
    ];
    let server_blocks = client.mcs_connect(&client_blocks).unwrap();
    // encryptionLevel = 0: no server random/certificate to derive keys from.
    assert_eq!(server_crypto(&server_blocks).unwrap(), None);

    let io_channel = server_blocks
        .iter()
        .find_map(|b| match b {
            UserDataBlock::ServerNetwork(net) => Some(net.io_channel_id),
            _ => None,
        })
        .unwrap();
    let virtual_channels: Vec<u16> = server_blocks
        .iter()
        .find_map(|b| match b {
            UserDataBlock::ServerNetwork(net) => Some(net.channel_ids.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(virtual_channels.len(), 1);

    // 3. Channel setup.
    client.erect_domain().unwrap();
    let user_id = client.attach_user().unwrap();
    client.join_channel(user_id, user_id).unwrap();
    client.join_channel(user_id, io_channel).unwrap();
    for &vc in &virtual_channels {
        client.join_channel(user_id, vc).unwrap();
    }

    // 4. Client Info, in the clear (encryptionLevel = 0, no RC4 session).
    let info = ClientInfo::new("CORP", "alice", "secret");
    client
        .send_client_info(None, user_id, io_channel, &info)
        .unwrap();

    // 5. Licensing: expect "no license required".
    let (_channel, data) = client.recv_data().unwrap();
    let (flags, body) = security::unwrap_pdu(None, &data).unwrap();
    assert!(flags & SEC_LICENSE_PKT != 0);
    match LicensePdu::decode(&body).unwrap() {
        LicensePdu::ErrorAlert(msg) => assert!(msg.is_valid_client()),
        other => panic!("expected ErrorAlert, got {other:?}"),
    }

    // 6. Capability exchange — headerless past licensing, same as the
    //    TLS/`establish_enhanced` path, since encryptionLevel = 0 also
    //    carries no Basic Security Header on Share Control/Data PDUs.
    let (_channel, demand_body) = client.recv_data().unwrap();
    let (server_channel, demand) = DemandActive::decode(&demand_body).unwrap();
    assert_eq!(demand.source_descriptor, b"RDP\0");

    let confirm = ConfirmActive::new(demand.share_id, client_capability_sets(1024, 768, 16));
    let confirm_bytes = confirm.encode(user_id).unwrap();
    client
        .send_data(user_id, io_channel, &confirm_bytes)
        .unwrap();

    // 7. Client finalization sequence.
    for pdu in client_finalization_sequence(server_channel) {
        let bytes = pdu.encode(demand.share_id, user_id).unwrap();
        client.send_data(user_id, io_channel, &bytes).unwrap();
    }

    // 8. Server finalization sequence in reply.
    let mut got_font_map = false;
    for _ in 0..4 {
        let (_channel, body) = client.recv_data().unwrap();
        let (_source, share_id, pdu) = FinalizationPdu::decode(&body).unwrap();
        assert_eq!(share_id, demand.share_id);
        if matches!(pdu, FinalizationPdu::FontMap(_)) {
            got_font_map = true;
        }
    }
    assert!(got_font_map);

    let accepted = server.join().unwrap();
    assert_eq!(accepted.user_id, user_id);
    assert_eq!(accepted.io_channel, io_channel);
    assert_eq!(accepted.share_id, demand.share_id);
    assert_eq!(accepted.channels.get("rdpdr"), Some(&virtual_channels[0]));
    assert_eq!(accepted.client_info.username, "alice");
    assert_eq!(accepted.client_info.domain, "CORP");
}

/// A 512-bit RSA test key pair (freshly generated for this test, not
/// used anywhere else), little-endian as this crate's `RsaPublicKey`/
/// `RsaPrivateKey` expect.
fn test_rsa_key_pair() -> (RsaPublicKey, RsaPrivateKey) {
    #[rustfmt::skip]
        let modulus_le: [u8; 64] = [
            0x81, 0x7d, 0xb9, 0xf7, 0x70, 0xef, 0x15, 0xb1, 0x2e, 0xfb, 0x94, 0x37,
            0x9b, 0x70, 0xae, 0x91, 0x99, 0x23, 0x71, 0xac, 0x86, 0x1d, 0xc6, 0xf7,
            0xef, 0x18, 0x82, 0xce, 0x38, 0x5a, 0xcc, 0xc6, 0xee, 0xb6, 0x82, 0x24,
            0x9f, 0xe9, 0x76, 0x00, 0x58, 0x1b, 0xde, 0xc9, 0x63, 0x09, 0x3f, 0x26,
            0x66, 0xcb, 0xd0, 0x2c, 0x0c, 0x5f, 0xf5, 0x48, 0xb6, 0xd8, 0x48, 0x08,
            0xe4, 0x31, 0xbe, 0xb4,
        ];
    #[rustfmt::skip]
        let private_exponent_le: [u8; 64] = [
            0x11, 0xc9, 0x5c, 0x53, 0xd8, 0x32, 0x7f, 0x57, 0x10, 0xba, 0xef, 0x8c,
            0x1d, 0x9f, 0x66, 0xa4, 0x11, 0xb0, 0x41, 0xea, 0x85, 0xce, 0x57, 0x17,
            0xe1, 0x23, 0x7e, 0xfc, 0x2a, 0x84, 0x44, 0x89, 0xb8, 0x87, 0x1c, 0x82,
            0x35, 0xe2, 0x90, 0x19, 0xce, 0x56, 0x4c, 0xbc, 0x46, 0x9d, 0x14, 0x71,
            0x97, 0x68, 0xa1, 0xd6, 0x4a, 0xee, 0x5a, 0x66, 0xa5, 0x78, 0xb8, 0xe8,
            0x02, 0xb8, 0x35, 0x84,
        ];
    (
        RsaPublicKey {
            modulus_le: modulus_le.to_vec(),
            exponent: 65537,
        },
        RsaPrivateKey {
            modulus_le: modulus_le.to_vec(),
            private_exponent_le: private_exponent_le.to_vec(),
        },
    )
}

/// Same as [`accept_completes_full_connection_sequence_with_a_real_client`]
/// but with [`AcceptConfig::encryption`] set, driving real encrypted
/// standard RDP security end to end over a real TCP loopback connection
/// — and this time the client is the real [`RdpTransport::establish`]
/// rather than a hand-driven one, since `establish` requires the server
/// to select encryption, which this configuration now does.
#[test]
fn accept_with_encryption_completes_full_connection_sequence_with_establish() {
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    let (public_key, private_key) = test_rsa_key_pair();
    let server_random = [0x77u8; RANDOM_LEN];
    let client_random = [0x99u8; RANDOM_LEN];

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut t = RdpTransport::new(stream);
        let mut config = AcceptConfig::new(1024, 768);
        config.encryption = Some(AcceptEncryption {
            public_key,
            private_key,
            server_random,
        });
        (t.accept(&config).unwrap(), t.session.is_some())
    });

    let stream = TcpStream::connect(addr).unwrap();
    let mut client = RdpTransport::new(stream);
    let mut establish_config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
    establish_config.extra_channels = vec![ChannelDef {
        name: "rdpdr".to_string(),
        options: 0,
    }];
    let session = client.establish(&establish_config, &client_random).unwrap();

    let (accepted, server_had_session) = server.join().unwrap();
    assert!(
        server_had_session,
        "accept() should have set up an Rc4Session"
    );
    assert!(
        client.session.is_some(),
        "establish() should have set up an Rc4Session"
    );
    assert_eq!(accepted.user_id, session.user_id);
    assert_eq!(accepted.io_channel, session.io_channel);
    assert_eq!(accepted.share_id, session.share_id);
    assert_eq!(
        accepted.channels.get("rdpdr"),
        session.channel_id("rdpdr").as_ref()
    );
    assert_eq!(accepted.client_info.username, "alice");
    assert_eq!(accepted.client_info.domain, "CORP");
}
