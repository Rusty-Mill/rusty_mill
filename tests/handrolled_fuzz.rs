//! Fuzzing the DER reader and certificate parser, on stable, in CI.
//!
//! ADR-0002 lists "fuzz the parsers" as a shipping-bar item and stage 2a
//! landed owing it. This is that debt, in the form that can actually run on
//! every pull request: `cargo-fuzz` needs nightly and sustained runtime,
//! neither of which a per-PR check has, so a coverage-guided fuzzer is a
//! thing someone runs deliberately rather than a thing that protects a
//! branch. `fuzz/` holds those targets; this file is what stops a regression
//! from landing in between.
//!
//! # Why this is not just "call the parser with random bytes"
//!
//! Random bytes are rejected by the first tag octet roughly 99% of the time,
//! so a random-byte fuzzer tests the early-return path over and over and
//! almost never reaches the code where the interesting bugs are. Two things
//! fix that here:
//!
//! - **A real seed corpus.** The machine's own trust anchors, plus generated
//!   certificates, are mutated rather than replaced — a single flipped byte
//!   in a real certificate is still 99.9% a real certificate, so it reaches
//!   deep into the parser before anything goes wrong.
//! - **A measured reach.** [`mutated_certificates_reach_deep_into_the_parser`]
//!   asserts that a meaningful fraction of mutants still parse. Without that,
//!   a change that made every input fail at the first byte would leave this
//!   whole file passing while testing nothing.
//!
//! # What is actually asserted
//!
//! Not merely "does not panic", though that is checked everywhere and is what
//! a fuzzer is usually for. The stronger property is canonicality:
//!
//! > **If the reader accepts a value, re-encoding that value's tag and
//! > contents with a minimal length must reproduce the accepted bytes
//! > exactly.**
//!
//! That is the whole promise of `der.rs` stated as a testable invariant. A
//! reader that let through a non-minimal length, an indefinite length, or a
//! high-tag-number form would produce a `Value` whose `encoded` is longer
//! than its canonical form, and this fails — for every input, not just the
//! ones someone thought to write a case for.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use platform::security::TrustAnchors;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose, SanType};
use rusty_tls::handrolled::der::{Reader, Tag};
use rusty_tls::handrolled::x509::Certificate;

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------

/// xorshift64*, so a failure is reproducible from the seed printed with it.
///
/// A fuzzer that cannot reproduce its own findings is a fuzzer that reports
/// bugs nobody can fix.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }

    fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }
}

