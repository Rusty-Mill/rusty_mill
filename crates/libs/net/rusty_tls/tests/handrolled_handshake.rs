//! Handshake messages and the transcript, against RFC 8448.
//!
//! Three properties, in order of how much they are worth:
//!
//! 1. **The transcript hashes match.** Parsing the RFC's messages, feeding
//!    them to [`Transcript`], and hashing gives exactly the values RFC 8448
//!    publishes — which is what stage 3a's tests took as *inputs*. The two
//!    stages now close on each other.
//! 2. **Round-tripping is byte-identical.** Parse the RFC's ClientHello,
//!    encode it, get the same 196 octets. The transcript covers encoded
//!    messages, so a parser and encoder that are not inverses compute a hash
//!    the peer does not share.
//! 3. **The fields are what the RFC says.** Ordinary, and the least of the
//!    three: a parser can read every field correctly and still be useless if
//!    the transcript is wrong.
//!
//! # What this closes
//!
//! Stage 3a could not assert the server's Finished `verify_data`, because its
//! transcript runs through the server's CertificateVerify and RFC 8448 does
//! not publish that hash as a labelled value. It is computable from the
//! messages, and `the_server_finished_verify_data_now_checks_out` computes
//! it — closing a gap 3a documented rather than hid.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::handshake::{
    certificate_verify_content, extension, find, messages, parse_encrypted_extensions,
    parse_finished, pre_shared_key_placeholder, BinderHello, CertificateMessage, CertificateVerify,
    ClientHello, Extension, HandshakeError, HandshakeType, Message, PresharedKeyOffer, PskIdentity,
    ServerHello, Transcript, SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use rusty_tls::handrolled::schedule::{finished_verify_data, Hash, KeySchedule};
use rusty_tls::handrolled::wire::WireError;

mod rfc8448;

use rfc8448::{
    hex, CLIENT_FINISHED, CLIENT_HELLO, SERVER_FLIGHT, SERVER_HELLO, SERVER_VERIFY_DATA,
    SHARED_SECRET, TRANSCRIPT_CLIENT_FINISHED, TRANSCRIPT_HELLO, TRANSCRIPT_SERVER_FINISHED,
};

// ---------------------------------------------------------------------------
// The transcript — the property worth the most
// ---------------------------------------------------------------------------

/// Every transcript hash RFC 8448 publishes, computed from the messages.
///
/// Stage 3a asserted the key schedule *given* these hashes. This computes
/// them, so the two stages now meet: the schedule's inputs are no longer
/// pasted constants but the output of parsing the RFC's own messages.
#[test]
fn every_published_transcript_hash_is_reproduced() {
    let mut transcript = Transcript::new(Hash::Sha256);

    transcript.add(&hex(CLIENT_HELLO));
    transcript.add(&hex(SERVER_HELLO));
    assert_eq!(
        transcript.hash(),
        hex(TRANSCRIPT_HELLO),
        "Hash(ClientHello..ServerHello)"
    );

    // The server's flight is four concatenated messages inside one record.
    transcript.add(&hex(SERVER_FLIGHT));
    assert_eq!(
        transcript.hash(),
        hex(TRANSCRIPT_SERVER_FINISHED),
        "Hash(ClientHello..server Finished)"
    );

    transcript.add(&hex(CLIENT_FINISHED));
    assert_eq!(
        transcript.hash(),
        hex(TRANSCRIPT_CLIENT_FINISHED),
        "Hash(ClientHello..client Finished)"
    );
}

/// Adding messages one at a time must equal adding them in one blob — a
/// caller reassembling records has no control over how a peer splits them.
#[test]
fn the_transcript_does_not_depend_on_how_messages_were_batched() {
    let flight = hex(SERVER_FLIGHT);
    let parsed = messages(&flight).expect("the flight parses");
    assert_eq!(
        parsed.len(),
        4,
        "EE, Certificate, CertificateVerify, Finished"
    );

    let mut one_blob = Transcript::new(Hash::Sha256);
    one_blob.add(&hex(CLIENT_HELLO));
    one_blob.add(&hex(SERVER_HELLO));
    one_blob.add(&flight);

    let mut one_at_a_time = Transcript::new(Hash::Sha256);
    one_at_a_time.add(&hex(CLIENT_HELLO));
    one_at_a_time.add(&hex(SERVER_HELLO));
    for message in &parsed {
        one_at_a_time.add_message(message);
    }

    assert_eq!(one_blob.hash(), one_at_a_time.hash());
    assert_eq!(one_at_a_time.hash(), hex(TRANSCRIPT_SERVER_FINISHED));
}

