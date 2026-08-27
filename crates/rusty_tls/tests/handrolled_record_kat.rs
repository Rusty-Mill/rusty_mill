//! Known-answer tests for the hand-rolled record layer, from RFC 8448.
//!
//! [RFC 8448] ("Example Handshake Traces for TLS 1.3") publishes a complete
//! 1-RTT handshake with every secret, key, IV, and wire byte spelled out.
//! That makes it the one oracle in this crate's testing strategy that is
//! *independent of rustls*: a differential test proves the two
//! implementations agree, which is worth a lot but is silent about a
//! misreading of the spec they might share — most realistically one where
//! this crate's author read rustls' source and reproduced its interpretation
//! rather than RFC 8446's text. These vectors were produced by neither
//! implementation.
//!
//! [RFC 8448]: https://www.rfc-editor.org/rfc/rfc8448
//!
//! Four records from the "Simple 1-RTT Handshake" trace are checked, chosen
//! to cover the axes the record layer actually has:
//!
//! | Record | Covers |
//! | --- | --- |
//! | client application data | AES-128-GCM, `application_data` inner type |
//! | server application data | a non-zero sequence number |
//! | client `Finished` | `handshake` inner type — the content type is inside the AEAD, so getting this right is not implied by the application-data cases |
//! | server handshake flight | a 674-byte record, well past a single AES block |
//!
//! Every one is AES-128-GCM, which is not a coincidence but is worth naming:
//! rustls' `AeadKey` is publicly constructible only at its maximum length of
//! 32 bytes, so the differential suite cannot cover AES-128-GCM at all.
//! These vectors are the only coverage that algorithm has, which is why they
//! matter more than a KAT usually would.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::record::{Aead, ContentType, Opened, Opener, RecordError, Sealer};

/// Parse the RFC's hex dump format, ignoring all whitespace and layout.
fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(digits.len() % 2, 0, "hex input has an odd number of digits");
    digits
        .chunks(2)
        .map(|pair| {
            let byte: String = pair.iter().collect();
            u8::from_str_radix(&byte, 16).expect("hex digits parse")
        })
        .collect()
}

/// RFC 8448 §3, "client application_data" — the 50-byte payload both sides
/// send, `00 01 02 ... 31`.
fn rfc8448_payload() -> Vec<u8> {
    (0u8..=0x31).collect()
}

// ---------------------------------------------------------------------------
// Vectors, verbatim from RFC 8448 §3 (Simple 1-RTT Handshake).
// ---------------------------------------------------------------------------

const CLIENT_APP_KEY: &str = "17 42 2d da 59 6e d5 d9 ac d8 90 e3 c6 3f 50 51";
const CLIENT_APP_IV: &str = "5b 78 92 3d ee 08 57 90 33 e5 23 d9";
const CLIENT_APP_RECORD: &str = "
    17 03 03 00 43 a2 3f 70 54 b6 2c 94 d0 af fa fe 82 28 ba 55
    cb ef ac ea 42 f9 14 aa 66 bc ab 3f 2b 98 19 a8 a5 b4 6b 39
    5b d5 4a 9a 20 44 1e 2b 62 97 4e 1f 5a 62 92 a2 97 70 14 bd
    1e 3d ea e6 3a ee bb 21 69 49 15 e4";

const SERVER_APP_KEY: &str = "9f 02 28 3b 6c 9c 07 ef c2 6b b9 f2 ac 92 e3 56";
const SERVER_APP_IV: &str = "cf 78 2b 88 dd 83 54 9a ad f1 e9 84";
const SERVER_APP_RECORD: &str = "
    17 03 03 00 43 2e 93 7e 11 ef 4a c7 40 e5 38 ad 36 00 5f c4
    a4 69 32 fc 32 25 d0 5f 82 aa 1b 36 e3 0e fa f9 7d 90 e6 df
    fc 60 2d cb 50 1a 59 a8 fc c4 9c 4b f2 e5 f0 a2 1c 00 47 c2
    ab f3 32 54 0d d0 32 e1 67 c2 95 5d";

