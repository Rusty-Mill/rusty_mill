//! Fuzzing the parsers and verifiers — DER, X.509, the TLS handshake, key
//! exchange, and handshake signatures — on stable, in CI.
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
//!
//! # The handshake half
//!
//! Stage 3b added a second parser that reads attacker-supplied bytes, so it
//! gets the same treatment, seeded from RFC 8448's real exchange. Its
//! invariant is the same shape as the DER one and matters for the same
//! reason:
//!
//! > **If a message parses, re-encoding it must reproduce the accepted body
//! > exactly.**
//!
//! The transcript hash covers encoded messages. A parser and encoder that
//! disagree on any input compute a transcript the peer does not share, and a
//! parser that *normalises* while re-encoding hashes something nobody sent —
//! the same class of bug as re-encoding a certificate before checking its
//! signature.
//!
//! Plus one framing invariant that is purely about not being lied to: the
//! spans `messages()` returns must tile their input exactly, with no gap, no
//! overlap, and nothing pointing outside.
//!
//! # The verifiers
//!
//! Stage 3c-i added two more places attacker-controlled bytes arrive: a
//! `SignatureScheme` is a `uint16` a peer picks, and a `key_share` is a byte
//! string a peer picks. Their invariant is blunt and is the only one that
//! matters:
//!
//! > **Nothing an attacker can choose alone makes a verifier return `Ok`.**
//!
//! A random signature under a random scheme against a real key must be
//! refused every time. So must a random key share. Neither may panic — a
//! panic in a verifier is a denial of service reachable from the first
//! handshake flight, which is the same bug class the SAN iterator had.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use platform::security::TrustAnchors;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose, SanType};
use rusty_tls::handrolled::der::{Reader, Tag};
use rusty_tls::handrolled::handshake::{
    messages, parse_encrypted_extensions, parse_finished, CertificateMessage, CertificateVerify,
    ClientHello, HandshakeType, Message, ServerHello,
};
use rusty_tls::handrolled::kx::{KeyExchange, NamedGroup};
use rusty_tls::handrolled::verify::{verify_tls13_signature, SignatureScheme};
use rusty_tls::handrolled::x509::Certificate;

mod rfc8448;

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
// The handshake invariants
// ---------------------------------------------------------------------------