/// Stage 3a left this gap open and said so. It is now closed.
///
/// The server's Finished `verify_data` is a MAC over
/// `Hash(ClientHello..CertificateVerify)`, which RFC 8448 does not publish as
/// a labelled value. With the messages parseable, the hash is computable — and
/// the MAC over it matches the RFC's published `verify_data`.
///
/// Three stages have to be simultaneously correct for this to pass: the
/// transcript (3b), the key schedule (3a), and the Finished MAC (3a).
#[test]
fn the_server_finished_verify_data_now_checks_out() {
    let flight = hex(SERVER_FLIGHT);
    let parsed = messages(&flight).expect("parses");

    // Everything through CertificateVerify — the flight minus its Finished.
    let mut transcript = Transcript::new(Hash::Sha256);
    transcript.add(&hex(CLIENT_HELLO));
    transcript.add(&hex(SERVER_HELLO));
    for message in &parsed {
        if message.typ == HandshakeType::Finished {
            break;
        }
        transcript.add_message(message);
    }

    let handshake = KeySchedule::new(Hash::Sha256).into_handshake(&hex(SHARED_SECRET));
    let secret = handshake.derive("s hs traffic", &hex(TRANSCRIPT_HELLO));

    assert_eq!(
        finished_verify_data(Hash::Sha256, &secret, &transcript.hash()),
        hex(SERVER_VERIFY_DATA),
        "the server's Finished verify_data"
    );

    // And the Finished message in the flight carries exactly that value.
    let finished = parsed
        .iter()
        .find(|m| m.typ == HandshakeType::Finished)
        .expect("the flight ends with Finished");
    assert_eq!(
        parse_finished(finished.body).expect("non-empty"),
        hex(SERVER_VERIFY_DATA)
    );
}

// ---------------------------------------------------------------------------
// Round-tripping
// ---------------------------------------------------------------------------

/// Parse and encode must be inverses on the RFC's own bytes. See the module
/// docs on why this is a correctness property rather than a tidiness one.
#[test]
fn every_message_round_trips_byte_for_byte() {
    let client_hello = hex(CLIENT_HELLO);
    let parsed = messages(&client_hello).expect("parses");
    let hello = ClientHello::parse(parsed[0].body).expect("ClientHello parses");
    assert_eq!(
        Message::encode(HandshakeType::ClientHello, &hello.encode()),
        client_hello,
        "ClientHello"
    );

    let server_hello = hex(SERVER_HELLO);
    let parsed = messages(&server_hello).expect("parses");
    let hello = ServerHello::parse(parsed[0].body).expect("ServerHello parses");
    assert_eq!(
        Message::encode(HandshakeType::ServerHello, &hello.encode()),
        server_hello,
        "ServerHello"
    );

    let flight = hex(SERVER_FLIGHT);
    for message in messages(&flight).expect("parses") {
        match message.typ {
            HandshakeType::Certificate => {
                let certificate = CertificateMessage::parse(message.body).expect("parses");
                assert_eq!(
                    Message::encode(HandshakeType::Certificate, &certificate.encode()),
                    message.encoded,
                    "Certificate"
                );
            }
            HandshakeType::CertificateVerify => {
                let verify = CertificateVerify::parse(message.body).expect("parses");
                assert_eq!(
                    Message::encode(HandshakeType::CertificateVerify, &verify.encode()),
                    message.encoded,
                    "CertificateVerify"
                );
            }
            _ => {}
        }
    }
}

