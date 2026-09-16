//! Coverage-guided fuzzing of the X.509 certificate parser.
//!
//! Beyond "does not panic", this checks the property that makes the parser
//! usable by a validator at all: every field it hands back must be a borrow
//! of the input it was given. A field pointing anywhere else would mean the
//! parser copied or synthesized bytes somewhere it documents that it does
//! not — which is exactly how an implementation ends up verifying a signature
//! over something other than what it parsed.
//!
//! Seed this with real certificates for it to be worth much:
//!
//!   mkdir -p fuzz/corpus/certificate
//!   # drop DER-encoded certificates in, e.g. from /etc/ssl/certs
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run certificate

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::x509::Certificate;

fn within(inner: &[u8], outer: &[u8]) -> bool {
    let (i, o) = (inner.as_ptr_range(), outer.as_ptr_range());
    i.start >= o.start && i.end <= o.end
}

fuzz_target!(|data: &[u8]| {
    let Ok(cert) = Certificate::parse(data) else {
        return;
    };

    for field in [
        cert.tbs_der(),
        cert.serial(),
        cert.issuer(),
        cert.subject(),
        cert.signature(),
        cert.subject_public_key_info().encoded,
        cert.subject_public_key_info().key,
        cert.signature_algorithm().encoded,
    ] {
        assert!(within(field, data), "a parsed field escaped its input");
    }

    assert!(!cert.tbs_der().is_empty());
    assert!(!cert.serial().is_empty());
    assert!(cert.serial() == [0] || cert.serial()[0] != 0);
    assert_eq!(cert.issuer()[0], 0x30);
    assert_eq!(cert.subject()[0], 0x30);

    // Both iterators must terminate on attacker-chosen input. A `Reader`
    // leaves a wrong-tagged value unconsumed so that OPTIONAL fields work,
    // which once made these spin forever — libFuzzer reports that as a
    // timeout rather than a crash, so the bound is asserted explicitly too.
    let mut names = 0;
    for name in cert.extensions().subject_alt_names() {
        names += 1;
        assert!(names < 100_000, "the SAN iterator did not terminate");
        if let Ok(name) = name {
            let _ = format!("{name:?}");
        }
    }
    let mut purposes = 0;
    for _ in cert.extensions().extended_key_usage() {
        purposes += 1;
        assert!(purposes < 100_000, "the EKU iterator did not terminate");
    }
    for oid in cert.extensions().unhandled_critical() {
        let _ = format!("{oid:?}");
    }
    let _ = format!("{:?}", cert.extensions().key_usage());
    let _ = cert.extensions().basic_constraints();
    let _ = cert.is_self_issued();
});