const CLIENT_HS_KEY: &str = "db fa a6 93 d1 76 2c 5b 66 6a f5 d9 50 25 8d 01";
const CLIENT_HS_IV: &str = "5b d3 c7 1b 83 6e 0b 76 bb 73 26 5f";
const CLIENT_FINISHED_RECORD: &str = "
    17 03 03 00 35 75 ec 4d c2 38 cc e6 0b 29 80 44 a7 1e 21 9c
    56 cc 77 b0 51 7f e9 b9 3c 7a 4b fc 44 d8 7f 38 f8 03 38 ac
    98 fc 46 de b3 84 bd 1c ae ac ab 68 67 d7 26 c4 05 46";

const SERVER_HS_KEY: &str = "3f ce 51 60 09 c2 17 27 d0 f2 e4 e8 6e e4 03 bc";
const SERVER_HS_IV: &str = "5d 31 3e b2 67 12 76 ee 13 00 0b 30";
const SERVER_HS_RECORD: &str = "
    17 03 03 02 a2 d1 ff 33 4a 56 f5 bf f6 59 4a 07
    cc 87 b5 80 23 3f 50 0f 45 e4 89 e7 f3 3a f3 5e
    df 78 69 fc f4 0a a4 0a a2 b8 ea 73 f8 48 a7 ca
    07 61 2e f9 f9 45 cb 96 0b 40 68 90 51 23 ea 78
    b1 11 b4 29 ba 91 91 cd 05 d2 a3 89 28 0f 52 61
    34 aa dc 7f c7 8c 4b 72 9d f8 28 b5 ec f7 b1 3b
    d9 ae fb 0e 57 f2 71 58 5b 8e a9 bb 35 5c 7c 79
    02 07 16 cf b9 b1 18 3e f3 ab 20 e3 7d 57 a6 b9
    d7 47 76 09 ae e6 e1 22 a4 cf 51 42 73 25 25 0c
    7d 0e 50 92 89 44 4c 9b 3a 64 8f 1d 71 03 5d 2e
    d6 5b 0e 3c dd 0c ba e8 bf 2d 0b 22 78 12 cb b3
    60 98 72 55 cc 74 41 10 c4 53 ba a4 fc d6 10 92
    8d 80 98 10 e4 b7 ed 1a 8f d9 91 f0 6a a6 24 82
    04 79 7e 36 a6 a7 3b 70 a2 55 9c 09 ea d6 86 94
    5b a2 46 ab 66 e5 ed d8 04 4b 4c 6d e3 fc f2 a8
    94 41 ac 66 27 2f d8 fb 33 0e f8 19 05 79 b3 68
    45 96 c9 60 bd 59 6e ea 52 0a 56 a8 d6 50 f5 63
    aa d2 74 09 96 0d ca 63 d3 e6 88 61 1e a5 e2 2f
    44 15 cf 95 38 d5 1a 20 0c 27 03 42 72 96 8a 26
    4e d6 54 0c 84 83 8d 89 f7 2c 24 46 1a ad 6d 26
    f5 9e ca ba 9a cb bb 31 7b 66 d9 02 f4 f2 92 a3
    6a c1 b6 39 c6 37 ce 34 31 17 b6 59 62 22 45 31
    7b 49 ee da 0c 62 58 f1 00 d7 d9 61 ff b1 38 64
    7e 92 ea 33 0f ae ea 6d fa 31 c7 a8 4d c3 bd 7e
    1b 7a 6c 71 78 af 36 87 90 18 e3 f2 52 10 7f 24
    3d 24 3d c7 33 9d 56 84 c8 b0 37 8b f3 02 44 da
    8c 87 c8 43 f5 e5 6e b4 c5 e8 28 0a 2b 48 05 2c
    f9 3b 16 49 9a 66 db 7c ca 71 e4 59 94 26 f7 d4
    61 e6 6f 99 88 2b d8 9f c5 08 00 be cc a6 2d 6c
    74 11 6d bd 29 72 fd a1 fa 80 f8 5d f8 81 ed be
    5a 37 66 89 36 b3 35 58 3b 59 91 86 dc 5c 69 18
    a3 96 fa 48 a1 81 d6 b6 fa 4f 9d 62 d5 13 af bb
    99 2f 2b 99 2f 67 f8 af e6 7f 76 91 3f a3 88 cb
    56 30 c8 ca 01 e0 c6 5d 11 c6 6a 1e 2a c4 c8 59
    77 b7 c7 a6 99 9b bf 10 dc 35 ae 69 f5 51 56 14
    63 6c 0b 9b 68 c1 9e d2 e3 1c 0b 3b 66 76 30 38
    eb ba 42 f3 b3 8e dc 03 99 f3 a9 f2 3f aa 63 97
    8c 31 7f c9 fa 66 a7 3f 60 f0 50 4d e9 3b 5b 84
    5e 27 55 92 c1 23 35 ee 34 0b bc 4f dd d5 02 78
    40 16 e4 b3 be 7e f0 4d da 49 f4 b4 40 a3 0c b5
    d2 af 93 98 28 fd 4a e3 79 4e 44 f9 4d f5 a6 31
    ed e4 2c 17 19 bf da bf 02 53 fe 51 75 be 89 8e
    75 0e dc 53 37 0d 2b";