/// The `encoded` span must be the bytes that arrived, not a reconstruction —
/// it is what the transcript hashes.
#[test]
fn the_encoded_span_points_into_the_input() {
    let flight = hex(SERVER_FLIGHT);
    let parsed = messages(&flight).expect("parses");

    let range = flight.as_ptr_range();
    let mut total = 0usize;
    for message in &parsed {
        let start = message.encoded.as_ptr();
        assert!(
            start >= range.start && start < range.end,
            "an encoded span escaped its input"
        );
        assert_eq!(message.encoded.len(), message.body.len() + 4);
        assert_eq!(&message.encoded[4..], message.body);
        total += message.encoded.len();
    }
    assert_eq!(total, flight.len(), "the spans tile the input exactly");
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

#[test]
fn the_client_hello_fields_match_the_rfc() {
    let der = hex(CLIENT_HELLO);
    let parsed = messages(&der).expect("parses");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].typ, HandshakeType::ClientHello);

    let hello = ClientHello::parse(parsed[0].body).expect("parses");
    assert_eq!(hello.random.len(), 32);
    assert_eq!(
        hello.random,
        hex("cb34ecb1e78163ba1c38c6dacb196a6dffa21a8d9912ec18a2ef6283024dece7")
    );
    // TLS_AES_128_GCM_SHA256, TLS_CHACHA20_POLY1305_SHA256, TLS_AES_256_GCM_SHA384
    assert_eq!(hello.cipher_suites, vec![0x1301, 0x1303, 0x1302]);
    assert!(hello.session_id.is_empty());

    // supported_versions must offer TLS 1.3 (0x0304).
    let versions = find(&hello.extensions, extension::SUPPORTED_VERSIONS).expect("present");
    assert_eq!(versions, &[0x02, 0x03, 0x04]);

    // SNI for "server".
    let sni = find(&hello.extensions, extension::SERVER_NAME).expect("present");
    assert!(
        sni.ends_with(b"server"),
        "the SNI extension names the server"
    );

    assert!(find(&hello.extensions, extension::KEY_SHARE).is_some());
    assert!(find(&hello.extensions, extension::SIGNATURE_ALGORITHMS).is_some());
    assert!(find(&hello.extensions, extension::SUPPORTED_GROUPS).is_some());
}

#[test]
fn the_server_hello_fields_match_the_rfc() {
    let der = hex(SERVER_HELLO);
    let parsed = messages(&der).expect("parses");
    let hello = ServerHello::parse(parsed[0].body).expect("parses");

    assert_eq!(hello.cipher_suite, 0x1301, "TLS_AES_128_GCM_SHA256");
    assert!(!hello.is_hello_retry_request());

    let versions = find(&hello.extensions, extension::SUPPORTED_VERSIONS).expect("present");
    assert_eq!(versions, &[0x03, 0x04], "the server selected TLS 1.3");

    let key_share = find(&hello.extensions, extension::KEY_SHARE).expect("present");
    // group x25519 (0x001d), then a 32-octet key.
    assert_eq!(&key_share[..2], &[0x00, 0x1d]);
    assert_eq!(u16::from_be_bytes([key_share[2], key_share[3]]), 32);
}

#[test]
fn the_server_flight_parses_into_its_four_messages() {
    let flight = hex(SERVER_FLIGHT);
    let parsed = messages(&flight).expect("parses");

    assert_eq!(
        parsed.iter().map(|m| m.typ).collect::<Vec<_>>(),
        vec![
            HandshakeType::EncryptedExtensions,
            HandshakeType::Certificate,
            HandshakeType::CertificateVerify,
            HandshakeType::Finished,
        ]
    );

    parse_encrypted_extensions(parsed[0].body).expect("EncryptedExtensions parses");

    let certificate = CertificateMessage::parse(parsed[1].body).expect("parses");
    assert!(certificate.context.is_empty());
    assert_eq!(certificate.entries.len(), 1);
    // The entry is a real certificate — hand it to the stage 2a parser.
    let cert = rusty_tls::handrolled::x509::Certificate::parse(certificate.entries[0].certificate)
        .expect("the server's certificate parses with the hand-rolled X.509 parser");
    assert_eq!(cert.version(), rusty_tls::handrolled::x509::Version::V3);

    let verify = CertificateVerify::parse(parsed[2].body).expect("parses");
    assert_eq!(verify.scheme, 0x0804, "rsa_pss_rsae_sha256");
    assert_eq!(verify.signature.len(), 128);

    assert_eq!(parse_finished(parsed[3].body).expect("non-empty").len(), 32);
}