/// Iteration counts are modest by default so a pull request is not waiting on
/// them, and raisable for a deliberate longer run:
///
/// ```text
/// RUSTY_TLS_FUZZ_ITERATIONS=2000000 cargo test --features handrolled-engine
/// ```
fn iterations(default: usize) -> usize {
    std::env::var("RUSTY_TLS_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// The invariants
// ---------------------------------------------------------------------------

/// Minimal-length DER encoding of one value — the canonical form.
fn canonical_encode(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = contents.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let first = bytes.iter().position(|&b| b != 0).expect("len is non-zero");
        out.push(0x80 | (bytes.len() - first) as u8);
        out.extend_from_slice(&bytes[first..]);
    }
    out.extend_from_slice(contents);
    out
}

fn is_subslice(inner: &[u8], outer: &[u8]) -> bool {
    let (inner_range, outer_range) = (inner.as_ptr_range(), outer.as_ptr_range());
    inner_range.start >= outer_range.start && inner_range.end <= outer_range.end
}

/// Drive the reader over every value reachable in `input`, checking each
/// promise it makes. Returns how many values were successfully read, which
/// callers use to tell "the parser was exercised" from "the parser bailed on
/// byte one".
///
/// Iterative, with an explicit work stack: the reader is non-recursive by
/// design, and a recursive checker would reintroduce exactly the stack
/// exhaustion the design avoids — the test would crash on deeply nested input
/// and be blamed on the code.
fn check_der_invariants(input: &[u8]) -> usize {
    let mut work: Vec<&[u8]> = vec![input];
    let mut values_read = 0usize;

    while let Some(region) = work.pop() {
        let mut reader = Reader::new(region);
        while !reader.is_empty() {
            let before = reader.remaining();
            let Ok(value) = reader.read_any() else {
                break;
            };
            values_read += 1;

            assert!(
                is_subslice(value.encoded, input),
                "a value's encoded bytes escaped the input"
            );
            assert!(
                is_subslice(value.contents, value.encoded),
                "a value's contents escaped its own encoding"
            );
            assert!(
                value.encoded.len() > value.contents.len(),
                "a value with no header"
            );
            assert_eq!(
                before - reader.remaining(),
                value.encoded.len(),
                "the cursor moved by something other than the value's length"
            );

            // The property this whole module exists to provide.
            assert_eq!(
                canonical_encode(value.tag.0, value.contents),
                value.encoded,
                "a non-canonical encoding was accepted: tag 0x{:02x}, {} content bytes",
                value.tag.0,
                value.contents.len()
            );

            // Descend into constructed values, without recursing.
            if value.tag.is_constructed() && values_read < 200_000 {
                work.push(value.contents);
            }
        }
    }

    values_read
}

/// The typed readers, each checked for the shape it promises to return.
fn check_typed_readers(input: &[u8]) {
    if let Ok(magnitude) = Reader::new(input).read_unsigned_integer() {
        assert!(is_subslice(magnitude, input));
        assert!(!magnitude.is_empty(), "an empty INTEGER magnitude");
        assert!(
            magnitude == [0] || magnitude[0] != 0,
            "a leading zero survived an unsigned INTEGER: {magnitude:?}"
        );
        assert!(
            magnitude[0] & 0x80 == 0 || magnitude.len() > 1 || magnitude == [0],
            "a negative INTEGER was returned as unsigned"
        );
    }

    if let Ok(oid) = Reader::new(input).read_oid() {
        let bytes = oid.as_bytes();
        assert!(!bytes.is_empty(), "an empty OID");
        assert!(
            bytes[bytes.len() - 1] & 0x80 == 0,
            "an OID ending mid-subidentifier"
        );
        // Rendering must not panic on anything the reader accepted.
        let _ = format!("{oid:?}");
    }

    if let Ok((bits, unused)) = Reader::new(input).read_bit_string_flags() {
        assert!(unused <= 7, "more than seven unused bits");
        assert!(is_subslice(bits, input));
        assert!(!bits.is_empty() || unused == 0);
    }

    if let Ok(octets) = Reader::new(input).read_bit_string_octets() {
        assert!(is_subslice(octets, input));
    }

    // These must simply never panic.
    let _ = Reader::new(input).read_bool();
    let _ = Reader::new(input).read_u64();
    let _ = Reader::new(input).read_null();
    let _ = Reader::new(input).read(Tag::SEQUENCE);
    let _ = Reader::new(input).read_optional(Tag::BOOLEAN);
    let _ = Reader::new(input).read_sequence();
    let _ = Reader::new(input).read_set();
}

/// Certificate parsing must never panic, and anything it accepts must be
/// coherent — every borrowed field pointing into the input it came from.
///
/// Returns whether the input parsed, so callers can measure reach.
fn check_certificate_invariants(input: &[u8]) -> bool {
    let Ok(cert) = Certificate::parse(input) else {
        return false;
    };

    for (name, field) in [
        ("tbs_der", cert.tbs_der()),
        ("serial", cert.serial()),
        ("issuer", cert.issuer()),
        ("subject", cert.subject()),
        ("signature", cert.signature()),
        ("spki", cert.subject_public_key_info().encoded),
        ("spki key", cert.subject_public_key_info().key),
        ("signature algorithm", cert.signature_algorithm().encoded),
    ] {
        assert!(
            is_subslice(field, input),
            "{name} does not point into the certificate it was parsed from"
        );
    }

    assert!(!cert.tbs_der().is_empty(), "an empty tbsCertificate");
    assert!(!cert.serial().is_empty(), "an empty serial number");
    assert!(
        cert.serial() == [0] || cert.serial()[0] != 0,
        "a serial number kept its DER sign octet"
    );
    assert_eq!(cert.issuer()[0], 0x30, "issuer is not a SEQUENCE");
    assert_eq!(cert.subject()[0], 0x30, "subject is not a SEQUENCE");

    // The iterators must terminate and must not panic, whatever is in them.
    let mut names = 0;
    for name in cert.extensions().subject_alt_names() {
        names += 1;
        assert!(names < 100_000, "the SAN iterator did not terminate");
        if let Ok(name) = name {
            let _ = format!("{name:?}");
        }
    }
    let mut purposes = 0;
    for purpose in cert.extensions().extended_key_usage() {
        purposes += 1;
        assert!(purposes < 100_000, "the EKU iterator did not terminate");
        let _ = purpose;
    }
    for oid in cert.extensions().unhandled_critical() {
        let _ = format!("{oid:?}");
    }
    let _ = format!("{:?}", cert.extensions().key_usage());

    true
}

fn check_everything(input: &[u8]) -> (usize, bool) {
    let values = check_der_invariants(input);
    check_typed_readers(input);
    let parsed = check_certificate_invariants(input);
    (values, parsed)
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

fn load_anchors() -> Vec<Vec<u8>> {
    #[cfg(target_os = "linux")]
    let backend = platform_linux::LinuxTrustAnchors;
    #[cfg(windows)]
    let backend = platform_windows::WindowsTrustAnchors;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    let backend = platform_bsd::BsdTrustAnchors;

    backend.load_anchors().unwrap_or_default()
}

/// Real trust anchors, plus generated certificates covering shapes a trust
/// store does not carry — a leaf with several SAN forms, a constrained CA.
fn seed_corpus() -> Vec<Vec<u8>> {
    let mut corpus = load_anchors();

    let mut leaf = CertificateParams::new(vec!["example.com".to_string()]).expect("params");
    leaf.subject_alt_names = vec![
        SanType::DnsName("example.com".try_into().unwrap()),
        SanType::IpAddress("192.0.2.1".parse().unwrap()),
        SanType::IpAddress("2001:db8::1".parse().unwrap()),
    ];
    let mut ca = CertificateParams::new(Vec::<String>::new()).expect("params");
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(2));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    for params in [leaf, ca] {
        let key = KeyPair::generate().expect("key");
        corpus.push(params.self_signed(&key).expect("cert").der().to_vec());
    }

    corpus
}

/// One mutation of `seed`. Deliberately small: the point of a seed corpus is
/// that a mutant is still *nearly* a certificate, so it reaches deep before
/// it goes wrong.
fn mutate(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut out = seed.to_vec();
    if out.is_empty() {
        return out;
    }

    match rng.below(7) {
        0 => {
            // Flip one bit.
            let index = rng.below(out.len());
            out[index] ^= 1 << rng.below(8);
        }
        1 => {
            // Replace one byte outright.
            let index = rng.below(out.len());
            out[index] = rng.byte();
        }
        2 => {
            // Truncate. Length fields now disagree with reality, which is the
            // most common malformation in the wild.
            out.truncate(rng.below(out.len()));
        }
        3 => {
            // Append, so something trails a complete certificate.
            for _ in 0..1 + rng.below(8) {
                out.push(rng.byte());
            }
        }
        4 => {
            // Overwrite a short run — likely to land inside a length or tag.
            let start = rng.below(out.len());
            let end = (start + 1 + rng.below(8)).min(out.len());
            for byte in &mut out[start..end] {
                *byte = rng.byte();
            }
        }
        5 => {
            // Swap two bytes, preserving length.
            let (a, b) = (rng.below(out.len()), rng.below(out.len()));
            out.swap(a, b);
        }
        _ => {
            // Several bit flips at once, to escape a local plateau.
            for _ in 0..1 + rng.below(4) {
                let index = rng.below(out.len());
                out[index] ^= 1 << rng.below(8);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// Uniformly random bytes. Cheap, and almost all of it is rejected
/// immediately — which is why it is the least valuable test here and not the
/// only one.
#[test]
fn random_bytes_never_break_an_invariant() {
    let mut rng = Rng::new(0x5eed_0001);
    let rounds = iterations(30_000);

    for round in 0..rounds {
        let len = rng.below(256);
        let input: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        // Any panic below is reported with the round number, and the seed is
        // fixed, so a failure replays exactly.
        let _ = std::panic::catch_unwind(|| check_everything(&input))
            .unwrap_or_else(|_| panic!("invariant broken at round {round} on {input:02x?}"));
    }
}

/// Random bytes with a plausible DER shape — a valid tag and a valid length,
/// wrapping arbitrary contents. Reaches past the first octet far more often
/// than uniform noise.
#[test]
fn structured_der_never_breaks_an_invariant() {
    let mut rng = Rng::new(0x5eed_0002);
    let rounds = iterations(30_000);
    const TAGS: [u8; 10] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x17, 0x18, 0x30, 0x31];

    for round in 0..rounds {
        // Build one to three nested levels of well-formed framing around
        // random contents.
        let mut payload: Vec<u8> = (0..rng.below(64)).map(|_| rng.byte()).collect();
        for _ in 0..1 + rng.below(3) {
            let tag = TAGS[rng.below(TAGS.len())];
            payload = canonical_encode(tag, &payload);
        }
        // Then corrupt one byte, so the framing is nearly-but-not-quite right.
        if rng.below(2) == 0 && !payload.is_empty() {
            let index = rng.below(payload.len());
            payload[index] = rng.byte();
        }

        let input = payload;
        let _ = std::panic::catch_unwind(|| check_everything(&input))
            .unwrap_or_else(|_| panic!("invariant broken at round {round} on {input:02x?}"));
    }
}

/// The one that matters: mutations of real certificates.
#[test]
fn mutated_certificates_never_break_an_invariant() {
    let corpus = seed_corpus();
    assert!(
        corpus.len() >= 10,
        "corpus of {} is too small to fuzz against",
        corpus.len()
    );

    let mut rng = Rng::new(0x5eed_0003);
    let rounds = iterations(20_000);

    for round in 0..rounds {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);
        let _ = std::panic::catch_unwind(|| check_everything(&input)).unwrap_or_else(|_| {
            panic!(
                "invariant broken at round {round} on a {}-byte mutant: {input:02x?}",
                input.len()
            )
        });
    }
}

/// A fuzzer that never gets past the front door is not fuzzing anything.
///
/// This measures how far the mutants actually reach, and fails if the answer
/// is "nowhere" — which is what a change that broke parsing outright would
/// look like, and which would otherwise leave every test above passing.
#[test]
fn mutated_certificates_reach_deep_into_the_parser() {
    let corpus = seed_corpus();
    let mut rng = Rng::new(0x5eed_0004);
    let rounds = iterations(20_000);

    let (mut parsed, mut deep) = (0usize, 0usize);
    for _ in 0..rounds {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);
        let (values, ok) = check_everything(&input);
        if ok {
            parsed += 1;
        }
        // A real certificate decomposes into dozens of DER values; reaching
        // ten means the mutant survived well past the outer framing.
        if values >= 10 {
            deep += 1;
        }
    }

    let parsed_pct = parsed * 100 / rounds;
    let deep_pct = deep * 100 / rounds;
    println!("of {rounds} mutants: {parsed_pct}% parsed as certificates, {deep_pct}% reached ten or more DER values");

    assert!(
        parsed_pct >= 5,
        "only {parsed_pct}% of mutants parsed — the fuzzer is testing the \
         early-reject path and almost nothing else"
    );
    assert!(
        deep_pct >= 50,
        "only {deep_pct}% of mutants reached ten DER values — mutations are \
         being rejected too early to exercise the parser"
    );
}

/// Parsing must be a function of its input and nothing else. A parser whose
/// answer depends on hidden state is one whose test results say nothing about
/// production.
#[test]
fn parsing_the_same_bytes_twice_gives_the_same_answer() {
    let corpus = seed_corpus();
    let mut rng = Rng::new(0x5eed_0005);

    for _ in 0..iterations(5_000) {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);

        let first = Certificate::parse(&input);
        let second = Certificate::parse(&input);
        match (first, second) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.tbs_der(), b.tbs_der());
                assert_eq!(a.serial(), b.serial());
                assert_eq!(a.issuer(), b.issuer());
                assert_eq!(a.subject(), b.subject());
                assert_eq!(a.validity(), b.validity());
                assert_eq!(a.version(), b.version());
            }
            (Err(a), Err(b)) => assert_eq!(a, b),
            _ => panic!("parsing the same {} bytes twice disagreed", input.len()),
        }
    }
}

/// Every unmutated seed must parse, or the corpus is not what the tests above
/// assume it is and their reach numbers mean nothing.
#[test]
fn the_unmutated_corpus_parses() {
    let corpus = seed_corpus();
    let failures: Vec<String> = corpus
        .iter()
        .filter_map(|der| Certificate::parse(der).err().map(|e| format!("  {e}")))
        .collect();

    assert!(
        failures.is_empty(),
        "{} of {} seeds do not parse:\n{}",
        failures.len(),
        corpus.len(),
        failures.join("\n")
    );
}
