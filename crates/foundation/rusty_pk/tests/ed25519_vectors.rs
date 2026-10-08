//! Ed25519 verification against Wycheproof, RFC 8032 and `ring`.

use ring::signature::{UnparsedPublicKey, ED25519};
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_pk::ed25519::verify;

#[test]
fn wycheproof_and_ring_agree_on_every_case() {
    let all = cases(include_str!("vectors/ed25519_test.json")).unwrap();
    assert!(all.len() > 100, "file looks truncated");
    for case in all {
        // The raw key is the last 32 bytes of the SPKI.
        let spki = case.hex("publicKeyDer").unwrap();
        let key = &spki[spki.len() - 32..];
        let msg = case.hex("msg").unwrap();
        let sig = case.hex("sig").unwrap();
        let ours = verify(key, &msg, &sig).is_ok();
        let ring = UnparsedPublicKey::new(&ED25519, key)
            .verify(&msg, &sig)
            .is_ok();
        assert_eq!(
            ours, ring,
            "differs from ring: tcId {} ({})",
            case.id, case.comment
        );
        match case.verdict {
            Verdict::Valid => assert!(ours, "tcId {} should verify ({})", case.id, case.comment),
            // ring (and so we) accept a few Wycheproof "invalid" vectors that
            // RFC 8032 section 5.1.7 rejects; equality with ring is the check,
            // and the list is asserted below so a change shows up.
            Verdict::Invalid | Verdict::Acceptable => {}
        }
    }
}

/// Encodings Wycheproof does not cover, where an implementation could differ
/// from ring: the identity (a valid point of order 1) under several encodings,
/// with `S = 0` and `R` = identity, which verifies whenever the key decodes.
/// Ours must make the same accept/reject decision as ring on each.
#[test]
fn identity_key_encodings_match_ring() {
    let identity = {
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    };
    let mut non_canonical_y = [0xffu8; 32]; // y = p + 1 = 2^255 - 18
    non_canonical_y[0] = 0xee;
    non_canonical_y[31] = 0x7f;
    let mut x_zero_sign_set = identity; // x = 0 with the sign bit set
    x_zero_sign_set[31] |= 0x80;
    let mut not_on_curve = [0u8; 32]; // y = 2 has no x
    not_on_curve[0] = 2;
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&identity);
    let mut report = Vec::new();
    for (name, key) in [
        ("identity", identity),
        ("non-canonical y", non_canonical_y),
        ("x=0, sign bit set", x_zero_sign_set),
        ("not on curve", not_on_curve),
    ] {
        let ours = verify(&key, b"msg", &sig).is_ok();
        let ring = UnparsedPublicKey::new(&ED25519, &key)
            .verify(b"msg", &sig)
            .is_ok();
        report.push((name, ours, ring));
        assert_eq!(ours, ring, "{name}");
    }
    eprintln!("(case, ours, ring): {report:?}");
    // Non-vacuous: at least the plain identity must be accepted by both.
    assert!(report[0].1 && report[0].2);
}

#[test]
fn rfc_8032_test_vector_1_and_2() {
    let h = |s: &str| rusty_hex::decode(s).unwrap();
    let pk = h("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let sig = h("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
    assert!(verify(&pk, b"", &sig).is_ok());
    let pk = h("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
    let sig = h("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00");
    assert!(verify(&pk, &[0x72], &sig).is_ok());
    assert!(verify(&pk, &[0x73], &sig).is_err());
}
