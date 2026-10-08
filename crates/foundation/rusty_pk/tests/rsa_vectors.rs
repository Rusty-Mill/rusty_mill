//! RSA verification against Wycheproof and `ring`.

use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use rusty_ct_check::wycheproof::{cases, Verdict};
use rusty_pk::rsa::{verify, Scheme};

macro_rules! vectors {
    ($name:literal) => {
        include_str!(concat!("vectors/", $name, ".json"))
    };
}

fn ring_alg(scheme: Scheme) -> &'static dyn VerificationAlgorithm {
    match scheme {
        Scheme::Pkcs1Sha256 => &signature::RSA_PKCS1_2048_8192_SHA256,
        Scheme::Pkcs1Sha384 => &signature::RSA_PKCS1_2048_8192_SHA384,
        Scheme::Pkcs1Sha512 => &signature::RSA_PKCS1_2048_8192_SHA512,
        Scheme::PssSha256 => &signature::RSA_PSS_2048_8192_SHA256,
        Scheme::PssSha384 => &signature::RSA_PSS_2048_8192_SHA384,
        Scheme::PssSha512 => &signature::RSA_PSS_2048_8192_SHA512,
    }
}

/// Runs every case. `native_profile` says whether the vector's own parameters
/// match what we (and ring) implement; if not, the signature must be rejected
/// whatever Wycheproof says about the vector's parameters.
fn run(
    json: &str,
    scheme: Scheme,
    native_profile: impl Fn(&rusty_ct_check::wycheproof::Case) -> bool,
) {
    let all = cases(json).unwrap();
    assert!(all.len() > 50, "file looks truncated");
    for case in all {
        let key = case.hex("publicKeyAsn").unwrap();
        let msg = case.hex("msg").unwrap();
        let sig = case.hex("sig").unwrap();
        let ours = verify(scheme, &key, &msg, &sig).is_ok();
        let ring = UnparsedPublicKey::new(ring_alg(scheme), &key)
            .verify(&msg, &sig)
            .is_ok();
        assert_eq!(
            ours, ring,
            "differs from ring: tcId {} ({})",
            case.id, case.comment
        );
        if native_profile(&case) {
            match case.verdict {
                Verdict::Valid => {
                    assert!(ours, "tcId {} should verify ({})", case.id, case.comment)
                }
                Verdict::Invalid => {
                    assert!(!ours, "tcId {} must reject ({})", case.id, case.comment)
                }
                // Wycheproof marks e.g. BER-ish leniencies "acceptable"; ring
                // is the reference for those and the equality above covers it.
                Verdict::Acceptable => {}
            }
        } else if case.verdict == Verdict::Valid {
            // Valid only under parameters we do not implement, so it cannot
            // verify under ours. An *invalid* vector may still verify: tcId 69
            // of the salt-0 file is a salt-32 signature, which is our profile.
            // Equality with ring above is the check there.
            assert!(
                !ours,
                "tcId {}: non-native parameters must be rejected",
                case.id
            );
        }
    }
}

fn pkcs1(json: &str, scheme: Scheme) {
    run(json, scheme, |_| true);
}

#[test]
fn pkcs1_sha256() {
    pkcs1(
        vectors!("rsa_signature_2048_sha256_test"),
        Scheme::Pkcs1Sha256,
    );
    pkcs1(
        vectors!("rsa_signature_3072_sha256_test"),
        Scheme::Pkcs1Sha256,
    );
    pkcs1(
        vectors!("rsa_signature_4096_sha256_test"),
        Scheme::Pkcs1Sha256,
    );
}

#[test]
fn pkcs1_sha384() {
    pkcs1(
        vectors!("rsa_signature_2048_sha384_test"),
        Scheme::Pkcs1Sha384,
    );
    pkcs1(
        vectors!("rsa_signature_3072_sha384_test"),
        Scheme::Pkcs1Sha384,
    );
    pkcs1(
        vectors!("rsa_signature_4096_sha384_test"),
        Scheme::Pkcs1Sha384,
    );
}

#[test]
fn pkcs1_sha512() {
    pkcs1(
        vectors!("rsa_signature_2048_sha512_test"),
        Scheme::Pkcs1Sha512,
    );
    pkcs1(
        vectors!("rsa_signature_3072_sha512_test"),
        Scheme::Pkcs1Sha512,
    );
    pkcs1(
        vectors!("rsa_signature_4096_sha512_test"),
        Scheme::Pkcs1Sha512,
    );
}

fn pss_native(case: &rusty_ct_check::wycheproof::Case, sha: &str, salt: u64) -> bool {
    case.str("sha").unwrap() == sha
        && case.str("mgfSha").unwrap() == sha
        && case.uint("sLen").unwrap() == salt
}

#[test]
fn pss_sha256() {
    let native = |c: &_| pss_native(c, "SHA-256", 32);
    run(
        vectors!("rsa_pss_2048_sha256_mgf1_32_test"),
        Scheme::PssSha256,
        native,
    );
    run(
        vectors!("rsa_pss_3072_sha256_mgf1_32_test"),
        Scheme::PssSha256,
        native,
    );
    run(
        vectors!("rsa_pss_4096_sha256_mgf1_32_test"),
        Scheme::PssSha256,
        native,
    );
    // Salt length 0 is not our profile: every signature there must be rejected.
    run(
        vectors!("rsa_pss_2048_sha256_mgf1_0_test"),
        Scheme::PssSha256,
        native,
    );
}

#[test]
fn pss_sha512() {
    run(
        vectors!("rsa_pss_4096_sha512_mgf1_32_test"),
        Scheme::PssSha512,
        |c| pss_native(c, "SHA-512", 64),
    );
}

#[test]
fn pss_misc_parameters_follow_the_native_profile() {
    for (scheme, sha, salt) in [
        (Scheme::PssSha256, "SHA-256", 32),
        (Scheme::PssSha384, "SHA-384", 48),
        (Scheme::PssSha512, "SHA-512", 64),
    ] {
        run(vectors!("rsa_pss_misc_test"), scheme, |c| {
            pss_native(c, sha, salt)
        });
    }
}