/// Drive the handshake parsers over `input`, checking every promise they
/// make. Returns how many messages were framed and how many bodies parsed, so
/// callers can measure reach the same way the certificate tests do.
fn check_handshake_invariants(input: &[u8]) -> (usize, usize) {
    // The type mapping must be total and lossless, whatever byte arrives.
    for byte in input.iter().copied() {
        assert_eq!(
            HandshakeType::from_u8(byte).as_u8(),
            byte,
            "the handshake type mapping lost a value"
        );
    }

    let Ok(parsed) = messages(input) else {
        return (0, 0);
    };

    // The spans must tile the input: no gap, no overlap, nothing outside.
    let mut offset = 0usize;
    for message in &parsed {
        assert!(
            is_subslice(message.encoded, input),
            "an encoded span escaped its input"
        );
        assert!(
            is_subslice(message.body, message.encoded),
            "a body escaped its own message"
        );
        assert_eq!(
            message.encoded.len(),
            message.body.len() + 4,
            "a message without a four-octet header"
        );
        assert_eq!(&message.encoded[4..], message.body);

        let start = message.encoded.as_ptr() as usize - input.as_ptr() as usize;
        assert_eq!(start, offset, "the message spans do not tile the input");
        offset += message.encoded.len();

        // Framing round-trips for every message, parseable body or not.
        assert_eq!(
            Message::encode(message.typ, message.body),
            message.encoded,
            "re-framing a message did not reproduce it"
        );
    }
    assert_eq!(offset, input.len(), "the spans left a tail unaccounted for");

    let mut bodies = 0usize;
    for message in &parsed {
        // The property this whole section exists for: parse and encode are
        // inverses on anything accepted, because the transcript hashes the
        // bytes and a disagreement is a handshake nobody can complete.
        match message.typ {
            HandshakeType::ClientHello => {
                if let Ok(hello) = ClientHello::parse(message.body) {
                    assert_eq!(
                        hello.encode(),
                        message.body,
                        "ClientHello did not round-trip"
                    );
                    bodies += 1;
                }
            }
            HandshakeType::ServerHello => {
                if let Ok(hello) = ServerHello::parse(message.body) {
                    assert_eq!(
                        hello.encode(),
                        message.body,
                        "ServerHello did not round-trip"
                    );
                    // A HelloRetryRequest is decided by `random` alone, and
                    // asking must never panic on a short or odd one.
                    let _ = hello.is_hello_retry_request();
                    bodies += 1;
                }
            }
            HandshakeType::Certificate => {
                if let Ok(certificate) = CertificateMessage::parse(message.body) {
                    assert_eq!(
                        certificate.encode(),
                        message.body,
                        "Certificate did not round-trip"
                    );
                    // Entries feed the stage 2a parser; it must not panic on
                    // whatever this one accepted.
                    for entry in &certificate.entries {
                        assert!(is_subslice(entry.certificate, input));
                        let _ = Certificate::parse(entry.certificate);
                    }
                    bodies += 1;
                }
            }
            HandshakeType::CertificateVerify => {
                if let Ok(verify) = CertificateVerify::parse(message.body) {
                    assert_eq!(
                        verify.encode(),
                        message.body,
                        "CertificateVerify did not round-trip"
                    );
                    assert!(!verify.signature.is_empty(), "an empty signature");
                    bodies += 1;
                }
            }
            HandshakeType::EncryptedExtensions => {
                if let Ok(extensions) = parse_encrypted_extensions(message.body) {
                    for extension in &extensions {
                        assert!(is_subslice(extension.data, input));
                    }
                    // Duplicates are refused, so the types must be distinct.
                    let mut types: Vec<u16> = extensions.iter().map(|e| e.typ).collect();
                    let before = types.len();
                    types.sort_unstable();
                    types.dedup();
                    assert_eq!(before, types.len(), "a duplicate extension was accepted");
                    bodies += 1;
                }
            }
            HandshakeType::Finished => {
                if let Ok(verify_data) = parse_finished(message.body) {
                    assert!(!verify_data.is_empty(), "an empty verify_data");
                    bodies += 1;
                }
            }
            _ => {}
        }
    }

    (parsed.len(), bodies)
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

// ---------------------------------------------------------------------------
// The handshake tests
// ---------------------------------------------------------------------------

/// Every prefix and concatenation of RFC 8448's exchange, plus the exchange
/// itself — the shapes a real reassembly buffer holds part-way through a
/// handshake.
fn handshake_corpus() -> Vec<Vec<u8>> {
    let mut corpus = rfc8448::all_messages();

    // The whole exchange in order, which is what a client that buffered
    // everything would hand to `messages()`.
    let whole: Vec<u8> = corpus.iter().flatten().copied().collect();
    corpus.push(whole);

    corpus
}

/// Uniformly random bytes through the handshake parsers. Cheap and shallow —
/// a random first octet is a valid `HandshakeType` every time, but the
/// `uint24` that follows almost never matches the buffer.
#[test]
fn random_bytes_never_break_a_handshake_invariant() {
    let mut rng = Rng::new(0x5eed_0006);

    for round in 0..iterations(30_000) {
        let len = rng.below(256);
        let input: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let _ = std::panic::catch_unwind(|| check_handshake_invariants(&input))
            .unwrap_or_else(|_| panic!("invariant broken at round {round} on {input:02x?}"));
    }
}

/// The one that matters: mutations of RFC 8448's real messages.
///
/// A single flipped byte in a real ClientHello is still a ClientHello, so it
/// reaches past the framing and into the extension loop, the cipher-suite
/// vector, and the compression check — where a round-trip bug would live.
#[test]
fn mutated_handshake_messages_never_break_an_invariant() {
    let corpus = handshake_corpus();
    let mut rng = Rng::new(0x5eed_0007);

    for round in 0..iterations(30_000) {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);
        let _ =
            std::panic::catch_unwind(|| check_handshake_invariants(&input)).unwrap_or_else(|_| {
                panic!(
                    "invariant broken at round {round} on a {}-byte mutant: {input:02x?}",
                    input.len()
                )
            });
    }
}

