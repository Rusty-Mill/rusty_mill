//! The TLS 1.2 handshake messages: `handshake12`.
//!
//! Every message must parse and encode as inverses on the wire bytes, because
//! `Finished` is a MAC over the encoded messages and the signature on a
//! `ServerKeyExchange` covers its encoded parameters: a parser that normalised
//! while re-encoding would hash or sign something nobody sent.
//!
//! The second property is strictness, and it is checked exhaustively rather
//! than by example: **every strict prefix** of a valid message must be refused
//! (a truncated message is not a shorter message), and **every trailing octet**
//! must be refused (a second message hiding behind the first).

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::handshake::{Extension, HandshakeError};
use rusty_tls::handrolled::handshake12::{
    encode_client_key_exchange, parse_client_key_exchange, parse_finished, parse_server_hello_done,
    signed_content, Certificate12, CertificateRequest12, ServerHello12, ServerKeyExchange, TLS12,
    VERIFY_DATA_LEN,
};

/// Parse every strict prefix and every one-octet extension of `valid`, and
/// require all of them to fail. `parses` says whether a byte string parses.
fn strict(name: &str, valid: &[u8], parses: impl Fn(&[u8]) -> bool) {
    assert!(parses(valid), "{name}: the valid message must parse");
    for cut in 0..valid.len() {
        assert!(
            !parses(&valid[..cut]),
            "{name}: a {cut}-octet prefix of {} parsed",
            valid.len()
        );
    }
    for extra in [0u8, 1, 0xff] {
        let mut longer = valid.to_vec();
        longer.push(extra);
        assert!(
            !parses(&longer),
            "{name}: trailing octet {extra:#x} was tolerated"
        );
    }
}

// ------------------------------------------------------------- ServerHello