/// The sequence number of the server's application-data record in the trace.
///
/// Not zero, because the server's *first* record under the application
/// traffic key is a `NewSessionTicket`, not the application data — the
/// handshake ends, tickets go out, then the payload. That makes this vector
/// the one that actually exercises the §5.3 nonce construction: at sequence
/// zero the nonce is just the IV, so a completely broken XOR would still pass
/// every other case in this file.
const SERVER_APP_SEQ: u64 = 1;

// ---------------------------------------------------------------------------

/// Both directions of one vector: sealing must reproduce the RFC's exact
/// bytes, and opening must recover the exact plaintext and content type.
fn check_vector(
    name: &str,
    key: &str,
    iv: &str,
    seq: u64,
    record: &str,
    typ: ContentType,
    fragment: &[u8],
) {
    let (key, iv, record) = (hex(key), hex(iv), hex(record));

    let mut opener = Opener::new_at(Aead::Aes128Gcm, &key, &iv, seq).expect("opener builds");
    let opened = opener
        .open(&record)
        .unwrap_or_else(|e| panic!("{name}: opening the RFC 8448 record failed: {e}"));
    assert_eq!(opened.typ, typ, "{name}: inner content type");
    assert_eq!(
        opened.fragment, fragment,
        "{name}: recovered fragment differs from the RFC's plaintext"
    );
    assert_eq!(
        opener.sequence(),
        Some(seq + 1),
        "{name}: sequence advanced"
    );

    let mut sealer = Sealer::new_at(Aead::Aes128Gcm, &key, &iv, seq).expect("sealer builds");
    let sealed = sealer
        .seal(typ, fragment, 0)
        .unwrap_or_else(|e| panic!("{name}: sealing failed: {e}"));
    assert_eq!(
        sealed, record,
        "{name}: sealed record differs from the RFC 8448 wire bytes"
    );
}

#[test]
fn rfc8448_client_application_data() {
    check_vector(
        "client application_data",
        CLIENT_APP_KEY,
        CLIENT_APP_IV,
        0,
        CLIENT_APP_RECORD,
        ContentType::ApplicationData,
        &rfc8448_payload(),
    );
}

#[test]
fn rfc8448_server_application_data_at_a_nonzero_sequence() {
    check_vector(
        "server application_data",
        SERVER_APP_KEY,
        SERVER_APP_IV,
        SERVER_APP_SEQ,
        SERVER_APP_RECORD,
        ContentType::ApplicationData,
        &rfc8448_payload(),
    );
}

