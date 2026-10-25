//! PKCS#8 parsing against `ring`: for generated keys, hand-built edge cases and every
//! single-byte mutation, truncation and extension of them, both accept or both reject,
//! and when both accept they derive the same public key.

use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, Ed25519KeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING,
    ECDSA_P384_SHA384_ASN1_SIGNING,
};
use rusty_pk::ecdsa::CurveId;
use rusty_pk::pkcs8::{self, KeyRejected};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Ecdsa(CurveId),
    Ed25519,
}

/// The public key each side derives, or `None` if it rejects the document.
fn both(kind: Kind, der: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    match kind {
        Kind::Ecdsa(curve) => {
            let alg = match curve {
                CurveId::P256 => &ECDSA_P256_SHA256_ASN1_SIGNING,
                CurveId::P384 => &ECDSA_P384_SHA384_ASN1_SIGNING,
            };
            let ours = pkcs8::ecdsa(curve, der)
                .ok()
                .map(|k| k.public_key().unwrap().as_bytes().to_vec());
            let theirs = EcdsaKeyPair::from_pkcs8(alg, der, &SystemRandom::new())
                .ok()
                .map(|k| k.public_key().as_ref().to_vec());
            (ours, theirs)
        }
        Kind::Ed25519 => {
            let ours = pkcs8::ed25519(der).ok().map(|k| k.public_key().to_vec());
            let theirs = Ed25519KeyPair::from_pkcs8_maybe_unchecked(der)
                .ok()
                .map(|k| k.public_key().as_ref().to_vec());
            (ours, theirs)
        }
    }
}

fn assert_agree(kind: Kind, der: &[u8], what: &str) {
    let (ours, theirs) = both(kind, der);
    assert_eq!(ours, theirs, "{kind:?} {what}: {}", hex(der));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generated(kind: Kind) -> Vec<u8> {
    let rng = SystemRandom::new();
    match kind {
        Kind::Ecdsa(CurveId::P256) => {
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
                .unwrap()
                .as_ref()
                .to_vec()
        }
        Kind::Ecdsa(CurveId::P384) => {
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P384_SHA384_ASN1_SIGNING, &rng)
                .unwrap()
                .as_ref()
                .to_vec()
        }
        Kind::Ed25519 => Ed25519KeyPair::generate_pkcs8(&rng)
            .unwrap()
            .as_ref()
            .to_vec(),
    }
}

/// A v1 Ed25519 document (RFC 8410 section 7) for `seed`.
fn ed25519_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(seed);
    der
}

/// A DER TLV with a short- or two-byte long-form length.
fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match body.len() {
        n @ 0..=0x7f => out.push(n as u8),
        n @ 0x80..=0xff => out.extend_from_slice(&[0x81, n as u8]),
        n => out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(body);
    out
}

/// A v2 Ed25519 document for `seed` with `public` under the given `[1]` encoding.
fn ed25519_v2(seed: &[u8; 32], public: &[u8], legacy: bool) -> Vec<u8> {
    let mut bits = vec![0u8];
    bits.extend_from_slice(public);
    let public_field = if legacy {
        tlv(0xa1, &tlv(0x03, &bits))
    } else {
        tlv(0x81, &bits)
    };
    let body = [
        tlv(0x02, &[1]),
        tlv(0x30, &tlv(0x06, &[0x2b, 0x65, 0x70])),
        tlv(0x04, &tlv(0x04, seed)),
        public_field,
    ]
    .concat();
    tlv(0x30, &body)
}

/// An EC PKCS#8 v1 document: `scalar`, optional `[0]` curve, optional `[1]` public key.
fn ec_document(
    curve_oid: &[u8],
    scalar: &[u8],
    params: Option<&[u8]>,
    public: Option<&[u8]>,
) -> Vec<u8> {
    let mut ec = [tlv(0x02, &[1]), tlv(0x04, scalar)].concat();
    if let Some(oid) = params {
        ec.extend_from_slice(&tlv(0xa0, oid));
    }
    if let Some(point) = public {
        let mut bits = vec![0u8];
        bits.extend_from_slice(point);
        ec.extend_from_slice(&tlv(0xa1, &tlv(0x03, &bits)));
    }
    let algorithm = [
        vec![0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01],
        curve_oid.to_vec(),
    ]
    .concat();
    let body = [
        tlv(0x02, &[0]),
        tlv(0x30, &algorithm),
        tlv(0x04, &tlv(0x30, &ec)),
    ]
    .concat();
    tlv(0x30, &body)
}

const P256_OID: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const P384_OID: &[u8] = &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22];

/// Every single-byte change (three masks), every truncation and one extension.
fn assert_mutations_agree(kind: Kind, base: &[u8]) {
    for i in 0..base.len() {
        for mask in [0x01u8, 0x80, 0xff] {
            let mut m = base.to_vec();
            m[i] ^= mask;
            assert_agree(kind, &m, &format!("byte {i} ^ {mask:#04x}"));
        }
    }
    for len in 0..base.len() {
        assert_agree(kind, &base[..len], &format!("truncated to {len}"));
    }
    let mut longer = base.to_vec();
    longer.push(0);
    assert_agree(kind, &longer, "one byte appended");
}

