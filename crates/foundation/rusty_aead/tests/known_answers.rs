//! RFC 8439 vectors and the vendored Wycheproof ChaCha20-Poly1305 file.

use rusty_aead::{ChaCha20Poly1305, Poly1305, TAG_LEN};
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_hex::{decode, encode};

#[test]
fn poly1305_rfc_8439_section_2_5_2() {
    let key: [u8; 32] =
        decode_array("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b");
    let mut mac = Poly1305::new(&key);
    mac.update(b"Cryptographic Forum Research Group");
    assert_eq!(encode(&mac.finalize()), "a8061dc1305136c6c22b8baf0c0127a9");
}

#[test]
fn poly1305_streaming_matches_one_shot_at_every_split() {
    let key = [0x42u8; 32];
    let data: Vec<u8> = (0..100u8).collect();
    let mut whole = Poly1305::new(&key);
    whole.update(&data);
    let expected = whole.finalize();
    for cut in 0..=data.len() {
        let mut mac = Poly1305::new(&key);
        mac.update(&data[..cut]);
        mac.update(&data[cut..]);
        assert_eq!(mac.finalize(), expected, "split at {cut}");
    }
}

#[test]
fn poly1305_empty_message_is_the_pad() {
    // With no blocks, h = 0 and the tag is just s.
    let mut key = [0u8; 32];
    key[16..].copy_from_slice(&[0xaa; 16]);
    assert_eq!(Poly1305::new(&key).finalize(), [0xaa; 16]);
}

#[test]
fn aead_rfc_8439_section_2_8_2() {
    let key = decode_array("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let nonce = decode_array("070000004041424344454647");
    let aad = decode("50515253c0c1c2c3c4c5c6c7").unwrap();
    let mut buf = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
    let cipher = ChaCha20Poly1305::new(&key);
    let tag = cipher.seal_in_place(&nonce, &aad, &mut buf).unwrap();
    assert_eq!(encode(&tag), "1ae10b594f09e26a7e902ecbd0600691");
    assert!(encode(&buf).starts_with("d31a8d34648e60db7b86afbc53ef7ec2"));
    cipher.open_in_place(&nonce, &aad, &mut buf, &tag).unwrap();
    assert!(buf.starts_with(b"Ladies and Gentlemen"));
}

#[test]
fn open_failure_leaves_the_buffer_untouched() {
    let cipher = ChaCha20Poly1305::new(&[9u8; 32]);
    let nonce = [3u8; 12];
    let mut buf = b"secret data".to_vec();
    let tag = cipher.seal_in_place(&nonce, b"", &mut buf).unwrap();
    let ciphertext = buf.clone();
    let mut bad = tag;
    bad[0] ^= 1;
    assert!(cipher.open_in_place(&nonce, b"", &mut buf, &bad).is_err());
    assert_eq!(buf, ciphertext, "failed open must not decrypt");
    assert!(cipher.open_in_place(&nonce, b"x", &mut buf, &tag).is_err());
    assert!(cipher
        .open_in_place(&[4u8; 12], b"", &mut buf, &tag)
        .is_err());
    assert!(cipher
        .open_in_place(&nonce, b"", &mut buf, &tag[..TAG_LEN - 1])
        .is_err());
    assert!(cipher.open_in_place(&nonce, b"", &mut buf, &[]).is_err());
    assert!(cipher.open_in_place(&nonce, b"", &mut buf, &tag).is_ok());
    assert_eq!(buf, b"secret data");
}

#[test]
fn empty_plaintext_and_empty_aad_round_trip() {
    let cipher = ChaCha20Poly1305::new(&[1u8; 32]);
    let nonce = [2u8; 12];
    let tag = cipher.seal_in_place(&nonce, b"", &mut []).unwrap();
    assert!(cipher.open_in_place(&nonce, b"", &mut [], &tag).is_ok());
}

#[test]
fn wycheproof() {
    let all = cases(include_str!("vectors/chacha20_poly1305_test.json")).unwrap();
    assert!(all.len() > 300, "file looks truncated");
    for case in all {
        let iv = case.hex("iv").unwrap();
        let (key, aad, msg) = (
            case.hex("key").unwrap(),
            case.hex("aad").unwrap(),
            case.hex("msg").unwrap(),
        );
        let (ct, tag) = (case.hex("ct").unwrap(), case.hex("tag").unwrap());
        let Ok(nonce) = <[u8; 12]>::try_from(iv.as_slice()) else {
            // The typed API cannot even express a wrong-size nonce.
            assert_eq!(case.verdict, Verdict::Invalid, "tcId {}", case.id);
            continue;
        };
        let key: [u8; 32] = key.try_into().unwrap();
        let cipher = ChaCha20Poly1305::new(&key);
        let mut sealed = msg.clone();
        let produced = cipher.seal_in_place(&nonce, &aad, &mut sealed).unwrap();
        let mut opened = ct.clone();
        let accepted = cipher
            .open_in_place(&nonce, &aad, &mut opened, &tag)
            .is_ok();
        match case.verdict {
            Verdict::Valid => {
                assert_eq!(sealed, ct, "tcId {} ciphertext", case.id);
                assert_eq!(produced.as_slice(), tag, "tcId {} tag", case.id);
                assert!(accepted && opened == msg, "tcId {} open", case.id);
            }
            _ => assert!(!accepted, "tcId {} must reject ({})", case.id, case.comment),
        }
    }
}

fn decode_array<const N: usize>(hex: &str) -> [u8; N] {
    rusty_hex::decode_array(hex).unwrap()
}

/// Messages that drive the accumulator to exactly `p - 1`, `p` and `p + 3`
/// (p = 2^130 - 5) so the branch-free final reduction must select the
/// subtracted value. Key: r = 1, s = 0, so h is just the sum of the blocks.
/// Expected tags computed with an independent big-integer Poly1305.
#[test]
fn poly1305_final_reduction_edges() {
    let mut key = [0u8; 32];
    key[0] = 1;
    let tag = |msg: &[u8]| {
        let mut mac = Poly1305::new(&key);
        mac.update(msg);
        encode(&mac.finalize())
    };
    let ff = [0xffu8; 16];
    let with = |first: u8| {
        let mut block = ff;
        block[0] = first;
        [ff, block].concat()
    };
    assert_eq!(tag(&[ff, ff].concat()), "03000000000000000000000000000000"); // h = p + 3
    assert_eq!(tag(&with(0xfc)), "00000000000000000000000000000000"); // h = p
    assert_eq!(tag(&with(0xfb)), "faffffffffffffffffffffffffffffff"); // h = p - 1
}