fn hello_body(extensions: Option<&[Extension<'_>]>) -> Vec<u8> {
    let hello = ServerHello12 {
        version: TLS12,
        random: &[7u8; 32],
        session_id: &[1, 2, 3],
        cipher_suite: 0xc02b,
        compression: 0,
        extensions: extensions
            .map(<[Extension<'_>]>::to_vec)
            .unwrap_or_default(),
    };
    hello.encode()
}

#[test]
fn a_server_hello_round_trips_with_and_without_extensions() {
    let extensions = [
        Extension { typ: 23, data: &[] },
        Extension {
            typ: 0xff01,
            data: &[0],
        },
        Extension {
            typ: 11,
            data: &[1, 0],
        },
    ];
    for list in [Some(&extensions[..]), None] {
        let body = hello_body(list);
        let parsed = ServerHello12::parse(&body).expect("parses");
        assert_eq!(parsed.version, TLS12);
        assert_eq!(parsed.random, &[7u8; 32]);
        assert_eq!(parsed.session_id, &[1, 2, 3]);
        assert_eq!(parsed.cipher_suite, 0xc02b);
        assert_eq!(parsed.extensions.len(), list.map_or(0, <[_]>::len));
        assert_eq!(parsed.encode(), body, "encode is parse's inverse");
    }
}

#[test]
fn a_server_hello_is_strict_about_truncation_and_trailing_data() {
    // Truncating a hello that has extensions: every prefix is refused except the
    // one that ends exactly where the extensions block would begin, which is a
    // complete, valid hello with no extensions (how servers that predate them
    // answer). That one cut is accepted by design, and only that one.
    let with = hello_body(Some(&[Extension { typ: 23, data: &[] }]));
    let without = hello_body(None);
    for cut in 0..with.len() {
        let parses = ServerHello12::parse(&with[..cut]).is_ok();
        assert_eq!(
            parses,
            cut == without.len(),
            "a {cut}-octet prefix of {}",
            with.len()
        );
    }
    // And the extension-less form is itself strict: nothing may follow it
    // except a whole, well-formed extensions block.
    for cut in 0..without.len() {
        assert!(
            ServerHello12::parse(&without[..cut]).is_err(),
            "a {cut}-octet prefix parsed"
        );
    }
    let mut stray = without.clone();
    stray.push(0);
    assert!(
        ServerHello12::parse(&stray).is_err(),
        "one stray octet is neither a block nor nothing"
    );
    for extra in [0u8, 1, 0xff] {
        let mut longer = with.clone();
        longer.push(extra);
        assert!(
            ServerHello12::parse(&longer).is_err(),
            "trailing octet {extra:#x} was tolerated"
        );
    }
}

#[test]
fn a_server_hello_with_a_long_session_id_or_duplicate_extension_is_refused() {
    let mut body = hello_body(None);
    // Rewrite the session id length (offset 34) to claim 33 octets and add them.
    body.splice(
        34..35 + 3,
        [33u8].into_iter().chain(std::iter::repeat_n(0, 33)),
    );
    assert_eq!(
        ServerHello12::parse(&body).err(),
        Some(HandshakeError::Malformed("session_id is over 32 octets"))
    );

    let duplicated = [
        Extension { typ: 23, data: &[] },
        Extension { typ: 23, data: &[] },
    ];
    assert_eq!(
        ServerHello12::parse(&hello_body(Some(&duplicated))).err(),
        Some(HandshakeError::DuplicateExtension(23))
    );
}

#[test]
fn the_server_hello_version_is_reported_not_judged() {
    // The parser reports whatever version arrived; deciding it is wrong is the
    // client's job, so it can name it.
    let mut body = hello_body(None);
    body[0..2].copy_from_slice(&0x0301u16.to_be_bytes());
    assert_eq!(ServerHello12::parse(&body).expect("parses").version, 0x0301);
}

// ------------------------------------------------------------- Certificate

#[test]
fn a_certificate_round_trips() {
    let certs: [&[u8]; 3] = [&[1, 2, 3], &[4; 300], &[5]];
    let body = Certificate12::encode(&certs);
    let parsed = Certificate12::parse(&body).expect("parses");
    assert_eq!(parsed.certificates, certs);
    // An empty list is valid on the wire: it is how a client says it has none.
    assert!(Certificate12::parse(&Certificate12::encode(&[]))
        .expect("parses")
        .certificates
        .is_empty());
}

#[test]
fn a_certificate_is_strict_and_refuses_empty_entries() {
    let body = Certificate12::encode(&[&[1, 2, 3], &[4, 5]]);
    strict("Certificate", &body, |b| Certificate12::parse(b).is_ok());

    // A zero-length certificate in the chain has no meaning and is refused.
    let with_empty = Certificate12::encode(&[&[1, 2, 3], &[]]);
    assert_eq!(
        Certificate12::parse(&with_empty).err(),
        Some(HandshakeError::Malformed(
            "a certificate in the chain is empty"
        ))
    );
    // A list length that disagrees with its contents.
    assert!(Certificate12::parse(&[0, 0, 5, 0, 0, 1, 9]).is_err());
}

// -------------------------------------------------------- ServerKeyExchange

fn ske_body() -> (Vec<u8>, Vec<u8>) {
    let params = ServerKeyExchange::encode_params(0x0017, &[4; 65]);
    (ServerKeyExchange::encode(&params, 0x0403, &[9; 70]), params)
}

#[test]
fn a_server_key_exchange_round_trips_and_exposes_the_signed_params() {
    let (body, params) = ske_body();
    let parsed = ServerKeyExchange::parse(&body).expect("parses");
    assert_eq!(parsed.named_curve, 0x0017);
    assert_eq!(parsed.public, &[4; 65]);
    assert_eq!(parsed.scheme, 0x0403);
    assert_eq!(parsed.signature, &[9; 70]);
    // The signed bytes are a borrow of exactly what was on the wire.
    assert_eq!(parsed.params, params);
    assert!(
        std::ptr::eq(parsed.params.as_ptr(), body.as_ptr()),
        "params must be a borrow, not a copy"
    );
    assert_eq!(params[0], 3, "named_curve(3)");
}

#[test]
fn a_server_key_exchange_is_strict() {
    let (body, _) = ske_body();
    strict("ServerKeyExchange", &body, |b| {
        ServerKeyExchange::parse(b).is_ok()
    });
}

#[test]
fn only_named_curves_are_parsed() {
    let (mut body, _) = ske_body();
    for curve_type in [0u8, 1, 2, 4, 255] {
        body[0] = curve_type;
        assert_eq!(
            ServerKeyExchange::parse(&body).err(),
            Some(HandshakeError::UnexpectedCurveType(curve_type))
        );
    }
}

#[test]
fn empty_keys_and_signatures_are_refused() {
    let no_key =
        ServerKeyExchange::encode(&ServerKeyExchange::encode_params(0x0017, &[]), 0x0403, &[1]);
    assert!(matches!(
        ServerKeyExchange::parse(&no_key),
        Err(HandshakeError::Empty(_))
    ));
    let no_sig =
        ServerKeyExchange::encode(&ServerKeyExchange::encode_params(0x0017, &[4]), 0x0403, &[]);
    assert!(matches!(
        ServerKeyExchange::parse(&no_sig),
        Err(HandshakeError::Empty(_))
    ));
}

#[test]
fn the_signed_content_is_both_randoms_then_the_params() {
    let (body, params) = ske_body();
    let parsed = ServerKeyExchange::parse(&body).expect("parses");
    let (client, server) = ([1u8; 32], [2u8; 32]);

    let signed = parsed.signed_content(&client, &server);
    assert_eq!(signed.len(), 32 + 32 + params.len());
    assert_eq!(&signed[..32], &client);
    assert_eq!(&signed[32..64], &server);
    assert_eq!(&signed[64..], &params[..]);
    assert_eq!(signed, signed_content(&client, &server, &params));

    // The order matters: swapping the randoms is a different message.
    assert_ne!(signed, parsed.signed_content(&server, &client));
}

// ----------------------------------------------------------- the small ones

#[test]
fn a_client_key_exchange_round_trips_and_is_strict() {
    let body = encode_client_key_exchange(&[4; 65]);
    assert_eq!(parse_client_key_exchange(&body).expect("parses"), &[4; 65]);
    strict("ClientKeyExchange", &body, |b| {
        parse_client_key_exchange(b).is_ok()
    });
    assert!(matches!(
        parse_client_key_exchange(&[0]),
        Err(HandshakeError::Empty(_))
    ));
}

#[test]
fn a_certificate_request_round_trips_and_is_strict() {
    let request = CertificateRequest12 {
        certificate_types: &[64, 1],
        signature_algorithms: vec![0x0403, 0x0804],
        authorities: &[0, 1, 2],
    };
    let body = request.encode();
    assert_eq!(CertificateRequest12::parse(&body).expect("parses"), request);
    strict("CertificateRequest", &body, |b| {
        CertificateRequest12::parse(b).is_ok()
    });

    // No certificate types, and no signature algorithms, are both refused.
    let none = CertificateRequest12 {
        certificate_types: &[],
        ..request.clone()
    };
    assert!(matches!(
        CertificateRequest12::parse(&none.encode()),
        Err(HandshakeError::Empty(_))
    ));
    let no_algorithms = CertificateRequest12 {
        signature_algorithms: vec![],
        ..request
    };
    assert_eq!(
        CertificateRequest12::parse(&no_algorithms.encode()).err(),
        Some(HandshakeError::MissingSignatureAlgorithms)
    );
}

#[test]
fn server_hello_done_and_finished_have_fixed_shapes() {
    assert!(parse_server_hello_done(&[]).is_ok());
    assert!(parse_server_hello_done(&[0]).is_err());

    assert_eq!(VERIFY_DATA_LEN, 12);
    assert!(parse_finished(&[0; 12]).is_ok());
    for len in [0usize, 1, 11, 13, 32, 48] {
        assert!(parse_finished(&vec![0; len]).is_err(), "{len} octets");
    }
}