/// The reach check, for the same reason the certificate one exists: a change
/// that made every handshake input fail at the first octet would leave both
/// tests above passing while testing nothing at all.
#[test]
fn mutated_handshake_messages_reach_past_the_framing() {
    let corpus = handshake_corpus();
    let mut rng = Rng::new(0x5eed_0008);
    let rounds = iterations(30_000);

    let (mut framed, mut deep) = (0usize, 0usize);
    for _ in 0..rounds {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);
        let (frames, bodies) = check_handshake_invariants(&input);
        if frames > 0 {
            framed += 1;
        }
        if bodies > 0 {
            deep += 1;
        }
    }

    let framed_pct = framed * 100 / rounds;
    let deep_pct = deep * 100 / rounds;
    println!(
        "of {rounds} mutants: {framed_pct}% framed into messages, {deep_pct}% had a body parse"
    );

    assert!(
        framed_pct >= 20,
        "only {framed_pct}% of mutants framed — the fuzzer is testing the \
         length check and almost nothing else"
    );
    assert!(
        deep_pct >= 10,
        "only {deep_pct}% of mutants reached a message body — mutations are \
         being rejected before any parser runs"
    );
}

/// Handshake parsing must be a function of its input, like certificate
/// parsing, and for the same reason.
#[test]
fn parsing_the_same_handshake_bytes_twice_gives_the_same_answer() {
    let corpus = handshake_corpus();
    let mut rng = Rng::new(0x5eed_0009);

    for _ in 0..iterations(5_000) {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(&mut rng, seed);

        match (messages(&input), messages(&input)) {
            (Ok(a), Ok(b)) => assert_eq!(a, b),
            (Err(a), Err(b)) => assert_eq!(a, b),
            _ => panic!("framing the same {} bytes twice disagreed", input.len()),
        }
    }
}

/// Every unmutated handshake seed must frame and its bodies must parse, or
/// the reach numbers above mean nothing.
#[test]
fn the_unmutated_handshake_corpus_parses() {
    for (index, seed) in handshake_corpus().iter().enumerate() {
        let (frames, bodies) = check_handshake_invariants(seed);
        assert!(frames > 0, "seed {index} did not frame into any message");
        assert!(bodies > 0, "seed {index} framed but no body parsed");
    }

    // The whole exchange is seven messages: CH, SH, then the server's four,
    // then the client's Finished.
    let whole: Vec<u8> = rfc8448::all_messages().into_iter().flatten().collect();
    assert_eq!(messages(&whole).expect("the exchange frames").len(), 7);
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

// ---------------------------------------------------------------------------
// The verifiers — stage 3c-i
// ---------------------------------------------------------------------------

/// A real certificate to verify against, so the refusals below are refusals of
/// the *signature* rather than of a nonsense key.
fn verification_key() -> Vec<u8> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keypair");
    let params = rcgen::CertificateParams::new(vec!["fuzz.example".to_string()]).expect("params");
    params.self_signed(&key).expect("self-sign").der().to_vec()
}

/// No `SignatureScheme` a peer can name, with a signature a peer can choose,
/// verifies against a key the peer does not hold.
///
/// Every scheme number is walked, not sampled: there are only 65 536 of them,
/// and an accidental catch-all arm — a `_ => Ok(...)` in the wrong place —
/// would be invisible to a sampling test and fatal in production.
#[test]
fn no_scheme_and_no_random_signature_ever_verifies() {
    let certificate_der = verification_key();
    let certificate = Certificate::parse(&certificate_der).expect("parses");
    let key = certificate.subject_public_key_info();

    let mut rng = Rng::new(0x5eed_000a);
    let message = b"a message nobody signed";

    // Every scheme in the registry's range, with one random signature each.
    for value in 0u32..=u32::from(u16::MAX) {
        let length = 1 + rng.below(80);
        let signature: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
        let result =
            verify_tls13_signature(SignatureScheme(value as u16), &key, message, &signature);
        assert!(
            result.is_err(),
            "scheme 0x{value:04x} accepted a random {length}-byte signature"
        );
    }
}

