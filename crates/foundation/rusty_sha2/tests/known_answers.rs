//! Published vectors and the vendored Wycheproof files.

use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_hex::{decode, encode};
use rusty_sha2::{extract, Hash, Hmac, Prk, Sha256, Sha384, Sha512};

fn hex_of<H: Hash>(data: &[u8]) -> String {
    encode(H::digest(data).as_ref())
}

#[test]
fn fips_180_examples() {
    assert_eq!(
        hex_of::<Sha256>(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex_of::<Sha256>(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex_of::<Sha384>(b"abc"),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
         8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(
        hex_of::<Sha512>(b"abc"),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
}

#[test]
fn sha256_million_a_streamed() {
    let mut hasher = Sha256::new();
    for _ in 0..1000 {
        hasher.update(&[b'a'; 1000]);
    }
    assert_eq!(
        encode(hasher.finalize().as_ref()),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn padding_boundaries_agree_with_one_shot() {
    // 55, 56 and 63..65 bytes straddle the SHA-256 length-field boundary;
    // 111, 112 and 127..129 do the same for SHA-512.
    for len in [
        0usize, 1, 55, 56, 57, 63, 64, 65, 111, 112, 113, 127, 128, 129, 255, 256,
    ] {
        let data = vec![0xa5u8; len];
        let mut split = Sha512::new();
        split.update(&data[..len / 3]);
        split.update(&data[len / 3..]);
        assert_eq!(split.finalize(), Sha512::digest(&data), "len {len}");
    }
}

#[test]
fn hmac_rfc_4231_case_1() {
    let tag = Hmac::<Sha256>::mac(&[0x0b; 20], b"Hi There");
    assert_eq!(
        encode(&tag),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn hmac_verify_rejects_wrong_and_truncated_tags() {
    let tag = Hmac::<Sha256>::mac(b"key", b"msg");
    assert!(Hmac::<Sha256>::verify(b"key", b"msg", &tag));
    let mut bad = tag;
    bad[31] ^= 1;
    assert!(!Hmac::<Sha256>::verify(b"key", b"msg", &bad));
    assert!(!Hmac::<Sha256>::verify(b"key", b"msg", &tag[..31]));
    assert!(!Hmac::<Sha256>::verify(b"key", b"msg", &[]));
    assert!(!Hmac::<Sha256>::verify(b"other", b"msg", &tag));
}

#[test]
fn hkdf_rejects_oversized_output_and_bad_prk_length() {
    let prk = extract::<Sha256>(None, b"ikm");
    let mut out = vec![0u8; 255 * 32 + 1];
    assert!(prk.expand(b"", &mut out).is_err());
    let mut max = vec![0u8; 255 * 32];
    assert!(prk.expand(b"", &mut max).is_ok());
    assert!(Prk::<Sha256>::from_bytes(&[0u8; 31]).is_none());
    assert!(Prk::<Sha256>::from_bytes(&[0u8; 33]).is_none());
    assert!(Prk::<Sha256>::from_bytes(&[0u8; 32]).is_some());
}

#[test]
fn hkdf_zero_length_output_is_ok_and_empty() {
    let prk = extract::<Sha384>(Some(b"salt"), b"ikm");
    assert!(prk.expand(b"info", &mut []).is_ok());
}

fn wycheproof_hmac<H: Hash>(json: &str) {
    let all = cases(json).unwrap();
    assert!(all.len() > 100, "file looks truncated");
    for case in all {
        let key = case.hex("key").unwrap();
        let msg = case.hex("msg").unwrap();
        let tag = case.hex("tag").unwrap();
        let bits = case.uint("tagSize").unwrap() as usize;
        let full = Hmac::<H>::mac(&key, &msg);
        let produced = &full.as_ref()[..bits / 8];
        match case.verdict {
            Verdict::Valid => assert_eq!(produced, tag, "tcId {}", case.id),
            _ => assert_ne!(produced, tag, "tcId {}", case.id),
        }
        if bits / 8 == H::OUTPUT_LEN {
            let accepted = Hmac::<H>::verify(&key, &msg, &tag);
            assert_eq!(accepted, case.verdict == Verdict::Valid, "tcId {}", case.id);
        }
    }
}

fn wycheproof_hkdf<H: Hash>(json: &str) {
    let all = cases(json).unwrap();
    assert!(all.len() > 50, "file looks truncated");
    for case in all {
        let prk = extract::<H>(Some(&case.hex("salt").unwrap()), &case.hex("ikm").unwrap());
        let mut out = vec![0u8; case.uint("size").unwrap() as usize];
        let result = prk.expand(&case.hex("info").unwrap(), &mut out);
        match case.verdict {
            Verdict::Valid => {
                assert!(result.is_ok(), "tcId {}", case.id);
                assert_eq!(out, case.hex("okm").unwrap(), "tcId {}", case.id);
            }
            _ => assert!(result.is_err(), "tcId {}", case.id),
        }
    }
}

macro_rules! vector_file {
    ($name:literal) => {
        include_str!(concat!("vectors/", $name, ".json"))
    };
}

#[test]
fn wycheproof_hmac_sha256() {
    wycheproof_hmac::<Sha256>(vector_file!("hmac_sha256_test"));
}
#[test]
fn wycheproof_hmac_sha384() {
    wycheproof_hmac::<Sha384>(vector_file!("hmac_sha384_test"));
}
#[test]
fn wycheproof_hmac_sha512() {
    wycheproof_hmac::<Sha512>(vector_file!("hmac_sha512_test"));
}
#[test]
fn wycheproof_hkdf_sha256() {
    wycheproof_hkdf::<Sha256>(vector_file!("hkdf_sha256_test"));
}
#[test]
fn wycheproof_hkdf_sha384() {
    wycheproof_hkdf::<Sha384>(vector_file!("hkdf_sha384_test"));
}
#[test]
fn wycheproof_hkdf_sha512() {
    wycheproof_hkdf::<Sha512>(vector_file!("hkdf_sha512_test"));
}

#[test]
fn hex_helper_roundtrip_sanity() {
    assert_eq!(decode("0aff").unwrap(), vec![0x0a, 0xff]);
}
