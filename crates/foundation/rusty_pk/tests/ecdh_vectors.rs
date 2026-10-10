//! ECDH on P-256 and P-384 against Wycheproof (`ecpoint` files) and `ring`.

use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use ring::rand::SystemRandom;
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_pk::ecdh::{Curve, PrivateKey};

const P256: &str = include_str!("vectors/ecdh_secp256r1_ecpoint_test.json");
const P384: &str = include_str!("vectors/ecdh_secp384r1_ecpoint_test.json");

fn files() -> [(Curve, &'static str, usize); 2] {
    [(Curve::P256, P256, 355), (Curve::P384, P384, 790)]
}

/// Wycheproof stores the private key as a signed DER integer: strip leading zeros
/// and left-pad to the curve's scalar length. `None` if it does not fit.
fn scalar(curve: Curve, raw: &[u8]) -> Option<Vec<u8>> {
    let start = raw.iter().position(|&b| b != 0).unwrap_or(raw.len());
    let digits = &raw[start..];
    let pad = curve.len().checked_sub(digits.len())?;
    let mut out = vec![0u8; pad];
    out.extend_from_slice(digits);
    Some(out)
}

#[test]
fn wycheproof_every_verdict() {
    for (curve, json, expected) in files() {
        let all = cases(json).unwrap();
        assert_eq!(all.len(), expected, "{curve:?}: file looks truncated");
        let (mut valid, mut invalid, mut acceptable) = (0, 0, 0);
        for case in all {
            let public = case.hex("public").unwrap();
            let private = scalar(curve, &case.hex("private").unwrap()).expect("scalar fits");
            let key = PrivateKey::from_bytes(curve, &private)
                .unwrap_or_else(|_| panic!("{curve:?} tcId {}: private key rejected", case.id));
            let got = key.agree(&public);
            match case.verdict {
                Verdict::Valid => {
                    let shared = case.hex("shared").unwrap();
                    let got = got.unwrap_or_else(|_| {
                        panic!("{curve:?} tcId {} ({}) rejected", case.id, case.comment)
                    });
                    assert_eq!(
                        got.as_bytes(),
                        &shared[..],
                        "{curve:?} tcId {} ({})",
                        case.id,
                        case.comment
                    );
                    valid += 1;
                }
                Verdict::Invalid => {
                    assert!(
                        got.is_err(),
                        "{curve:?} tcId {} ({}) accepted",
                        case.id,
                        case.comment
                    );
                    invalid += 1;
                }
                // Compressed points: allowed either way by Wycheproof, refused by
                // design (TLS 1.3 sends uncompressed points only).
                Verdict::Acceptable => {
                    assert!(
                        case.has_flag("CompressedPoint") && got.is_err(),
                        "tcId {}",
                        case.id
                    );
                    acceptable += 1;
                }
            }
        }
        eprintln!("{curve:?}: {valid} valid, {invalid} invalid, {acceptable} acceptable");
        assert!(valid > 300 && invalid >= 18 && acceptable == 1, "{curve:?}");
    }
}

#[test]
fn rejects_exactly_the_public_keys_ring_rejects() {
    let rng = SystemRandom::new();
    for (curve, json, _) in files() {
        let alg = match curve {
            Curve::P256 => &agreement::ECDH_P256,
            Curve::P384 => &agreement::ECDH_P384,
        };
        let ours = PrivateKey::from_bytes(curve, &scalar(curve, &[0x5a]).unwrap()).unwrap();
        for case in cases(json).unwrap() {
            let public = case.hex("public").unwrap();
            let theirs = EphemeralPrivateKey::generate(alg, &rng).unwrap();
            let ring_ok =
                agreement::agree_ephemeral(theirs, &UnparsedPublicKey::new(alg, &public), |_| ())
                    .is_ok();
            assert_eq!(
                ours.agree(&public).is_ok(),
                ring_ok,
                "{curve:?} tcId {}",
                case.id
            );
        }
    }
}

#[test]
fn shared_secret_agrees_with_ring_both_ways() {
    let rng = SystemRandom::new();
    for (curve, alg) in [
        (Curve::P256, &agreement::ECDH_P256),
        (Curve::P384, &agreement::ECDH_P384),
    ] {
        for i in 0..20u8 {
            let ours = PrivateKey::generate(curve, |b: &mut [u8]| -> Result<(), ()> {
                for (j, x) in b.iter_mut().enumerate() {
                    *x = (usize::from(i) * 29 + j * 13 + 1) as u8 ^ 0x6d;
                }
                Ok(())
            })
            .unwrap();
            let our_public = ours.public_key().unwrap();

            // ring's key, our scalar
            let theirs = EphemeralPrivateKey::generate(alg, &rng).unwrap();
            let their_public = theirs.compute_public_key().unwrap().as_ref().to_vec();
            let ring_shared = agreement::agree_ephemeral(
                theirs,
                &UnparsedPublicKey::new(alg, our_public.as_bytes()),
                |s| s.to_vec(),
            )
            .unwrap();
            assert_eq!(
                ours.agree(&their_public).unwrap().as_bytes(),
                &ring_shared[..],
                "{curve:?} {i}"
            );
        }
    }
}

#[test]
fn public_key_matches_ring_for_a_known_scalar() {
    // RFC 5903 section 8.1 (P-256) and 8.3 (P-384) initiator keys.
    let h = |s: &str| rusty_hex::decode(s).unwrap();
    for (curve, d, x, y) in [
        (
            Curve::P256,
            "c88f01f510d9ac3f70a292daa2316de544e9aab8afe84049c62a9c57862d1433",
            "dad0b65394221cf9b051e1feca5787d098dfe637fc90b9ef945d0c3772581180",
            "5271a0461cdb8252d61f1c456fa3e59ab1f45b33accf5f58389e0577b8990bb3",
        ),
        (
            Curve::P384,
            "099f3c7034d4a2c699884d73a375a67f7624ef7c6b3c0f160647b67414dce655e35b538041e649ee3faef896783ab194",
            "667842d7d180ac2cde6f74f37551f55755c7645c20ef73e31634fe72b4c55ee6de3ac808acb4bdb4c88732aee95f41aa",
            "9482ed1fc0eeb9cafc4984625ccfc23f65032149e0e144ada024181535a0f38eeb9fcff3c2c947dae69b4c634573a81c",
        ),
    ] {
        let key = PrivateKey::from_bytes(curve, &h(d)).unwrap();
        let mut want = vec![4u8];
        want.extend(h(x));
        want.extend(h(y));
        assert_eq!(key.public_key().unwrap().as_bytes(), &want[..], "{curve:?}");
    }
}
