//! The ALPN offer is validated before it reaches rustls: an empty protocol
//! name trips a debug assertion there, and a name over 255 bytes would have
//! its length truncated to one byte on the wire. The constructors return
//! `Result`, so both are configuration errors, not panics. Covers the
//! ALPN-only and the identity-plus-ALPN constructors, sync and async.

use std::io::Cursor;

use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};
use rustls::pki_types::CertificateDer;

use rusty_tls::{Error, TlsStream, TrustPolicy};

struct Fixture {
    policy: TrustPolicy,
    cert: Vec<u8>,
    key: Vec<u8>,
}

fn fixture() -> Fixture {
    let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "alpn validation test");
    params.distinguished_name = dn;
    let key_pair = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    let root: CertificateDer<'static> = cert.der().clone();
    Fixture {
        policy: TrustPolicy::PinnedAnchors(vec![root]),
        cert: cert.der().to_vec(),
        key: key_pair.serialize_der(),
    }
}

/// An offer of `n` entries of `len` bytes each, then one of `last` bytes.
fn offer(n: usize, len: usize, last: usize) -> Vec<Vec<u8>> {
    let mut v = vec![vec![b'a'; len]; n];
    v.push(vec![b'b'; last]);
    v
}

/// Total ProtocolNameList bytes: each entry costs its length plus one.
fn list_len(offer: &[Vec<u8>]) -> usize {
    offer.iter().map(|p| p.len() + 1).sum()
}

fn alpn_only(f: &Fixture, offer: Vec<Vec<u8>>) -> Result<(), Error> {
    TlsStream::new_with_alpn(Cursor::new(Vec::new()), "localhost", &f.policy, offer).map(drop)
}

fn identity_and_alpn(f: &Fixture, offer: Vec<Vec<u8>>) -> Result<(), Error> {
    TlsStream::new_with_client_identity_and_alpn(
        Cursor::new(Vec::new()),
        "localhost",
        &f.policy,
        vec![f.cert.clone()],
        f.key.clone(),
        offer,
    )
    .map(drop)
}

type Constructor = fn(&Fixture, Vec<Vec<u8>>) -> Result<(), Error>;
const BOTH: [(&str, Constructor); 2] = [
    ("new_with_alpn", alpn_only),
    ("new_with_client_identity_and_alpn", identity_and_alpn),
];

#[test]
fn an_empty_protocol_name_is_an_error_not_a_panic() {
    let f = fixture();
    for (name, build) in BOTH {
        let r = build(&f, vec![b"h2".to_vec(), Vec::new()]);
        assert!(
            matches!(r, Err(Error::InvalidAlpnProtocols(_))),
            "{name}: {r:?}"
        );
    }
}

#[test]
fn a_protocol_name_over_255_bytes_is_an_error() {
    let f = fixture();
    for (name, build) in BOTH {
        let r = build(&f, vec![vec![b'x'; 256]]);
        assert!(
            matches!(r, Err(Error::InvalidAlpnProtocols(_))),
            "{name}: {r:?}"
        );
    }
}

#[test]
fn a_protocol_name_of_exactly_255_bytes_is_accepted() {
    let f = fixture();
    for (name, build) in BOTH {
        assert!(build(&f, vec![vec![b'x'; 255]]).is_ok(), "{name}");
    }
}

#[test]
fn an_empty_offer_and_ordinary_names_are_accepted() {
    let f = fixture();
    for (name, build) in BOTH {
        assert!(build(&f, Vec::new()).is_ok(), "{name}: no ALPN at all");
        assert!(
            build(&f, vec![b"h2".to_vec(), b"http/1.1".to_vec()]).is_ok(),
            "{name}"
        );
    }
}

#[test]
fn an_offer_too_large_to_encode_is_an_error() {
    // rustls length-prefixes all of a ClientHello's extensions with 16 bits and
    // panics (in debug builds) past that, so the offer is capped well below it,
    // at 32 KiB, to leave room for the other extensions.
    let f = fixture();
    let at_limit = offer(128, 254, 127); // 128 * 255 + 128 = 32768
    assert_eq!(list_len(&at_limit), 32_768);
    let over = offer(128, 254, 128);
    assert_eq!(list_len(&over), 32_769);
    for (name, build) in BOTH {
        assert!(build(&f, at_limit.clone()).is_ok(), "{name}: at the limit");
        let r = build(&f, over.clone());
        assert!(
            matches!(r, Err(Error::InvalidAlpnProtocols(_))),
            "{name}: {r:?}"
        );
    }
    // Far past what 16 bits could hold.
    for (name, build) in BOTH {
        let r = build(&f, offer(300, 254, 10));
        assert!(
            matches!(r, Err(Error::InvalidAlpnProtocols(_))),
            "{name}: {r:?}"
        );
    }
}

#[cfg(feature = "rusty-tokio")]
mod async_ctors {
    use super::*;
    use rusty_tls::AsyncTlsStream;

    fn alpn_only(f: &Fixture, offer: Vec<Vec<u8>>) -> Result<(), Error> {
        AsyncTlsStream::new_with_alpn(rusty_tokio::io::duplex(64).0, "localhost", &f.policy, offer)
            .map(drop)
    }

    fn identity_and_alpn(f: &Fixture, offer: Vec<Vec<u8>>) -> Result<(), Error> {
        AsyncTlsStream::new_with_client_identity_and_alpn(
            rusty_tokio::io::duplex(64).0,
            "localhost",
            &f.policy,
            vec![f.cert.clone()],
            f.key.clone(),
            offer,
        )
        .map(drop)
    }

    const BOTH: [(&str, Constructor); 2] = [
        ("async new_with_alpn", alpn_only),
        ("async new_with_client_identity_and_alpn", identity_and_alpn),
    ];

    #[test]
    fn invalid_offers_are_errors_not_panics() {
        let f = fixture();
        for (name, build) in BOTH {
            let empty = build(&f, vec![Vec::new()]);
            assert!(
                matches!(empty, Err(Error::InvalidAlpnProtocols(_))),
                "{name}: {empty:?}"
            );
            let long = build(&f, vec![vec![b'x'; 256]]);
            assert!(
                matches!(long, Err(Error::InvalidAlpnProtocols(_))),
                "{name}: {long:?}"
            );
            let huge = build(&f, offer(128, 254, 128));
            assert!(
                matches!(huge, Err(Error::InvalidAlpnProtocols(_))),
                "{name}: {huge:?}"
            );
        }
    }

    #[test]
    fn valid_offers_are_accepted() {
        let f = fixture();
        for (name, build) in BOTH {
            assert!(build(&f, Vec::new()).is_ok(), "{name}");
            assert!(build(&f, vec![vec![b'x'; 255]]).is_ok(), "{name}");
            assert!(
                build(&f, offer(128, 254, 127)).is_ok(),
                "{name}: at the limit"
            );
        }
    }
}