/// The blob a CertificateVerify signature covers, RFC 8446 §4.4.3. The 64
/// octets of padding and the context string are what stop a signature over a
/// bare transcript hash from being reusable in another protocol.
#[test]
fn the_certificate_verify_content_has_the_required_shape() {
    let transcript = hex(TRANSCRIPT_HELLO);
    let content = certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, &transcript);

    assert_eq!(&content[..64], &[0x20u8; 64], "64 octets of 0x20");
    assert_eq!(
        &content[64..64 + SERVER_CERTIFICATE_VERIFY_CONTEXT.len()],
        SERVER_CERTIFICATE_VERIFY_CONTEXT.as_bytes()
    );
    assert_eq!(content[64 + SERVER_CERTIFICATE_VERIFY_CONTEXT.len()], 0x00);
    assert_eq!(
        &content[65 + SERVER_CERTIFICATE_VERIFY_CONTEXT.len()..],
        &transcript[..]
    );

    // The client and server context strings must differ, or a signature made
    // by one would verify as the other's.
    use rusty_tls::handrolled::handshake::CLIENT_CERTIFICATE_VERIFY_CONTEXT;
    assert_ne!(
        certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, &transcript),
        certificate_verify_content(CLIENT_CERTIFICATE_VERIFY_CONTEXT, &transcript)
    );
}

// ---------------------------------------------------------------------------
// Rejection
// ---------------------------------------------------------------------------

/// A message whose `uint24` length overruns the buffer must be refused, not
/// truncated to what is there.
#[test]
fn a_length_that_overruns_the_input_is_refused() {
    let mut der = hex(CLIENT_HELLO);
    der[3] = der[3].wrapping_add(1); // claim one more byte than exists
    assert!(matches!(
        messages(&der),
        Err(HandshakeError::LengthMismatch { .. })
    ));
}

/// Trailing bytes after the last message are refused: a second, partial
/// message hiding behind a complete one is exactly what an attacker appends.
#[test]
fn trailing_bytes_after_the_last_message_are_refused() {
    let mut der = hex(CLIENT_HELLO);
    der.push(0x01);
    assert!(
        messages(&der).is_err(),
        "a stray byte after the final message was ignored"
    );
}

#[test]
fn a_truncated_message_is_refused() {
    let der = hex(CLIENT_HELLO);
    for cut in 1..der.len() {
        assert!(
            messages(&der[..cut]).is_err(),
            "a ClientHello truncated to {cut} bytes parsed"
        );
    }
}

/// RFC 8446 §4.2: one extension of each type per block. A parser that takes
/// the first or the last lets a peer say two things.
#[test]
fn a_duplicated_extension_is_refused() {
    let der = hex(CLIENT_HELLO);
    let parsed = messages(&der).expect("parses");
    let hello = ClientHello::parse(parsed[0].body).expect("parses");

    // Re-encode with the first extension repeated.
    let mut duplicated = hello.clone();
    duplicated.extensions.push(hello.extensions[0]);
    let body = duplicated.encode();

    assert!(matches!(
        ClientHello::parse(&body),
        Err(HandshakeError::DuplicateExtension(_))
    ));
}

/// TLS 1.3 pins `legacy_version` and negotiates through `supported_versions`.
#[test]
fn a_wrong_legacy_version_is_refused() {
    let der = hex(CLIENT_HELLO);
    let parsed = messages(&der).expect("parses");
    let mut body = parsed[0].body.to_vec();
    body[1] = 0x04; // 0x0304 instead of 0x0303

    assert_eq!(
        ClientHello::parse(&body),
        Err(HandshakeError::UnexpectedLegacyVersion(0x0304))
    );
}

/// Compression is where CRIME lived. TLS 1.3 removed it, and a peer offering
/// a method other than null is not speaking this protocol.
#[test]
fn a_non_null_compression_method_is_refused() {
    let der = hex(CLIENT_HELLO);
    let parsed = messages(&der).expect("parses");
    let hello = ClientHello::parse(parsed[0].body).expect("parses");

    // Rebuild by hand with DEFLATE(1) offered instead of null(0).
    let mut body = hello.encode();
    let marker = body
        .windows(2)
        .position(|w| w == [0x01, 0x00])
        .expect("the compression vector is 01 00");
    body[marker + 1] = 0x01;

    assert_eq!(
        ClientHello::parse(&body),
        Err(HandshakeError::UnexpectedCompression)
    );
}

/// A ClientHello with no cipher suites cannot be negotiated with, and an
/// empty vector is the shape a stripped-down downgrade attempt takes.
#[test]
fn an_empty_cipher_suite_list_is_refused() {
    let der = hex(CLIENT_HELLO);
    let parsed = messages(&der).expect("parses");
    let hello = ClientHello::parse(parsed[0].body).expect("parses");

    let mut empty = hello.clone();
    empty.cipher_suites.clear();
    assert_eq!(
        ClientHello::parse(&empty.encode()),
        Err(HandshakeError::Empty("cipher_suites"))
    );
}

