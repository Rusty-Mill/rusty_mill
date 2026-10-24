//! ECDSA signing against `ring`: each side verifies the other's signatures and agrees on
//! public keys. ECDSA nonces are random in `ring` and hedged here, so signatures cannot be
//! compared byte for byte. Fixed-noise known answers are below; the plain RFC 6979 known
//! answers are unit tests in `src/ecdsa.rs`.

use std::collections::HashSet;

use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P384_SHA384_ASN1, ECDSA_P384_SHA384_ASN1_SIGNING,
};
use rusty_pk::ecdsa::{noise_len, verify, CurveId, Digest, SignError, SigningKey};

struct Lcg(u64);

impl Lcg {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (self.0 >> 33) as u8;
        }
    }
}

fn params(curve: CurveId) -> (usize, Digest) {
    match curve {
        CurveId::P256 => (32, Digest::Sha256),
        CurveId::P384 => (48, Digest::Sha384),
    }
}

/// 200 keys per curve, message lengths 0 to 398: our signatures verify under `ring` and
/// ours; `ring`'s verify under ours; `ring` accepts our public key for the private key.
#[test]
fn cross_verifies_with_ring_on_both_curves() {
    let rng = SystemRandom::new();
    let mut lcg = Lcg(0xec);
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, digest) = params(curve);
        let (signing, verifying) = match curve {
            CurveId::P256 => (&ECDSA_P256_SHA256_ASN1_SIGNING, &ECDSA_P256_SHA256_ASN1),
            CurveId::P384 => (&ECDSA_P384_SHA384_ASN1_SIGNING, &ECDSA_P384_SHA384_ASN1),
        };
        for i in 0..200 {
            let mut d = vec![0u8; len];
            lcg.fill(&mut d);
            d[0] &= 0x7f; // below n
            let ours = SigningKey::from_bytes(curve, &d).unwrap();
            let public = ours.public_key().unwrap();
            let theirs =
                EcdsaKeyPair::from_private_key_and_public_key(signing, &d, public.as_bytes(), &rng)
                    .expect("ring accepts our public key for this private key");
            assert_eq!(theirs.public_key().as_ref(), public.as_bytes());
            let mut msg = vec![0u8; i * 2];
            lcg.fill(&mut msg);

            let sig = ours
                .sign(&msg, |h: &mut [u8]| {
                    lcg.fill(h);
                    Ok::<(), ()>(())
                })
                .unwrap();
            assert!(
                verify(curve, digest, public.as_bytes(), &msg, &sig).is_ok(),
                "{curve:?} {i}"
            );
            assert!(UnparsedPublicKey::new(verifying, public.as_bytes())
                .verify(&msg, &sig)
                .is_ok());

            let ring_sig = theirs.sign(&rng, &msg).unwrap();
            assert!(verify(curve, digest, public.as_bytes(), &msg, ring_sig.as_ref()).is_ok());
        }
    }
}

/// Signs one message `count` times under one key per curve with noise from `fill`, and
/// asserts that no `r` repeats.
fn assert_r_never_repeats(count: usize, mut fill: impl FnMut(&mut [u8])) {
    for curve in [CurveId::P256, CurveId::P384] {
        let (len, _) = params(curve);
        let key = SigningKey::from_bytes(curve, &vec![0x42; len]).unwrap();
        let mut seen = HashSet::with_capacity(count);
        for _ in 0..count {
            let sig = key
                .sign(b"same message", |h: &mut [u8]| {
                    fill(h);
                    Ok::<(), ()>(())
                })
                .unwrap();
            // Both curves' sequences fit a short-form length, so `r`'s header is at 2.
            let r_len = usize::from(sig[3]);
            assert!(
                seen.insert(sig[4..4 + r_len].to_vec()),
                "{curve:?}: r repeated"
            );
        }
    }
}

/// `r` never repeats over 2,000 signatures of one message per curve.
#[test]
fn hedged_nonces_do_not_repeat() {
    let mut lcg = Lcg(7);
    assert_r_never_repeats(2_000, |h| lcg.fill(h));
}

/// The plan's bar: `r` never repeats over 100,000 signatures of one message per curve, with
/// noise from the operating system. Slow; run with
/// `cargo test --release -p rusty_pk --test ecdsa_sign -- --ignored`.
#[test]
#[ignore = "slow: 100,000 signatures per curve"]
fn hedged_nonces_do_not_repeat_over_100k_signatures() {
    use ring::rand::SecureRandom;
    let rng = SystemRandom::new();
    assert_r_never_repeats(100_000, |h| rng.fill(h).unwrap());
}

