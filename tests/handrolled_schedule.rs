//! The key schedule against RFC 8448's published intermediate values.
//!
//! This suite is unusually easy to make strong, because [RFC 8448] does not
//! merely give inputs and outputs — it publishes every PRK, every `info`
//! string, and every expanded secret at every step of the schedule. So the
//! tests do not check that a handshake works; they check that each individual
//! derivation produces exactly the bytes the RFC says it does.
//!
//! [RFC 8448]: https://www.rfc-editor.org/rfc/rfc8448
//!
//! That matters more here than in most places. A key schedule that is
//! self-consistent but wrong interoperates perfectly with itself and with
//! nothing else — and a *round-trip* test cannot tell the difference, because
//! both sides of the round trip are the code under test. The RFC's values were
//! produced by neither this implementation nor rustls.
//!
//! # The transcript is the part worth checking hardest
//!
//! Every traffic secret is bound to a hash of the handshake messages that
//! produced it. That binding is what makes a rewritten ClientHello detectable,
//! and an implementation that derived correct-looking bytes from the wrong
//! transcript would fail to detect exactly the attack the construction exists
//! for. `the_schedule_is_bound_to_the_transcript` is the test for that, and it
//! is the one that would survive if everything else here were deleted.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::schedule::{
    binder_key, derive_secret, expand_label, finished_key, finished_verify_data, psk_binder,
    traffic_keys, update_traffic_secret, verify_finished, Hash, KeySchedule,
};

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(digits.len() % 2, 0, "hex input has an odd number of digits");
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).expect("hex"))
        .collect()
}

// ---------------------------------------------------------------------------
// RFC 8448 §3, "Simple 1-RTT Handshake" — every value, verbatim.
// ---------------------------------------------------------------------------

/// The (EC)DHE shared secret, the only real keying material in the trace.
const SHARED_SECRET: &str = "8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d";

/// `Hash(ClientHello..ServerHello)`.
const TRANSCRIPT_HELLO: &str = "860c06edc07858ee8e78f0e7428c58edd6b43f2ca3e6e95f02ed063cf0e1cad8";
/// `Hash(ClientHello..server Finished)`.
const TRANSCRIPT_SERVER_FINISHED: &str =
    "9608102a0f1ccc6db6250b7b7e417b1a000eaada3daae4777a7686c9ff83df13";
/// `Hash(ClientHello..client Finished)`.
const TRANSCRIPT_CLIENT_FINISHED: &str =
    "209145a96ee8e2a122ff810047cc952684658d6049e86429426db87c54ad143d";

const EARLY_SECRET: &str = "33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a";
const HANDSHAKE_SECRET: &str = "1dc826e93606aa6fdc0aadc12f741b01046aa6b99f691ed221a9f0ca043fbeac";
const MASTER_SECRET: &str = "18df06843d13a08bf2a449844c5f8a478001bc4d4c627984d5a41da8d0402919";

const CLIENT_HS_TRAFFIC: &str = "b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21";
const SERVER_HS_TRAFFIC: &str = "b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38";
const CLIENT_AP_TRAFFIC: &str = "9e40646ce79a7f9dc05af8889bce6552875afa0b06df0087f792ebb7c17504a5";
const SERVER_AP_TRAFFIC: &str = "a11af9f05531f856ad47116b45a950328204b4f44bfb6b3a4b4f1f3fcb631643";
const EXPORTER_MASTER: &str = "fe22f881176eda18eb8f44529e6792c50c9a3f89452f68d8ae311b4309d3cf50";
const RESUMPTION_MASTER: &str = "7df235f2031d2a051287d02b0241b0bfdaf86cc856231f2d5aba46c434ec196c";

/// The `"derived"` secret between Early and Handshake.
const DERIVED_FOR_HANDSHAKE: &str =
    "6f2615a108c702c5678f54fc9dbab69716c076189c48250cebeac3576c3611ba";
/// The `"derived"` secret between Handshake and Master.
const DERIVED_FOR_MASTER: &str = "43de77e0c77713859a944db9db2590b53190a65b3ee2e4f12dd7a0bb7ce254b4";

// Traffic keys, which the record-layer KAT in `handrolled_record_kat.rs`
// already uses from the other end — these tests derive them rather than
// hardcoding them as inputs, so the two suites now meet in the middle.
const SERVER_HS_KEY: &str = "3fce516009c21727d0f2e4e86ee403bc";
const SERVER_HS_IV: &str = "5d313eb2671276ee13000b30";
const SERVER_AP_KEY: &str = "9f02283b6c9c07efc26bb9f2ac92e356";
const SERVER_AP_IV: &str = "cf782b88dd83549aadf1e984";