/// A ServerHello whose `random` is the HelloRetryRequest sentinel must be
/// recognised as one. TLS 1.3 encodes a retry as a ServerHello rather than a
/// distinct message type, so a client matching only on message type derives
/// keys from a handshake that has not happened.
#[test]
fn a_hello_retry_request_is_recognised() {
    use rusty_tls::handrolled::handshake::HELLO_RETRY_REQUEST_RANDOM;

    let der = hex(SERVER_HELLO);
    let parsed = messages(&der).expect("parses");
    let hello = ServerHello::parse(parsed[0].body).expect("parses");
    assert!(!hello.is_hello_retry_request());

    let mut retry = hello.clone();
    retry.random = &HELLO_RETRY_REQUEST_RANDOM;
    let body = retry.encode();
    let reparsed = ServerHello::parse(&body).expect("parses");
    assert!(
        reparsed.is_hello_retry_request(),
        "a HelloRetryRequest was mistaken for a ServerHello"
    );
}

/// Every truncation of the server's flight must be refused rather than
/// yielding a short list of messages that looks complete.
#[test]
fn a_truncated_flight_is_refused() {
    let flight = hex(SERVER_FLIGHT);
    let full = messages(&flight).expect("parses").len();
    assert_eq!(full, 4);

    let mut refused = 0usize;
    for cut in (1..flight.len()).step_by(7) {
        if messages(&flight[..cut]).is_err() {
            refused += 1;
        } else {
            // A prefix that happens to end exactly on a message boundary is
            // legitimately parseable — but it must never claim all four.
            assert!(
                messages(&flight[..cut]).expect("parses").len() < full,
                "a truncated flight yielded every message"
            );
        }
    }
    assert!(refused > 0, "no truncation was refused");
}

/// An extension whose `extension_data` length reaches past the extensions
/// block that contains it must be refused *by the block's bound*, not by the
/// message's.
///
/// This is the nesting property the whole sub-reader design exists for. The
/// message here is entirely well-formed at its own level — the four-octet
/// header is honest, the block length is honest — and the lie is one level
/// down, where an extension claims 255 octets inside a block with none left.
/// A parser that read extensions from the message's cursor rather than the
/// block's would happily consume bytes belonging to whatever follows.
#[test]
fn an_extension_cannot_reach_past_the_block_that_contains_it() {
    // A four-octet extensions block holding one extension: type 43, whose
    // data claims 255 octets that the block does not have.
    let body = hex("0004 002b 00ff");

    assert_eq!(
        parse_encrypted_extensions(&body),
        Err(HandshakeError::Wire(WireError::LengthOverrun {
            declared: 255,
            available: 0,
        })),
        "an extension overrunning its block was not named as an overrun"
    );

    // And the same lie is still caught when the message has plenty of bytes
    // after the block for the over-long extension to have eaten.
    let mut padded = body.clone();
    padded.extend_from_slice(&[0xaa; 255]);
    assert!(matches!(
        parse_encrypted_extensions(&padded),
        Err(HandshakeError::Wire(WireError::LengthOverrun { .. }))
    ));
}

/// A Certificate message whose entry length overruns the certificate list is
/// refused for the same reason, one level deeper.
#[test]
fn a_certificate_entry_cannot_reach_past_the_certificate_list() {
    let flight = hex(SERVER_FLIGHT);
    let parsed = messages(&flight).expect("parses");
    let certificate = parsed
        .iter()
        .find(|m| m.typ == HandshakeType::Certificate)
        .expect("the flight carries a Certificate");

    // body: context (1 octet, empty) || list length (3) || entry length (3).
    // The list holds the entry's 3-octet length, the certificate, and the
    // entry's 2-octet extensions prefix — so the certificate has exactly two
    // spare octets inside the list, and claiming three more overruns it.
    let mut body = certificate.body.to_vec();
    let entry_length = u32::from_be_bytes([0, body[4], body[5], body[6]]) as usize;
    body[6] = body[6].wrapping_add(3);

    assert_eq!(
        CertificateMessage::parse(&body),
        Err(HandshakeError::Wire(WireError::LengthOverrun {
            declared: entry_length + 3,
            available: entry_length + 2,
        })),
        "an entry overrunning the certificate list was not refused as an overrun"
    );
}