#[test]
fn generated_keys_match_ring() {
    for kind in [
        Kind::Ecdsa(CurveId::P256),
        Kind::Ecdsa(CurveId::P384),
        Kind::Ed25519,
    ] {
        for _ in 0..20 {
            let der = generated(kind);
            let (ours, theirs) = both(kind, &der);
            assert!(ours.is_some(), "{kind:?} rejected a ring-generated key");
            assert_eq!(ours, theirs, "{kind:?}");
        }
    }
}

#[test]
fn every_single_byte_mutation_agrees_with_ring() {
    for kind in [
        Kind::Ecdsa(CurveId::P256),
        Kind::Ecdsa(CurveId::P384),
        Kind::Ed25519,
    ] {
        assert_mutations_agree(kind, &generated(kind));
    }
    assert_mutations_agree(Kind::Ed25519, &ed25519_v1(&[7; 32]));
}

#[test]
fn ed25519_v1_and_both_v2_public_key_encodings_agree() {
    let seed = [9u8; 32];
    let public = Ed25519KeyPair::from_seed_unchecked(&seed)
        .unwrap()
        .public_key()
        .as_ref()
        .to_vec();
    let mut wrong = public.clone();
    wrong[0] ^= 1;
    for (der, what) in [
        (ed25519_v1(&seed), "v1"),
        (ed25519_v2(&seed, &public, false), "v2 primitive [1]"),
        (ed25519_v2(&seed, &public, true), "v2 constructed [1]"),
        (ed25519_v2(&seed, &wrong, false), "v2 wrong public key"),
        (
            ed25519_v2(&seed, &public[..31], false),
            "v2 short public key",
        ),
    ] {
        assert_agree(Kind::Ed25519, &der, what);
    }
    assert!(pkcs8::ed25519(&ed25519_v2(&seed, &public, true)).is_ok());
    assert_eq!(
        pkcs8::ed25519(&ed25519_v2(&seed, &wrong, false)).unwrap_err(),
        KeyRejected::PublicKeyMismatch
    );
}

#[test]
fn ec_parameters_public_key_and_range_edge_cases_agree() {
    let der = generated(Kind::Ecdsa(CurveId::P256));
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &der, &SystemRandom::new())
        .unwrap();
    let public = key.public_key().as_ref().to_vec();
    // The scalar sits in ring's own encoding; recover it from the parsed document.
    let ours = pkcs8::ecdsa(CurveId::P256, &der).unwrap();
    assert_eq!(ours.public_key().unwrap().as_bytes(), &public[..]);
    let scalar = scalar_of(&der);
    let n: Vec<u8> = (0..32)
        .map(|i| {
            u8::from_str_radix(
                &"ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551"
                    [2 * i..2 * i + 2],
                16,
            )
            .unwrap()
        })
        .collect();
    let mut other = public.clone();
    other[40] ^= 1;
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (
            ec_document(P256_OID, &scalar, Some(P256_OID), Some(&public)),
            "with [0] parameters",
        ),
        (
            ec_document(P256_OID, &scalar, None, Some(&public)),
            "without [0] parameters",
        ),
        (
            ec_document(P256_OID, &scalar, Some(P384_OID), Some(&public)),
            "[0] names another curve",
        ),
        (
            ec_document(P256_OID, &scalar, None, None),
            "no [1] public key",
        ),
        (
            ec_document(P256_OID, &scalar, None, Some(&other)),
            "wrong public key",
        ),
        (
            ec_document(P256_OID, &scalar[1..], None, Some(&public)),
            "short scalar",
        ),
        (
            ec_document(P256_OID, &[0; 32], None, Some(&public)),
            "zero scalar",
        ),
        (
            ec_document(P256_OID, &n, None, Some(&public)),
            "scalar equal to n",
        ),
        (
            ec_document(P384_OID, &scalar, None, Some(&public)),
            "P-384 algorithm",
        ),
    ];
    for (doc, what) in &cases {
        assert_agree(Kind::Ecdsa(CurveId::P256), doc, what);
    }
    assert!(pkcs8::ecdsa(CurveId::P256, &cases[1].0).is_ok());
    assert_eq!(
        pkcs8::ecdsa(CurveId::P256, &cases[3].0).unwrap_err(),
        KeyRejected::PublicKeyMismatch
    );
    assert_eq!(
        pkcs8::ecdsa(CurveId::P384, &der).unwrap_err(),
        KeyRejected::WrongAlgorithm
    );
}

/// The 32-byte scalar inside a P-256 document generated by `ring`.
fn scalar_of(der: &[u8]) -> Vec<u8> {
    let at = der
        .windows(4)
        .position(|w| w == [0x02, 0x01, 0x01, 0x04])
        .expect("ECPrivateKey version and scalar header");
    assert_eq!(der[at + 4], 32);
    der[at + 5..at + 37].to_vec()
}