const SERVER_FINISHED_KEY: &str =
    "008d3b66f816ea559f96b537e885c31fc068bf492c652f01f288a1d8cdc19fc8";
const CLIENT_FINISHED_KEY: &str =
    "b80ad01015fb2f0bd65ff7d4da5d6bf83f84821d1f87fdc7d3c75b5a7b42d9c4";
const CLIENT_VERIFY_DATA: &str = "a8ec436d677634ae525ac1fcebe11a039ec17694fac6e98527b642f2edd5ce61";

/// Rebuild the schedule from the RFC's inputs. Every test starts here.
fn rfc8448_schedule() -> (KeySchedule, KeySchedule, KeySchedule) {
    let early = KeySchedule::new(Hash::Sha256);
    let handshake = early.clone().into_handshake(&hex(SHARED_SECRET));
    let master = handshake.clone().into_master();
    (early, handshake, master)
}

// ---------------------------------------------------------------------------
// The schedule, stage by stage
// ---------------------------------------------------------------------------

/// Every extraction in the schedule, against the RFC's published secrets.
#[test]
fn the_three_extracted_secrets_match_rfc8448() {
    let (early, handshake, master) = rfc8448_schedule();

    assert_eq!(early.secret(), hex(EARLY_SECRET), "Early Secret");
    assert_eq!(
        handshake.secret(),
        hex(HANDSHAKE_SECRET),
        "Handshake Secret"
    );
    assert_eq!(master.secret(), hex(MASTER_SECRET), "Master Secret");
}

/// The `"derived"` steps between stages, which are easy to omit and produce a
/// schedule that is wrong from that point down.
#[test]
fn the_derived_secrets_between_stages_match_rfc8448() {
    let (early, handshake, _) = rfc8448_schedule();
    let empty = Hash::Sha256.empty_hash();

    assert_eq!(
        derive_secret(Hash::Sha256, early.secret(), "derived", &empty),
        hex(DERIVED_FOR_HANDSHAKE)
    );
    assert_eq!(
        derive_secret(Hash::Sha256, handshake.secret(), "derived", &empty),
        hex(DERIVED_FOR_MASTER)
    );
}

/// Every traffic secret the trace publishes, from the stage that produces it
/// and the transcript hash that binds it.
#[test]
fn every_traffic_secret_matches_rfc8448() {
    let (_, handshake, master) = rfc8448_schedule();

    for (label, transcript, expected, schedule) in [
        (
            "c hs traffic",
            TRANSCRIPT_HELLO,
            CLIENT_HS_TRAFFIC,
            &handshake,
        ),
        (
            "s hs traffic",
            TRANSCRIPT_HELLO,
            SERVER_HS_TRAFFIC,
            &handshake,
        ),
        (
            "c ap traffic",
            TRANSCRIPT_SERVER_FINISHED,
            CLIENT_AP_TRAFFIC,
            &master,
        ),
        (
            "s ap traffic",
            TRANSCRIPT_SERVER_FINISHED,
            SERVER_AP_TRAFFIC,
            &master,
        ),
        (
            "exp master",
            TRANSCRIPT_SERVER_FINISHED,
            EXPORTER_MASTER,
            &master,
        ),
        (
            "res master",
            TRANSCRIPT_CLIENT_FINISHED,
            RESUMPTION_MASTER,
            &master,
        ),
    ] {
        assert_eq!(
            schedule.derive(label, &hex(transcript)),
            hex(expected),
            "Derive-Secret(., {label:?}, ...)"
        );
    }
}