// ---------------------------------------------------------------------------
// The two-phase ClientHello — rusty_tls#43
// ---------------------------------------------------------------------------

/// A ClientHello carrying a `pre_shared_key` offer with `binder_lens`-sized
/// placeholders, built on the RFC's own hello so the surrounding message is
/// real rather than a fixture shaped to suit the test.
fn hello_with_offer(binder_lens: &[usize], trailing_extension: bool) -> Vec<u8> {
    let encoded = hex(CLIENT_HELLO);
    let message = messages(&encoded).expect("the RFC's hello parses")[0];
    let mut hello = ClientHello::parse(message.body).expect("a ClientHello");

    let identities = [PskIdentity {
        identity: b"an opaque ticket",
        obfuscated_ticket_age: 0x0102_0304,
    }];
    let offer = pre_shared_key_placeholder(&identities, binder_lens);
    hello.extensions.push(Extension {
        typ: extension::PRE_SHARED_KEY,
        data: &offer,
    });
    // An unregistered type, so this is a trailing extension and not a
    // duplicate of one the RFC's hello already carries — 0xff01 is
    // `renegotiation_info`, which that hello does carry.
    let trailing = [0x03u8, 0x04];
    if trailing_extension {
        hello.extensions.push(Extension {
            typ: 0xff02,
            data: &trailing,
        });
    }
    // Encoded here rather than returned as a `ClientHello`, because the struct
    // borrows `offer` and `trailing`, which do not outlive this function.
    Message::encode(HandshakeType::ClientHello, &hello.encode())
}

/// The bytes a binder covers are a **literal prefix** of the message that is
/// sent.
///
/// This is the property the two-phase encoding exists for, and the one that
/// cannot be checked by looking at either half alone. If the truncated bytes
/// were a separate serialisation — "encode everything up to the binders" —
/// then every length field in it would have to be independently correct, and a
/// disagreement between the two encoders would produce a binder over bytes
/// nobody ever sent. Building with placeholders and splicing makes the two
/// the same bytes by construction; this asserts that construction held.
#[test]
fn the_truncated_hello_is_a_prefix_of_the_finished_one() {
    let encoded = hex(CLIENT_HELLO);
    let message = messages(&encoded).expect("the RFC's hello parses")[0];
    let mut hello = ClientHello::parse(message.body).expect("a ClientHello");
    let identities = [PskIdentity {
        identity: b"an opaque ticket",
        obfuscated_ticket_age: 0x0102_0304,
    }];
    let offer = pre_shared_key_placeholder(&identities, &[32]);
    hello.extensions.push(Extension {
        typ: extension::PRE_SHARED_KEY,
        data: &offer,
    });

    let placeheld = BinderHello::new(&hello, &[32]).expect("a hello with placeholders");
    let truncated = placeheld.truncated().to_vec();
    let binder = vec![0xabu8; 32];
    let finished = placeheld
        .finish(std::slice::from_ref(&binder))
        .expect("the splice");

    assert_eq!(
        &finished[..truncated.len()],
        truncated.as_slice(),
        "the binder covers bytes that are not a prefix of the message sent"
    );
    assert_eq!(
        finished.len(),
        // uint16 for the binder list, then a uint8 length and the binder.
        truncated.len() + 2 + 1 + 32,
        "the binder block is not the whole of what truncation removed"
    );
    assert_eq!(
        &finished[finished.len() - 32..],
        binder.as_slice(),
        "the real binder did not land where the placeholder was"
    );

    // And the finished message is still a well-formed ClientHello whose offer
    // reads back as what went in — a splice that corrupted a length field
    // would leave the arithmetic above intact and this parse broken.
    let message = messages(&finished).expect("the spliced hello parses")[0];
    let hello = ClientHello::parse(message.body).expect("a ClientHello");
    let data = find(&hello.extensions, extension::PRE_SHARED_KEY).expect("the offer");
    let offer = PresharedKeyOffer::parse(data).expect("the offer parses");
    assert_eq!(offer.identities[0].identity, b"an opaque ticket");
    assert_eq!(offer.identities[0].obfuscated_ticket_age, 0x0102_0304);
    assert_eq!(offer.binders, vec![binder.as_slice()]);
    assert_eq!(
        offer.truncated(&finished).expect("the truncation point"),
        truncated.as_slice(),
        "the receiving side truncates somewhere else than the sending side"
    );
}