/// Fixed noise through the public `sign` gives the expected signature (RFC 6979 key and
/// message "sample", draft-irtf-cfrg-det-sigs-with-noise-05 section 4). The expected values
/// come from an independent Python implementation of that construction (`hashlib`, `hmac`
/// and affine curve arithmetic), which also reproduces RFC 6979 A.2.5 and A.2.6.
#[test]
fn fixed_noise_known_answers() {
    const X256: &str = "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721";
    const X384: &str = "6B9D3DAD2E1B8C1C05B19875B6659F4DE23C3B667BF297BA9AA47740787137D896D5724E4C70A825F872C9EA60D2EDF5";
    let cases = [
        (CurveId::P256, X256, 0x00, "51C74722DB8FA584212E37CB39FA873FD7301602D3388B811DA72C9E9C404AC7", "D133481EE6027B10D1E35ABB95A735F0681A3EF65D893AA678AD3B47EC14D1C1"),
        (CurveId::P256, X256, 0xff, "CB51D5492E7CE49F6C2540A9CFDC2D052D4252990F30B34CC26214E7322383FC", "48115ABCF892AE0AFA33E01DAB584026D1A078B29FFF7E83C3D0223F046EBC59"),
        (CurveId::P256, X256, 0x01, "8F921763C1BAA901677FCE6DF8F6459A7E4D16D80EBD445EF9777C2F1E0517A4", "81D38968695B0309DFE50026D2456411D823E07E22E536D67236D5D5D08444DD"),
        (CurveId::P384, X384, 0x00, "D00D17F46D516AF375808F8F704B042E4BF30800B8914B08EC42AE9ACC2EA2B79C34D50CAF838C4045EC0E053E5CD307", "9E52326F5729276617E4D13C0845378957951C8E0BB0BFF3DA03BE2F7BC2FC66C9445238D2224C055D2B8DFACB8C7705"),
        (CurveId::P384, X384, 0xff, "A0C35F5ED18A6781817B3D3C5FDD32D0C087A4D097A91FE736CA3422968FBE9D7B25D559603068B2DCC188368A1809C9", "BF501EBA9C06205A3DF1464F27A9E3600CC5DA0BCB61F1C255265E2FCF247328BE40EB1CAFF41957DD73934EC9E3ABFD"),
        (CurveId::P384, X384, 0x01, "5BA4031E9F055551E5A3857B61F2D425F9A0D5356EF2C5FDC01C0D77A0E2DAED5A96988DD6EE3C94187E59C7C4A16B86", "B756A0B1E1B1C6C38818FAFB24B1F276B55CB0AC8CA9379A08BCBC00389DBCDF075F2340889A176DC5C96DF01B2E7E98"),
    ];
    for (curve, x, noise, want_r, want_s) in cases {
        let (len, digest) = params(curve);
        let key = SigningKey::from_bytes(curve, &hex(x)).unwrap();
        // 0x01 stands for the ramp 1, 2, ..., len; 0x00 and 0xff fill every byte.
        let fill = |h: &mut [u8]| {
            assert_eq!(h.len(), noise_len(curve));
            for (i, b) in h.iter_mut().enumerate() {
                *b = if noise == 0x01 { i as u8 + 1 } else { noise };
            }
            Ok::<(), ()>(())
        };
        let sig = key.sign(b"sample", fill).unwrap();
        assert_eq!(
            sig,
            der(&hex(want_r), &hex(want_s), len),
            "{curve:?} {noise:#x}"
        );
        assert_eq!(
            sig,
            key.sign(b"sample", fill).unwrap(),
            "fixed noise repeats"
        );
        let public = key.public_key().unwrap();
        assert!(verify(curve, digest, public.as_bytes(), b"sample", &sig).is_ok());
    }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// The minimal DER `SEQUENCE { r, s }` for fixed-width `r` and `s`.
fn der(r: &[u8], s: &[u8], len: usize) -> Vec<u8> {
    let int = |v: &[u8]| {
        let v = &v[v.iter().position(|&b| b != 0).unwrap_or(len - 1)..];
        let pad = v[0] & 0x80 != 0;
        let mut out = vec![0x02, (v.len() + usize::from(pad)) as u8];
        if pad {
            out.push(0);
        }
        out.extend_from_slice(v);
        out
    };
    let body = [int(r), int(s)].concat();
    [vec![0x30, body.len() as u8], body].concat()
}

#[test]
fn a_failed_random_source_signs_nothing() {
    let key = SigningKey::from_bytes(CurveId::P256, &[1; 32]).unwrap();
    let mut asked = 0;
    let result = key.sign(b"m", |h: &mut [u8]| {
        asked = h.len();
        Err("no entropy")
    });
    assert_eq!(result, Err(SignError::Random));
    assert_eq!(asked, noise_len(CurveId::P256));
}

#[test]
fn invalid_private_keys_are_refused() {
    const N256: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";
    let n: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&N256[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    for bad in [vec![0u8; 32], n, vec![0xff; 32], vec![1; 31], vec![1; 33]] {
        assert_eq!(
            SigningKey::from_bytes(CurveId::P256, &bad).unwrap_err(),
            SignError::InvalidKey
        );
    }
    assert!(SigningKey::from_bytes(CurveId::P384, &[1; 32]).is_err());
}

#[test]
fn debug_does_not_print_the_key() {
    let key = SigningKey::from_bytes(CurveId::P384, &[3; 48]).unwrap();
    assert_eq!(format!("{key:?}"), "ecdsa::SigningKey(..)");
}