/// The client's `Finished`, which is a `handshake` record — the inner content
/// type lives inside the AEAD, so this is not implied by the application-data
/// vectors above.
#[test]
fn rfc8448_client_finished_carries_the_handshake_content_type() {
    let record = hex(CLIENT_FINISHED_RECORD);
    let mut opener = Opener::new(Aead::Aes128Gcm, &hex(CLIENT_HS_KEY), &hex(CLIENT_HS_IV))
        .expect("opener builds");

    let Opened { typ, fragment } = opener.open(&record).expect("client Finished opens");

    assert_eq!(typ, ContentType::Handshake);
    // A `Finished` message: type 20 (0x14), 3-byte length 0x000020, then 32
    // bytes of verify_data for SHA-256.
    assert_eq!(fragment.len(), 36, "Finished message length");
    assert_eq!(&fragment[..4], &[0x14, 0x00, 0x00, 0x20]);

    // And sealing it back reproduces the wire bytes exactly.
    let mut sealer = Sealer::new(Aead::Aes128Gcm, &hex(CLIENT_HS_KEY), &hex(CLIENT_HS_IV))
        .expect("sealer builds");
    assert_eq!(
        sealer.seal(ContentType::Handshake, &fragment, 0).unwrap(),
        record
    );
}

/// The server's full handshake flight — 674 bytes of encrypted record, which
/// is the only vector here long enough to catch a bug that only shows up past
/// the first AES block or the first ChaCha20 keystream block.
#[test]
fn rfc8448_server_handshake_flight() {
    let record = hex(SERVER_HS_RECORD);
    assert_eq!(record.len(), 679, "5-byte header plus a 674-byte body");

    let mut opener =
        Opener::new(Aead::Aes128Gcm, &hex(SERVER_HS_KEY), &hex(SERVER_HS_IV)).expect("builds");
    let Opened { typ, fragment } = opener.open(&record).expect("server flight opens");

    assert_eq!(typ, ContentType::Handshake);
    // 674 on the wire, minus the 16-byte tag and the 1-byte content type.
    assert_eq!(fragment.len(), 657);
    // The flight opens with EncryptedExtensions, handshake type 8.
    assert_eq!(fragment[0], 0x08);

    let mut sealer =
        Sealer::new(Aead::Aes128Gcm, &hex(SERVER_HS_KEY), &hex(SERVER_HS_IV)).expect("builds");
    assert_eq!(
        sealer.seal(ContentType::Handshake, &fragment, 0).unwrap(),
        record
    );
}

/// The sequence number is not decoration: the same record at the wrong
/// sequence must fail to authenticate.
///
/// Without this, a nonce construction that ignored the sequence number
/// entirely would still pass every vector above whose sequence is zero.
#[test]
fn rfc8448_records_do_not_open_at_the_wrong_sequence() {
    let record = hex(CLIENT_APP_RECORD);
    for wrong in [1u64, 2, 255, 256, u64::MAX] {
        let mut opener = Opener::new_at(
            Aead::Aes128Gcm,
            &hex(CLIENT_APP_KEY),
            &hex(CLIENT_APP_IV),
            wrong,
        )
        .expect("builds");
        assert_eq!(
            opener.open(&record),
            Err(RecordError::Decrypt),
            "the sequence-0 record must not open at sequence {wrong}"
        );
    }
}

/// The published traffic keys are direction-specific, and using the wrong
/// one must fail rather than quietly produce garbage.
#[test]
fn rfc8448_client_record_does_not_open_under_the_server_key() {
    let mut opener =
        Opener::new(Aead::Aes128Gcm, &hex(SERVER_APP_KEY), &hex(SERVER_APP_IV)).expect("builds");
    assert_eq!(
        opener.open(&hex(CLIENT_APP_RECORD)),
        Err(RecordError::Decrypt)
    );
}