/// The empty-string hash the `"derived"` steps use, against the RFC's own
/// published value.
///
/// Small, and worth its own test: every `Derive-Secret(., "derived", "")` in
/// the schedule depends on it, so getting it wrong breaks the schedule from
/// the handshake stage down while leaving the early secret correct.
#[test]
fn the_empty_transcript_hash_matches_rfc8448() {
    assert_eq!(
        Hash::Sha256.empty_hash(),
        hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
    // And `Hash::hash` agrees with it, so a caller building a transcript with
    // the same type cannot end up with a different digest than the schedule.
    assert_eq!(Hash::Sha256.hash(b""), Hash::Sha256.empty_hash());
}

// ---------------------------------------------------------------------------
// Traffic keys and Finished — where the schedule meets the record layer
// ---------------------------------------------------------------------------

/// The traffic keys `handrolled_record_kat.rs` uses as *inputs* are derived
/// here from the schedule, so the two suites now meet in the middle: the
/// schedule produces exactly the keys the record layer was independently
/// verified against.
#[test]
fn traffic_keys_match_the_record_layer_kat() {
    let (_, handshake, master) = rfc8448_schedule();

    let hs = handshake.derive("s hs traffic", &hex(TRANSCRIPT_HELLO));
    let hs_keys = traffic_keys(Hash::Sha256, &hs, 16);
    assert_eq!(hs_keys.key, hex(SERVER_HS_KEY), "server handshake key");
    assert_eq!(hs_keys.iv, hex(SERVER_HS_IV), "server handshake iv");

    let ap = master.derive("s ap traffic", &hex(TRANSCRIPT_SERVER_FINISHED));
    let ap_keys = traffic_keys(Hash::Sha256, &ap, 16);
    assert_eq!(ap_keys.key, hex(SERVER_AP_KEY), "server application key");
    assert_eq!(ap_keys.iv, hex(SERVER_AP_IV), "server application iv");
}

/// The whole point of the test above: keys derived here actually decrypt the
/// RFC's wire bytes through the hand-rolled record layer.
///
/// Two independently-verified modules, joined. Neither was written against
/// the other, and both were written against the same trace.
#[test]
fn derived_keys_decrypt_the_rfc8448_wire_record() {
    use rusty_tls::handrolled::record::{Aead, ContentType, Opener};

    let (_, handshake, _) = rfc8448_schedule();
    let secret = handshake.derive("s hs traffic", &hex(TRANSCRIPT_HELLO));
    let keys = traffic_keys(Hash::Sha256, &secret, 16);

    // The server's encrypted handshake flight, from RFC 8448 §3.
    let record = hex("17030302a2d1ff334a56f5bff6594a07\
         cc87b580233f500f45e489e7f33af35e\
         df7869fcf40aa40aa2b8ea73f848a7ca\
         07612ef9f945cb960b4068905123ea78\
         b111b429ba9191cd05d2a389280f5261\
         34aadc7fc78c4b729df828b5ecf7b13b\
         d9aefb0e57f271585b8ea9bb355c7c79\
         020716cfb9b1183ef3ab20e37d57a6b9\
         d7477609aee6e122a4cf51427325250c\
         7d0e509289444c9b3a648f1d71035d2e\
         d65b0e3cdd0cbae8bf2d0b227812cbb3\
         60987255cc744110c453baa4fcd61092\
         8d809810e4b7ed1a8fd991f06aa62482\
         04797e36a6a73b70a2559c09ead68694\
         5ba246ab66e5edd8044b4c6de3fcf2a8\
         9441ac66272fd8fb330ef8190579b368\
         4596c960bd596eea520a56a8d650f563\
         aad27409960dca63d3e688611ea5e22f\
         4415cf9538d51a200c27034272968a26\
         4ed6540c84838d89f72c24461aad6d26\
         f59ecaba9acbbb317b66d902f4f292a3\
         6ac1b639c637ce343117b65962224531\
         7b49eeda0c6258f100d7d961ffb13864\
         7e92ea330faeea6dfa31c7a84dc3bd7e\
         1b7a6c7178af36879018e3f252107f24\
         3d243dc7339d5684c8b0378bf30244da\
         8c87c843f5e56eb4c5e8280a2b48052c\
         f93b16499a66db7cca71e4599426f7d4\
         61e66f99882bd89fc50800becca62d6c\
         74116dbd2972fda1fa80f85df881edbe\
         5a37668936b335583b599186dc5c6918\
         a396fa48a181d6b6fa4f9d62d513afbb\
         992f2b992f67f8afe67f76913fa388cb\
         5630c8ca01e0c65d11c66a1e2ac4c859\
         77b7c7a6999bbf10dc35ae69f5515614\
         636c0b9b68c19ed2e31c0b3b66763038\
         ebba42f3b38edc0399f3a9f23faa6397\
         8c317fc9fa66a73f60f0504de93b5b84\
         5e275592c12335ee340bbc4fddd50278\
         4016e4b3be7ef04dda49f4b440a30cb5\
         d2af939828fd4ae3794e44f94df5a631\
         ede42c1719bfdabf0253fe5175be898e\
         750edc53370d2b");
    // Extracted from the RFC mechanically rather than transcribed by hand,
    // and its length asserted, so a slip shows up as a length error rather
    // than a decrypt failure blamed on the code.
    assert_eq!(record.len(), 679, "the RFC's record is 679 octets");

    let mut opener = Opener::new(Aead::Aes128Gcm, &keys.key, &keys.iv).expect("opener builds");
    let opened = opener
        .open(&record)
        .expect("keys derived from the schedule must open the RFC's own record");

    assert_eq!(opened.typ, ContentType::Handshake);
    // EncryptedExtensions, handshake type 8.
    assert_eq!(opened.fragment[0], 0x08);
    assert_eq!(opened.fragment.len(), 657);
}

#[test]
fn finished_keys_and_verify_data_match_rfc8448() {
    let (_, handshake, _) = rfc8448_schedule();

    let server = handshake.derive("s hs traffic", &hex(TRANSCRIPT_HELLO));
    assert_eq!(
        finished_key(Hash::Sha256, &server),
        hex(SERVER_FINISHED_KEY),
        "server finished_key"
    );

    let client = handshake.derive("c hs traffic", &hex(TRANSCRIPT_HELLO));
    assert_eq!(
        finished_key(Hash::Sha256, &client),
        hex(CLIENT_FINISHED_KEY),
        "client finished_key"
    );

    // The client's Finished covers everything through the server's Finished
    // (RFC 8446 §4.4.4), and RFC 8448 publishes that transcript hash — so
    // this is a real end-to-end known-answer test of the MAC and not just of
    // the key.
    //
    // The *server's* verify_data is deliberately not asserted here: its
    // transcript runs through the server's CertificateVerify, and RFC 8448
    // does not publish that hash as a labelled value. Computing it needs the
    // handshake messages themselves, which is stage 3b. Asserting it against
    // a hash invented for the purpose would be worse than not asserting it.
    assert_eq!(
        finished_verify_data(Hash::Sha256, &client, &hex(TRANSCRIPT_SERVER_FINISHED)),
        hex(CLIENT_VERIFY_DATA),
        "client verify_data"
    );
    assert!(verify_finished(
        Hash::Sha256,
        &client,
        &hex(TRANSCRIPT_SERVER_FINISHED),
        &hex(CLIENT_VERIFY_DATA)
    ));
}

/// A Finished MAC must verify, and must stop verifying when anything it
/// covers changes.
#[test]
fn finished_verification_rejects_a_tampered_transcript() {
    let (_, handshake, _) = rfc8448_schedule();
    let secret = handshake.derive("s hs traffic", &hex(TRANSCRIPT_HELLO));
    let transcript = hex(TRANSCRIPT_SERVER_FINISHED);

    let verify_data = finished_verify_data(Hash::Sha256, &secret, &transcript);
    assert!(verify_finished(
        Hash::Sha256,
        &secret,
        &transcript,
        &verify_data
    ));

    // Every single-bit change to the transcript must break it.
    for index in 0..transcript.len() {
        let mut tampered = transcript.clone();
        tampered[index] ^= 0x01;
        assert!(
            !verify_finished(Hash::Sha256, &secret, &tampered, &verify_data),
            "a transcript with byte {index} flipped still verified"
        );
    }
    // As must every change to the MAC itself.
    for index in 0..verify_data.len() {
        let mut tampered = verify_data.clone();
        tampered[index] ^= 0x01;
        assert!(
            !verify_finished(Hash::Sha256, &secret, &transcript, &tampered),
            "verify_data with byte {index} flipped still verified"
        );
    }
    // And a truncated MAC must not verify as a prefix.
    for cut in 0..verify_data.len() {
        assert!(!verify_finished(
            Hash::Sha256,
            &secret,
            &transcript,
            &verify_data[..cut]
        ));
    }
}

// ---------------------------------------------------------------------------
// The property the whole construction exists for
// ---------------------------------------------------------------------------

/// Every traffic secret is bound to the transcript that produced it.
///
/// This is what makes a rewritten ClientHello detectable: two peers that saw
/// different handshakes cannot arrive at the same keys. An implementation that
/// ignored the transcript, or hashed a different one, would pass a round-trip
/// test and fail to detect exactly the attack the construction exists for.
#[test]
fn the_schedule_is_bound_to_the_transcript() {
    let (_, handshake, master) = rfc8448_schedule();
    let transcript = hex(TRANSCRIPT_HELLO);

    let baseline = handshake.derive("c hs traffic", &transcript);
    assert_eq!(baseline, hex(CLIENT_HS_TRAFFIC));

    for index in 0..transcript.len() {
        let mut tampered = transcript.clone();
        tampered[index] ^= 0x01;
        assert_ne!(
            handshake.derive("c hs traffic", &tampered),
            baseline,
            "flipping transcript byte {index} did not change the traffic secret"
        );
    }

    // And the label separates secrets derived from the same transcript at the
    // same stage — otherwise client and server would share keys.
    assert_ne!(
        handshake.derive("c hs traffic", &transcript),
        handshake.derive("s hs traffic", &transcript),
        "client and server handshake secrets are identical"
    );
    assert_ne!(
        master.derive("c ap traffic", &transcript),
        master.derive("s ap traffic", &transcript)
    );
}

/// A different shared secret must produce a different schedule from the
/// handshake stage down. The early secret is fixed and public; everything
/// after the (EC)DHE input must not be.
#[test]
fn the_shared_secret_changes_everything_below_it() {
    let early = KeySchedule::new(Hash::Sha256);
    let baseline = early.clone().into_handshake(&hex(SHARED_SECRET));

    let mut other = hex(SHARED_SECRET);
    other[0] ^= 0x01;
    let changed = early.into_handshake(&other);

    assert_ne!(baseline.secret(), changed.secret());
    assert_ne!(
        baseline.clone().into_master().secret(),
        changed.clone().into_master().secret()
    );
    assert_ne!(
        baseline.derive("c hs traffic", &hex(TRANSCRIPT_HELLO)),
        changed.derive("c hs traffic", &hex(TRANSCRIPT_HELLO))
    );
}

// ---------------------------------------------------------------------------
// SHA-384, key updates, and HKDF itself
// ---------------------------------------------------------------------------

/// SHA-384 is the other hash TLS 1.3 defines, and every length in the
/// schedule follows from it. RFC 8448's traces are all SHA-256, so this
/// checks the shape rather than published bytes — and says so.
#[test]
fn the_sha384_schedule_has_the_right_shape() {
    let early = KeySchedule::new(Hash::Sha384);
    assert_eq!(early.secret().len(), 48);
    assert_eq!(Hash::Sha384.empty_hash().len(), 48);

    let handshake = early.into_handshake(&[0x42; 48]);
    assert_eq!(handshake.secret().len(), 48);

    let secret = handshake.derive("c hs traffic", &Hash::Sha384.empty_hash());
    assert_eq!(secret.len(), 48);

    // AES-256-GCM pairs with SHA-384: a 32-byte key, and an IV whose length
    // comes from the record layer rather than the hash.
    let keys = traffic_keys(Hash::Sha384, &secret, 32);
    assert_eq!(keys.key.len(), 32);
    assert_eq!(keys.iv.len(), 12);
    assert_eq!(finished_key(Hash::Sha384, &secret).len(), 48);

    // The two hashes must not produce the same schedule from the same inputs.
    assert_ne!(
        KeySchedule::new(Hash::Sha256).secret(),
        KeySchedule::new(Hash::Sha384).secret()
    );
}

/// A key update must move to a new secret, and must be one-way.
#[test]
fn a_key_update_produces_a_new_unrelated_secret() {
    let (_, _, master) = rfc8448_schedule();
    let first = master.derive("c ap traffic", &hex(TRANSCRIPT_SERVER_FINISHED));

    let second = update_traffic_secret(Hash::Sha256, &first);
    let third = update_traffic_secret(Hash::Sha256, &second);

    assert_eq!(second.len(), 32);
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert_ne!(first, third);

    // Each update produces different record keys, which is the observable
    // consequence a peer actually sees.
    assert_ne!(
        traffic_keys(Hash::Sha256, &first, 16),
        traffic_keys(Hash::Sha256, &second, 16)
    );
}

/// `HKDF-Expand-Label` must produce whatever length is asked for, including
/// lengths past one hash block, where the counter in HKDF's expand loop
/// starts to matter.
#[test]
fn expand_label_produces_the_requested_length() {
    let secret = hex(CLIENT_HS_TRAFFIC);

    for length in [1usize, 16, 31, 32, 33, 48, 64, 100, 255, 256] {
        let out = expand_label(Hash::Sha256, &secret, "test", b"", length);
        assert_eq!(out.len(), length, "expand_label({length})");
    }

    // Plain HKDF-Expand produces a stream, so a longer output would begin
    // with a shorter one. HKDF-Expand-*Label* does not, because the requested
    // length is a field of `HkdfLabel` and therefore part of the `info`.
    // That is free domain separation by output length, and asserting it the
    // other way round — which is what a reading of RFC 5869 alone suggests —
    // is how this test was first written and immediately failed.
    let short = expand_label(Hash::Sha256, &secret, "test", b"", 32);
    let long = expand_label(Hash::Sha256, &secret, "test", b"", 96);
    assert_ne!(&long[..32], &short[..]);

    // Different labels and different contexts must diverge.
    assert_ne!(
        expand_label(Hash::Sha256, &secret, "a", b"", 32),
        expand_label(Hash::Sha256, &secret, "b", b"", 32)
    );
    assert_ne!(
        expand_label(Hash::Sha256, &secret, "a", b"x", 32),
        expand_label(Hash::Sha256, &secret, "a", b"y", 32)
    );
}

/// The `"tls13 "` prefix is domain separation, and it has to actually be
/// there — a schedule without it would derive the same bytes as any other
/// protocol using HKDF with the same PRK and label.
#[test]
fn the_tls13_prefix_is_part_of_the_label() {
    let secret = hex(CLIENT_HS_TRAFFIC);
    // "key" with the prefix must differ from "tls13 key" with the prefix,
    // which is what a doubled prefix would produce.
    assert_ne!(
        expand_label(Hash::Sha256, &secret, "key", b"", 16),
        expand_label(Hash::Sha256, &secret, "tls13 key", b"", 16)
    );
    // And the published "key" expansion is the one that matches the RFC —
    // covered by `traffic_keys_match_the_record_layer_kat`, which would fail
    // if the prefix were missing or doubled.
}

// ---------------------------------------------------------------------------
// PSK binders — rusty_tls#43, stage three
// ---------------------------------------------------------------------------

/// A binder is a function of the PSK and the truncated transcript, and of
/// nothing else.
///
/// Deterministic is not a weak property here: the server recomputes the same
/// value from its own copy of both, and a binder that varied would fail every
/// resumption for reasons no log would explain.
#[test]
fn a_binder_is_determined_by_the_psk_and_the_transcript() {
    let psk = [7u8; 32];
    let transcript = [9u8; 32];

    let once = psk_binder(Hash::Sha256, &psk, &transcript);
    let twice = psk_binder(Hash::Sha256, &psk, &transcript);
    assert_eq!(once, twice);
    assert_eq!(once.len(), 32, "a SHA-256 binder is the hash length");
}

/// A different PSK gives a different binder.
///
/// This is what the binder is *for*: it proves the client holds the key the
/// ticket stands for. A binder that ignored the PSK would be a proof of
/// nothing, and would still look correct in a round-trip test.
#[test]
fn a_binder_depends_on_the_psk() {
    let transcript = [9u8; 32];
    let mine = psk_binder(Hash::Sha256, &[7u8; 32], &transcript);
    let theirs = psk_binder(Hash::Sha256, &[8u8; 32], &transcript);
    assert_ne!(mine, theirs, "the binder ignored the pre-shared key");
}

/// A different truncated transcript gives a different binder.
///
/// The other half of what it proves: that *this* ClientHello is the one the
/// key was offered with. Without it a captured binder could be replayed onto a
/// different hello.
#[test]
fn a_binder_depends_on_the_transcript() {
    let psk = [7u8; 32];
    let here = psk_binder(Hash::Sha256, &psk, &[9u8; 32]);
    let there = psk_binder(Hash::Sha256, &psk, &[10u8; 32]);
    assert_ne!(here, there, "the binder ignored the transcript");
}

/// The binder key is not the PSK, and not the PSK's early secret either.
///
/// `Derive-Secret` stands between them on purpose: a binder computed directly
/// from the PSK would leak a distinguisher on the key itself, and the whole
/// point of the schedule is that each secret is used for exactly one thing.
#[test]
fn the_binder_key_is_derived_rather_than_the_psk_itself() {
    let psk = [7u8; 32];
    let key = binder_key(Hash::Sha256, &psk);

    assert_ne!(key.as_slice(), psk.as_slice());
    assert_eq!(key.len(), 32);
    // And it is bound to the PSK, so two sessions never share one.
    assert_ne!(key, binder_key(Hash::Sha256, &[8u8; 32]));
}

/// A SHA-384 session's binder is 48 octets, not 32.
///
/// The binder's length follows the suite's hash, so a resumption offered under
/// a different suite than the ticket was issued for is a different computation
/// — which is why [`Session`] records the suite it belongs to.
#[test]
fn a_binders_length_follows_the_hash() {
    let psk = [7u8; 48];
    assert_eq!(psk_binder(Hash::Sha384, &psk, &[9u8; 48]).len(), 48);
}
