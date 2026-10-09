//! X25519 against Wycheproof and `ring`.

use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use ring::rand::SystemRandom;
use rusty_ct_check::wycheproof::cases;
use rusty_pk::x25519::{agree, public_key, x25519, LowOrderPoint};

fn arr(v: Vec<u8>) -> [u8; 32] {
    v.try_into().unwrap()
}

#[test]
fn wycheproof_matches_for_every_verdict() {
    let all = cases(include_str!("vectors/x25519_test.json")).unwrap();
    assert_eq!(all.len(), 518, "file looks truncated");
    let (mut zero_shared, mut low_order_rejected) = (0, 0);
    for case in all {
        let (private, public, shared) = (
            arr(case.hex("private").unwrap()),
            arr(case.hex("public").unwrap()),
            arr(case.hex("shared").unwrap()),
        );
        // The function value is fully specified, even for twists, low-order
        // and non-canonical points, so "acceptable" vectors must match too.
        assert_eq!(
            x25519(&private, &public),
            shared,
            "tcId {} ({})",
            case.id,
            case.comment
        );
        let is_zero = shared == [0u8; 32];
        assert_eq!(
            agree(&private, &public).is_err(),
            is_zero,
            "tcId {}",
            case.id
        );
        zero_shared += is_zero as usize;
        low_order_rejected += (agree(&private, &public) == Err(LowOrderPoint)) as usize;
    }
    assert!(zero_shared >= 20 && zero_shared == low_order_rejected);
}

#[test]
fn rejects_exactly_the_public_keys_ring_rejects() {
    // Whether the output is all zeros depends only on the peer key, so any
    // private key gives the same verdict as ring's freshly generated one.
    let rng = SystemRandom::new();
    for case in cases(include_str!("vectors/x25519_test.json")).unwrap() {
        let public = case.hex("public").unwrap();
        let theirs = EphemeralPrivateKey::generate(&agreement::X25519, &rng).unwrap();
        let theirs_ok = agreement::agree_ephemeral(
            theirs,
            &UnparsedPublicKey::new(&agreement::X25519, &public),
            |_| (),
        )
        .is_ok();
        let ours_ok = agree(&[0x5au8; 32], &arr(public)).is_ok();
        assert_eq!(ours_ok, theirs_ok, "tcId {}", case.id);
    }
}

#[test]
fn shared_secret_agrees_with_ring_both_ways() {
    let rng = SystemRandom::new();
    for i in 0..50u8 {
        let mut scalar = [0u8; 32];
        for (j, b) in scalar.iter_mut().enumerate() {
            *b = (i as usize * 31 + j * 7 + 3) as u8 ^ 0xa5;
        }
        let ours_pub = public_key(&scalar);

        // ring's key, our scalar
        let theirs = EphemeralPrivateKey::generate(&agreement::X25519, &rng).unwrap();
        let theirs_pub = arr(theirs.compute_public_key().unwrap().as_ref().to_vec());
        let ring_shared = agreement::agree_ephemeral(
            theirs,
            &UnparsedPublicKey::new(&agreement::X25519, ours_pub),
            |s| s.to_vec(),
        )
        .unwrap();
        assert_eq!(
            agree(&scalar, &theirs_pub).unwrap().to_vec(),
            ring_shared,
            "iteration {i}"
        );
    }
}
