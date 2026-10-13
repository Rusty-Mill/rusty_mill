//! The vendored Wycheproof AES-GCM file (`testvectors_v1/aes_gcm_test.json`, pinned in
//! `vectors/MANIFEST.txt`).
//!
//! The typed API supports AES-128 and AES-256 with a 96-bit IV and a 16-byte tag only, so the
//! 183 cases that use AES-192 or another IV length cannot be run through it. They are counted
//! and named here, not silently dropped: the executed and skipped totals are pinned, so a
//! schema change or a truncated file fails the test instead of passing on fewer cases.

use rusty_aead::{Aes128Gcm, Aes256Gcm};
use rusty_ct_check::wycheproof::{cases, Verdict};

const TOTAL: usize = 316;
const EXECUTED: usize = 133;
const SKIPPED_AES192: usize = 103;
const SKIPPED_IV_NOT_96_BITS: usize = 80;

enum Cipher {
    A128(Aes128Gcm),
    A256(Aes256Gcm),
}

impl Cipher {
    fn seal(&self, nonce: &[u8; 12], aad: &[u8], buf: &mut [u8]) -> [u8; 16] {
        match self {
            Cipher::A128(c) => c.seal_in_place(nonce, aad, buf).unwrap(),
            Cipher::A256(c) => c.seal_in_place(nonce, aad, buf).unwrap(),
        }
    }
    fn open(&self, nonce: &[u8; 12], aad: &[u8], buf: &mut [u8], tag: &[u8]) -> bool {
        match self {
            Cipher::A128(c) => c.open_in_place(nonce, aad, buf, tag).is_ok(),
            Cipher::A256(c) => c.open_in_place(nonce, aad, buf, tag).is_ok(),
        }
    }
}

#[test]
fn wycheproof_aes_gcm() {
    let all = cases(include_str!("vectors/aes_gcm_test.json")).unwrap();
    assert_eq!(all.len(), TOTAL, "file looks truncated or changed");
    let (mut executed, mut aes192, mut other_iv) = (0, 0, 0);
    for case in all {
        let key = case.hex("key").unwrap();
        let iv = case.hex("iv").unwrap();
        let (aad, msg) = (case.hex("aad").unwrap(), case.hex("msg").unwrap());
        let (ct, tag) = (case.hex("ct").unwrap(), case.hex("tag").unwrap());
        assert_eq!(
            tag.len(),
            16,
            "tcId {}: every tag in the file is 128 bits",
            case.id
        );
        let cipher = match key.len() {
            16 => Cipher::A128(Aes128Gcm::new(&key.clone().try_into().unwrap())),
            32 => Cipher::A256(Aes256Gcm::new(&key.clone().try_into().unwrap())),
            24 => {
                aes192 += 1;
                continue;
            }
            n => panic!("tcId {}: unexpected key length {n}", case.id),
        };
        let Ok(nonce) = <[u8; 12]>::try_from(iv.as_slice()) else {
            other_iv += 1;
            continue;
        };
        executed += 1;

        let mut sealed = msg.clone();
        let produced = cipher.seal(&nonce, &aad, &mut sealed);
        let mut opened = ct.clone();
        let accepted = cipher.open(&nonce, &aad, &mut opened, &tag);
        match case.verdict {
            Verdict::Valid => {
                assert_eq!(sealed, ct, "tcId {} ciphertext", case.id);
                assert_eq!(produced.as_slice(), tag, "tcId {} tag", case.id);
                assert!(accepted && opened == msg, "tcId {} open", case.id);
            }
            _ => {
                assert!(!accepted, "tcId {} must reject ({})", case.id, case.comment);
                assert_eq!(
                    opened, ct,
                    "tcId {}: a rejected open must not decrypt",
                    case.id
                );
            }
        }
    }
    assert_eq!(
        (executed, aes192, other_iv),
        (EXECUTED, SKIPPED_AES192, SKIPPED_IV_NOT_96_BITS)
    );
    assert_eq!(executed + aes192 + other_iv, TOTAL);
}