/// Mutated certificates, real schemes, random signatures. The key itself is
/// hostile here, which the test above deliberately kept clean.
#[test]
fn a_mutated_key_never_makes_a_signature_verify() {
    let certificate_der = verification_key();
    let mut rng = Rng::new(0x5eed_000b);
    let rounds = iterations(20_000);
    let mut reached = 0usize;

    for round in 0..rounds {
        let mutant = mutate(&mut rng, &certificate_der);
        let Ok(certificate) = Certificate::parse(&mutant) else {
            continue;
        };
        reached += 1;
        let key = certificate.subject_public_key_info();

        let scheme =
            SignatureScheme::TLS13_SUPPORTED[rng.below(SignatureScheme::TLS13_SUPPORTED.len())];
        let signature: Vec<u8> = (0..1 + rng.below(80)).map(|_| rng.byte()).collect();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            verify_tls13_signature(scheme, &key, b"unsigned", &signature)
        }))
        .unwrap_or_else(|_| panic!("panicked at round {round} on {mutant:02x?}"));

        assert!(result.is_err(), "a mutated key verified a random signature");
    }

    assert!(
        reached * 100 / rounds >= 5,
        "only {reached} of {rounds} mutants parsed — the verifier is barely being reached"
    );
}

/// A key share a peer chooses never panics, and what it *does* do differs by
/// group in a way worth pinning down.
///
/// The first draft of this test asserted that a random key share never agrees
/// a secret. That is false, and the fuzzer said so immediately: X25519 has no
/// invalid public keys. RFC 7748 §5 decodes *every* 32-octet string to a
/// u-coordinate, deliberately, so there is nothing to reject and a random
/// share agrees a perfectly good secret with a peer who does not know it.
///
/// That is not a weakness, but it does mean the small-order check is the only
/// validation X25519 has — which is why
/// `handrolled_kx::a_small_order_x25519_key_share_is_refused` carries more
/// weight than it looks like it should. The NIST curves are the opposite: a
/// random point is essentially never on the curve, so almost everything is
/// refused.
///
/// So the invariants are per-group, and the one that holds everywhere is that
/// a secret, if there is one, is never all zeroes.
#[test]
fn a_random_key_share_is_handled_the_way_its_group_requires() {
    let mut rng = Rng::new(0x5eed_000c);
    let rounds = iterations(2_000);

    for group in [
        NamedGroup::X25519,
        NamedGroup::SecP256R1,
        NamedGroup::SecP384R1,
    ] {
        let correct_length = match group {
            NamedGroup::SecP256R1 => 65,
            NamedGroup::SecP384R1 => 97,
            _ => 32,
        };
        let mut agreed = 0usize;

        for round in 0..rounds {
            // Three quarters correctly sized, so the NIST shares actually
            // reach the on-curve check rather than being turned away on
            // length.
            let length = if rng.below(4) == 0 {
                rng.below(128)
            } else {
                correct_length
            };
            let mut share: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
            if !share.is_empty() && rng.below(2) == 0 {
                share[0] = 0x04; // the uncompressed-point marker
            }
            let wrong_length = share.len() != correct_length;

            let kx = KeyExchange::generate(group).expect("generate");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                kx.agree(&share, <[u8]>::to_vec)
            }))
            .unwrap_or_else(|_| panic!("{group:?} panicked at round {round} on {share:02x?}"));

            match result {
                Ok(secret) => {
                    assert!(
                        !wrong_length,
                        "{group:?} agreed a secret from a {}-octet share",
                        share.len()
                    );
                    assert!(
                        secret.iter().any(|&b| b != 0),
                        "{group:?} produced an all-zero secret from {share:02x?}"
                    );
                    agreed += 1;
                }
                Err(_) => {}
            }
        }

        let rate = agreed * 100 / rounds;
        println!("{group:?}: {rate}% of random key shares agreed a secret");
        match group {
            // Every 32-octet string is a valid X25519 public key, so this
            // should be close to the three quarters that were correctly sized.
            NamedGroup::X25519 => assert!(
                rate >= 50,
                "X25519 refused {}% of random shares — it has no invalid \
                 public keys, so something is rejecting them for another reason",
                100 - rate
            ),
            // A random string is not a point on a NIST curve.
            _ => assert_eq!(
                rate, 0,
                "{group:?} accepted a random string as a point on the curve"
            ),
        }
    }
}
