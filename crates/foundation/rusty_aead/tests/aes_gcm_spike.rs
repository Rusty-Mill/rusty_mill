//! Spike tests for AES-GCM: a specification vector and differential tests against `ring`.

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use rusty_aead::{Aes128Gcm, Aes256Gcm};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// McGrew and Viega, "The Galois/Counter Mode of Operation", test case 4 (AES-128, 60-byte
/// plaintext, 20-byte AAD).
#[test]
fn gcm_spec_test_case_4() {
    let key: [u8; 16] = hex("feffe9928665731c6d6a8f9467308308").try_into().unwrap();
    let iv: [u8; 12] = hex("cafebabefacedbaddecaf888").try_into().unwrap();
    let pt = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39");
    let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
    let want_ct = hex("42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091");
    let want_tag = hex("5bc94fbc3221a5db94fae95ae7121a47");
    let mut buf = pt.clone();
    let tag = Aes128Gcm::new(&key)
        .seal_in_place(&iv, &aad, &mut buf)
        .unwrap();
    assert_eq!(buf, want_ct);
    assert_eq!(tag.to_vec(), want_tag);
    Aes128Gcm::new(&key)
        .open_in_place(&iv, &aad, &mut buf, &tag)
        .unwrap();
    assert_eq!(buf, pt);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

fn ring_seal(
    alg: &'static aead::Algorithm,
    key: &[u8],
    nonce: [u8; 12],
    aad: &[u8],
    pt: &[u8],
) -> Vec<u8> {
    let k = LessSafeKey::new(UnboundKey::new(alg, key).unwrap());
    let mut buf = pt.to_vec();
    let tag = k
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut buf,
        )
        .unwrap();
    buf.extend_from_slice(tag.as_ref());
    buf
}

#[test]
fn matches_ring_across_lengths_and_aad_for_both_key_sizes() {
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    // Around the 16-byte block, the 63/64-block batch boundary (1008 and 1024 bytes) and larger.
    let lengths: Vec<usize> = (0..=130)
        .chain([
            255, 256, 1007, 1008, 1009, 1023, 1024, 1025, 2047, 2048, 4096, 16384,
        ])
        .collect();
    for len in lengths {
        for aad_len in [0usize, 1, 13, 16, 17, 64] {
            let key128: [u8; 16] = rng.bytes(16).try_into().unwrap();
            let key256: [u8; 32] = rng.bytes(32).try_into().unwrap();
            let nonce: [u8; 12] = rng.bytes(12).try_into().unwrap();
            let aad = rng.bytes(aad_len);
            let pt = rng.bytes(len);

            let want = ring_seal(&aead::AES_128_GCM, &key128, nonce, &aad, &pt);
            let mut buf = pt.clone();
            let tag = Aes128Gcm::new(&key128)
                .seal_in_place(&nonce, &aad, &mut buf)
                .unwrap();
            assert_eq!(
                (&buf[..], &tag[..]),
                (&want[..len], &want[len..]),
                "aes128 len={len} aad={aad_len}"
            );
            Aes128Gcm::new(&key128)
                .open_in_place(&nonce, &aad, &mut buf, &tag)
                .unwrap();
            assert_eq!(buf, pt);

            let want = ring_seal(&aead::AES_256_GCM, &key256, nonce, &aad, &pt);
            let mut buf = pt.clone();
            let tag = Aes256Gcm::new(&key256)
                .seal_in_place(&nonce, &aad, &mut buf)
                .unwrap();
            assert_eq!(
                (&buf[..], &tag[..]),
                (&want[..len], &want[len..]),
                "aes256 len={len} aad={aad_len}"
            );
            Aes256Gcm::new(&key256)
                .open_in_place(&nonce, &aad, &mut buf, &tag)
                .unwrap();
            assert_eq!(buf, pt);
        }
    }
}

#[test]
fn tampering_is_rejected_and_leaves_the_buffer_unchanged() {
    let key = [9u8; 16];
    let nonce = [3u8; 12];
    let cipher = Aes128Gcm::new(&key);
    let mut buf = b"attack at dawn, attack at dawn!!".to_vec();
    let tag = cipher.seal_in_place(&nonce, b"hdr", &mut buf).unwrap();
    let sealed = buf.clone();

    let mut bad_tag = tag;
    bad_tag[15] ^= 1;
    assert!(cipher
        .open_in_place(&nonce, b"hdr", &mut buf, &bad_tag)
        .is_err());
    assert_eq!(buf, sealed);
    assert!(cipher
        .open_in_place(&nonce, b"hdX", &mut buf, &tag)
        .is_err());
    assert!(cipher
        .open_in_place(&[4u8; 12], b"hdr", &mut buf, &tag)
        .is_err());
    assert!(cipher
        .open_in_place(&nonce, b"hdr", &mut buf, &tag[..15])
        .is_err());
    let mut flipped = sealed.clone();
    flipped[0] ^= 0x80;
    assert!(cipher
        .open_in_place(&nonce, b"hdr", &mut flipped, &tag)
        .is_err());
    assert_eq!(buf, sealed);
    cipher
        .open_in_place(&nonce, b"hdr", &mut buf, &tag)
        .unwrap();
}
