//! Ephemeral key exchange — stage 3c-i.
//!
//! # What is worth testing when the arithmetic is not ours
//!
//! `ring` does the curve maths, so a test that X25519 computes the right
//! answer would be a test of `ring`. What is genuinely this crate's to get
//! wrong is narrower, and it is what this file covers:
//!
//! 1. **The group numbers.** `secp256r1` is `0x0017` and `x25519` is `0x001d`;
//!    swapping them is a silent interoperability failure, not a crash.
//! 2. **The public key encoding.** A `key_share` entry carries 32 raw octets
//!    for X25519 and an uncompressed point for the NIST curves. Getting the
//!    length or the `0x04` prefix wrong makes every handshake fail.
//! 3. **That the pairing actually agrees.** Two independently generated keys
//!    must reach the same secret from either side.
//! 4. **That hostile key shares are refused** — which `ring` decides, and
//!    which is tested here anyway, because "we pass this to `ring` correctly"
//!    is exactly the part that could break without anyone noticing.
//!
//! # The small-order tests are the ones that matter
//!
//! [`a_small_order_x25519_key_share_is_refused`] is the security-relevant
//! case. A peer that sends a small-order point forces the shared secret to a
//! value it already knows, and an implementation that carries on derives every
//! traffic key from a constant. RFC 7748 §6.1 requires the check and RFC 8446
//! §7.4.2 requires it for TLS; `ring` performs it, and this asserts that the
//! call reaches it rather than sidestepping it.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::kx::{KeyExchange, KxError, NamedGroup};

const GROUPS: [NamedGroup; 3] = [
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).expect("hex"))
        .collect()
}

// ---------------------------------------------------------------------------
// The wire contract
// ---------------------------------------------------------------------------

/// The `NamedGroup` numbers, which are not guessable and are load-bearing.
#[test]
fn the_group_numbers_are_the_registry_values() {
    assert_eq!(NamedGroup::SecP256R1.as_u16(), 0x0017);
    assert_eq!(NamedGroup::SecP384R1.as_u16(), 0x0018);
    assert_eq!(NamedGroup::X25519.as_u16(), 0x001d);

    for group in GROUPS {
        assert_eq!(
            NamedGroup::from_u16(group.as_u16()),
            Some(group),
            "{group:?} did not survive a round trip through its wire value"
        );
    }
}

/// An unknown group is `None`, never a default. A client that fell back to a
/// group the server did not name would compute a secret nobody shares.
#[test]
fn an_unknown_group_is_refused_rather_than_defaulted() {
    for value in [
        0x0000, 0x0001, 0x0016, 0x0019, 0x001c, 0x001e, 0x0100, 0xffff,
    ] {
        assert_eq!(
            NamedGroup::from_u16(value),
            None,
            "0x{value:04x} was mapped to a group"
        );
    }
}

/// The `key_share` encodings: 32 raw octets for X25519 (RFC 7748 §5), an
/// uncompressed point for the NIST curves (RFC 8446 §4.2.8.2, SEC1 §2.3.3).
#[test]
fn public_keys_are_encoded_the_way_a_key_share_carries_them() {
    let x25519 = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    assert_eq!(x25519.public_key().len(), 32, "x25519 is 32 raw octets");

    let p256 = KeyExchange::generate(NamedGroup::SecP256R1).expect("generate");
    assert_eq!(
        p256.public_key().len(),
        65,
        "P-256 uncompressed: 1 + 32 + 32"
    );
    assert_eq!(p256.public_key()[0], 0x04, "the uncompressed point marker");

    let p384 = KeyExchange::generate(NamedGroup::SecP384R1).expect("generate");
    assert_eq!(
        p384.public_key().len(),
        97,
        "P-384 uncompressed: 1 + 48 + 48"
    );
    assert_eq!(p384.public_key()[0], 0x04, "the uncompressed point marker");
}

/// `group()` reports what was asked for. Trivial, and the thing that silently
/// breaks if the struct's fields are ever reordered into the wrong assignment.
#[test]
fn a_key_remembers_which_group_it_is_for() {
    for group in GROUPS {
        let kx = KeyExchange::generate(group).expect("generate");
        assert_eq!(kx.group(), group);
    }
}

// ---------------------------------------------------------------------------
// Agreement
// ---------------------------------------------------------------------------

/// Both sides reach the same secret. The property the whole module exists for.
#[test]
fn two_parties_agree_the_same_secret_in_both_directions() {
    for group in GROUPS {
        let client = KeyExchange::generate(group).expect("generate");
        let server = KeyExchange::generate(group).expect("generate");

        let client_public = client.public_key().to_vec();
        let server_public = server.public_key().to_vec();

        let from_client = client
            .agree(&server_public, <[u8]>::to_vec)
            .expect("the client agrees");
        let from_server = server
            .agree(&client_public, <[u8]>::to_vec)
            .expect("the server agrees");

        assert_eq!(from_client, from_server, "{group:?} disagreed");
        assert!(!from_client.is_empty());
        assert!(
            from_client.iter().any(|&b| b != 0),
            "{group:?} produced an all-zero secret"
        );
    }
}

/// Every connection gets its own key. A generator that returned the same key
/// twice would destroy forward secrecy while every other test still passed.
#[test]
fn every_generated_key_is_fresh() {
    for group in GROUPS {
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for _ in 0..8 {
            let public = KeyExchange::generate(group)
                .expect("generate")
                .public_key()
                .to_vec();
            assert!(
                !seen.contains(&public),
                "{group:?} generated the same public key twice"
            );
            seen.push(public);
        }
    }
}