/// An extension after the offer is refused, on both sides.
///
/// The whole reason `pre_shared_key` must be last is that a binder covers only
/// what precedes it. A hello with anything after the offer has bytes the
/// binder does not prove, and both the encoder and the parser have to say so —
/// the encoder because it would otherwise splice into the middle of a message,
/// and the parser because a peer is not obliged to be well-behaved.
#[test]
fn an_extension_after_the_offer_is_refused() {
    let encoded = hex(CLIENT_HELLO);
    let message = messages(&encoded).expect("the RFC's hello parses")[0];
    let mut hello = ClientHello::parse(message.body).expect("a ClientHello");
    let identities = [PskIdentity {
        identity: b"an opaque ticket",
        obfuscated_ticket_age: 0,
    }];
    let offer = pre_shared_key_placeholder(&identities, &[32]);
    hello.extensions.push(Extension {
        typ: extension::PRE_SHARED_KEY,
        data: &offer,
    });
    let trailing = [0x03u8, 0x04];
    hello.extensions.push(Extension {
        typ: 0xff02,
        data: &trailing,
    });

    assert!(
        matches!(
            BinderHello::new(&hello, &[32]),
            Err(HandshakeError::PskOffer(_))
        ),
        "a hello with an extension after the offer was encoded anyway"
    );

    // The parsing side, on a message that really was built that way.
    let sent = hello_with_offer(&[32], true);
    let message = messages(&sent).expect("it parses as a message")[0];
    let parsed = ClientHello::parse(message.body).expect("a ClientHello");
    let data = find(&parsed.extensions, extension::PRE_SHARED_KEY).expect("the offer");
    let offer = PresharedKeyOffer::parse(data).expect("the offer parses");
    assert!(
        matches!(
            offer.truncated(message.encoded),
            Err(HandshakeError::PskOffer(_))
        ),
        "a received hello with an extension after the offer was truncated anyway"
    );
}

/// Binders that do not match the placeholders they replace are refused.
///
/// A binder of the wrong length cannot be spliced in without moving every byte
/// after it, and there are no bytes after it — so the failure would be a
/// silent truncation of the binder rather than a wrong hello. Refusing is the
/// only outcome that says what happened.
#[test]
fn a_binder_that_does_not_fit_its_placeholder_is_refused() {
    let encoded = hex(CLIENT_HELLO);
    let message = messages(&encoded).expect("the RFC's hello parses")[0];
    let mut hello = ClientHello::parse(message.body).expect("a ClientHello");
    let identities = [PskIdentity {
        identity: b"an opaque ticket",
        obfuscated_ticket_age: 0,
    }];
    let offer = pre_shared_key_placeholder(&identities, &[48]);
    hello.extensions.push(Extension {
        typ: extension::PRE_SHARED_KEY,
        data: &offer,
    });

    let placeheld = BinderHello::new(&hello, &[48]).expect("a hello with placeholders");
    assert!(
        matches!(
            placeheld.clone().finish(&[vec![0u8; 32]]),
            Err(HandshakeError::PskOffer(_))
        ),
        "a 32-octet binder was spliced into a 48-octet placeholder"
    );
    assert!(
        matches!(
            placeheld.clone().finish(&[vec![0u8; 48], vec![0u8; 48]]),
            Err(HandshakeError::PskOffer(_))
        ),
        "two binders were spliced into one placeholder"
    );
    assert!(placeheld.finish(&[vec![0u8; 48]]).is_ok());
}

/// A `pre_shared_key` offer with more binders than identities is refused.
///
/// RFC 8446 §4.2.11 requires the lists to be the same length. Index-matching
/// through a mismatch would check one identity's binder and use another's key,
/// which is a check that looks like it happened.
#[test]
fn an_offer_with_mismatched_lists_is_refused() {
    let mut writer = rusty_tls::handrolled::wire::Writer::new();
    writer.vector_u16(|w| {
        w.vector_u16(|w| w.bytes(b"one ticket"));
        w.u32(0);
    });
    writer.vector_u16(|w| {
        w.vector_u8(|w| w.bytes(&[0u8; 32]));
        w.vector_u8(|w| w.bytes(&[0u8; 32]));
    });

    assert!(
        matches!(
            PresharedKeyOffer::parse(&writer.into_vec()),
            Err(HandshakeError::PskOffer(_))
        ),
        "an offer with two binders for one identity was accepted"
    );
}