/// The secret is whatever the closure makes of it — the closure is the only
/// route to the bytes, which is what keeps a long-lived copy from existing
/// unless a caller asks for one in plain sight.
#[test]
fn the_secret_reaches_the_caller_only_through_the_closure() {
    let client = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    let server = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    let server_public = server.public_key().to_vec();

    let length = client.agree(&server_public, <[u8]>::len).expect("agree");
    assert_eq!(length, 32, "an X25519 secret is 32 octets");
}

// ---------------------------------------------------------------------------
// Refusal
// ---------------------------------------------------------------------------

/// The security-relevant one. See the module docs.
///
/// Each of these points has small order on Curve25519, so `k * P` lands on the
/// identity for every clamped scalar and the shared secret is all zeroes — a
/// value the peer chose and already knows. Carrying on would derive every
/// traffic key in the connection from a constant.
#[test]
fn a_small_order_x25519_key_share_is_refused() {
    // RFC 7748 §6.1 and the standard small-order set: the identity, the two
    // order-4 points, the two order-8 points, and p, p+1, p-1 reduced.
    let small_order = [
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0100000000000000000000000000000000000000000000000000000000000000",
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ];

    for point in small_order {
        let kx = KeyExchange::generate(NamedGroup::X25519).expect("generate");
        assert_eq!(
            kx.agree(&hex(point), <[u8]>::to_vec).err(),
            Some(KxError::BadPeerKey),
            "a small-order point was accepted: {point}"
        );
    }
}

/// A key share of the wrong length is refused rather than padded, truncated,
/// or interpreted as a different group's encoding.
#[test]
fn a_key_share_of_the_wrong_length_is_refused() {
    for group in GROUPS {
        let correct = KeyExchange::generate(group)
            .expect("generate")
            .public_key()
            .to_vec();

        for wrong in [
            Vec::new(),
            vec![0x04],
            correct[..correct.len() - 1].to_vec(),
            {
                let mut longer = correct.clone();
                longer.push(0x00);
                longer
            },
        ] {
            let kx = KeyExchange::generate(group).expect("generate");
            assert_eq!(
                kx.agree(&wrong, <[u8]>::to_vec).err(),
                Some(KxError::BadPeerKey),
                "{group:?} accepted a {}-octet key share",
                wrong.len()
            );
        }
    }
}

/// A point that is not on the curve is refused. An implementation that
/// multiplied by it anyway leaks information about its own private key —
/// the invalid-curve attack.
#[test]
fn a_nist_point_that_is_not_on_the_curve_is_refused() {
    for (group, length) in [(NamedGroup::SecP256R1, 65), (NamedGroup::SecP384R1, 97)] {
        // Correctly shaped: the uncompressed marker and the right length. The
        // coordinates are what is wrong, which is the only way this test is
        // distinguishable from the length test above.
        let mut point = vec![0x04u8; length];
        point[0] = 0x04;
        for (index, byte) in point.iter_mut().enumerate().skip(1) {
            *byte = index as u8;
        }

        let kx = KeyExchange::generate(group).expect("generate");
        assert_eq!(
            kx.agree(&point, <[u8]>::to_vec).err(),
            Some(KxError::BadPeerKey),
            "{group:?} accepted a point that is not on the curve"
        );
    }
}

/// A key share from a different group is refused. A server that echoes the
/// wrong group, or a client that fails to check which group came back, must
/// not quietly reinterpret one curve's encoding as another's.
#[test]
fn a_key_share_from_another_group_is_refused() {
    for group in GROUPS {
        for other in GROUPS {
            if other == group {
                continue;
            }
            let peer = KeyExchange::generate(other)
                .expect("generate")
                .public_key()
                .to_vec();
            let kx = KeyExchange::generate(group).expect("generate");
            assert_eq!(
                kx.agree(&peer, <[u8]>::to_vec).err(),
                Some(KxError::BadPeerKey),
                "{group:?} accepted a {other:?} key share"
            );
        }
    }
}

/// Every refusal looks identical from outside. A peer that could tell "wrong
/// length" from "not on the curve" from "small order" would have an oracle.
#[test]
fn every_refusal_is_the_same_error() {
    let kx = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    let short = kx.agree(&[0u8; 31], <[u8]>::to_vec).unwrap_err();
    let kx = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    let zero = kx.agree(&[0u8; 32], <[u8]>::to_vec).unwrap_err();

    assert_eq!(short, zero);
    assert_eq!(short.to_string(), zero.to_string());
}

// ---------------------------------------------------------------------------
// Hygiene
// ---------------------------------------------------------------------------

/// `Debug` must not carry key material.
///
/// `ring`'s own `Debug` already prints only the algorithm, so this is not
/// fixing a leak — it asserts that the guarantee is this crate's and would
/// survive `ring` changing its mind about what to render.
#[test]
fn debug_output_does_not_contain_key_material() {
    let kx = KeyExchange::generate(NamedGroup::X25519).expect("generate");
    let rendered = format!("{kx:?}");

    assert!(rendered.contains("<redacted>"), "{rendered}");
    assert!(rendered.contains("X25519"), "the group is not a secret");

    // No run of bytes from the public key, and nothing that looks like a
    // 32-octet dump, should appear.
    assert!(
        !rendered.contains(&format!("{:?}", kx.public_key())),
        "the raw key bytes were rendered: {rendered}"
    );
}

/// The error type renders every variant without panicking, and says something
/// different for each.
#[test]
fn every_error_renders_distinctly() {
    let errors = [
        KxError::UnsupportedGroup(0x0100),
        KxError::Generation,
        KxError::BadPeerKey,
    ];
    let rendered: Vec<String> = errors.iter().map(ToString::to_string).collect();

    for (index, text) in rendered.iter().enumerate() {
        assert!(!text.is_empty(), "{:?} rendered empty", errors[index]);
        assert_eq!(
            rendered.iter().filter(|other| *other == text).count(),
            1,
            "two errors render identically: {text}"
        );
    }
}
