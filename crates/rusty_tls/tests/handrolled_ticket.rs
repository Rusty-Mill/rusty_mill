//! Session tickets and the key that seals them — stage 5, `rusty_tls#43`.
//!
//! # What this file can and cannot show
//!
//! A ticket is sealed and opened by the same code with the same key, so almost
//! every test here is self-consistent by construction: a round trip proves the
//! encoder and the decoder agree and nothing else. That is worth having and it
//! is not evidence about the protocol.
//!
//! The evidence about the protocol is in `handrolled_server`, where a `rustls`
//! client redeems a ticket this code minted. `rustls` derives the PSK from the
//! NewSessionTicket's nonce itself, so its accepting the resumption is what
//! says the ticket carries the key it claims to.
//!
//! What *is* worth testing here is the set of refusals — the cases where a
//! ticket must not open, or must open and then not be honoured. Those are the
//! ones a round trip cannot reach and an interop test never produces, because a
//! well-behaved peer never sends them.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::client::CipherSuite;
use rusty_tls::handrolled::ticket::{TicketContents, TicketError, TicketKey, TicketKeys};

fn contents() -> TicketContents {
    TicketContents {
        suite: CipherSuite::TLS_AES_128_GCM_SHA256,
        issued_at: 1_800_000_000,
        lifetime: 7200,
        age_add: 0x1234_5678,
        identity: TicketContents::identity_of(&[b"a certificate".to_vec()]),
        client_certificates: Vec::new(),
        psk: vec![0xabu8; 32],
    }
}

fn keys<'a>(current: &'a TicketKey, previous: &'a [&'a TicketKey]) -> TicketKeys<'a> {
    TicketKeys { current, previous }
}

#[test]
fn a_ticket_opens_under_the_key_that_sealed_it() {
    let key = TicketKey::generate().expect("a key");
    let sealed = contents().seal(&key).expect("seal");
    let opened = TicketContents::open(&sealed, &keys(&key, &[]))
        .expect("the key opened it")
        .expect("it decoded");
    assert_eq!(opened, contents());
}

/// A ticket sealed under a different key is *not opened*, and that is not an
/// error.
///
/// The distinction carries the whole rotation story. Every ticket becomes
/// unopenable eventually — a key rotates, a fleet member has a different key —
/// and a server that treated that as a protocol violation would refuse clients
/// for holding a ticket it had itself issued a week earlier.
#[test]
fn a_ticket_under_another_key_is_ignored_rather_than_refused() {
    let key = TicketKey::generate().expect("a key");
    let other = TicketKey::generate().expect("another key");
    let sealed = contents().seal(&key).expect("seal");

    assert!(
        TicketContents::open(&sealed, &keys(&other, &[])).is_none(),
        "a ticket opened under a key that did not seal it"
    );
    assert!(
        TicketContents::open(&sealed, &keys(&other, &[&key])).is_some(),
        "a retained previous key did not open its own ticket"
    );
}

/// Every single-bit change anywhere in a ticket makes it not open.
///
/// This is AES-GCM's property rather than this code's, and it is tested
/// because the thing that would break it is *this* code: a ticket assembled
/// so that some prefix is outside the authenticated span would tamper
/// silently, and the nonce carried in the clear at the front is exactly the
/// kind of thing that invites that mistake.
#[test]
fn no_altered_ticket_opens() {
    let key = TicketKey::generate().expect("a key");
    let sealed = contents().seal(&key).expect("seal");

    for index in 0..sealed.len() {
        let mut altered = sealed.clone();
        altered[index] ^= 0x01;
        assert!(
            TicketContents::open(&altered, &keys(&key, &[])).is_none(),
            "a ticket with octet {index} flipped opened anyway"
        );
    }

    for length in 0..sealed.len() {
        assert!(
            TicketContents::open(&sealed[..length], &keys(&key, &[])).is_none(),
            "a ticket truncated to {length} octets opened anyway"
        );
    }
}

/// The sealing is bound to associated data, so a blob sealed under this key by
/// anything else does not open as a ticket.
///
/// `ring` is reached directly here, which is the only way to ask the question:
/// the assertion is about what *this* code passes to the cipher, and every
/// path through this crate passes the same thing. Without it, dropping the
/// associated data entirely is a change no test in this repo notices — which
/// was measured before this test existed.
#[test]
fn a_ticket_is_sealed_with_associated_data() {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};

    let secret = [0x5au8; TicketKey::LEN];
    let key = TicketKey::new(&secret).expect("a key");
    let sealed = contents().seal(&key).expect("seal");

    let raw = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &secret).expect("ring key"));
    let (nonce, body) = sealed.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("a nonce");

    let mut buffer = body.to_vec();
    assert!(
        raw.open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut buffer,
        )
        .is_err(),
        "a ticket opened with no associated data, so none was used to seal it"
    );

    // And the same call with the real associated data does open it, so the
    // assertion above is about the associated data and not about the layout.
    let mut buffer = body.to_vec();
    assert!(
        raw.open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(b"rusty_tls resumption ticket v1"),
            &mut buffer,
        )
        .is_ok(),
        "the associated data is not what the module documents"
    );
}

/// Two tickets sealed from identical contents differ.
///
/// The nonce is what makes them differ, and it is carried in the clear, so a
/// server that reused one would be visibly issuing the same bytes twice — and
/// invisibly reusing a GCM nonce, which is the failure that cipher is least
/// forgiving about.
#[test]
fn two_tickets_from_the_same_contents_differ() {
    let key = TicketKey::generate().expect("a key");
    let first = contents().seal(&key).expect("seal");
    let second = contents().seal(&key).expect("seal");
    assert_ne!(first, second, "two seals produced identical tickets");
    assert_ne!(&first[..12], &second[..12], "two tickets share a nonce");
}

/// A key that is not the right length is refused rather than stretched.
///
/// Padding a short key or truncating a long one produces a key nobody chose,
/// and the caller would have no way to tell it had happened.
#[test]
fn a_key_of_the_wrong_length_is_refused() {
    assert_eq!(
        TicketKey::new(&[0u8; 16]).err(),
        Some(TicketError::BadKeyLength(16))
    );
    assert_eq!(
        TicketKey::new(&[0u8; 48]).err(),
        Some(TicketError::BadKeyLength(48))
    );
    assert!(TicketKey::new(&[0u8; TicketKey::LEN]).is_ok());
}

/// Freshness is checked in both directions.
///
/// A ticket from the future is as suspect as one from too far in the past: it
/// means the issuing server's clock and this one's disagree, and honouring it
/// would extend the ticket's real lifetime by however much they differ.
#[test]
fn a_ticket_is_current_only_within_its_lifetime() {
    let contents = contents();
    let issued = contents.issued_at;

    assert!(contents.is_current(issued));
    assert!(contents.is_current(issued + i64::from(contents.lifetime)));
    assert!(!contents.is_current(issued + i64::from(contents.lifetime) + 1));
    assert!(
        !contents.is_current(issued - 1),
        "a ticket issued in the future was accepted"
    );
}

/// The identity digest is over a length-prefixed chain, so two different chains
/// cannot hash the same.
///
/// A digest over the concatenation alone would let `["ab", "c"]` and
/// `["a", "bc"]` collide — which, for a binding that decides whether a session
/// may continue under a given certificate, is a collision an attacker gets to
/// choose the inputs to.
#[test]
fn the_identity_digest_separates_chains_that_concatenate_the_same() {
    let one = TicketContents::identity_of(&[b"ab".to_vec(), b"c".to_vec()]);
    let other = TicketContents::identity_of(&[b"a".to_vec(), b"bc".to_vec()]);
    assert_ne!(one, other, "two different chains produced one identity");

    // And it is stable, so a ticket outlives the process that issued it.
    assert_eq!(
        one,
        TicketContents::identity_of(&[b"ab".to_vec(), b"c".to_vec()])
    );
}

/// A ticket that opens and is not a ticket is an error, unlike one that does
/// not open at all.
///
/// The two are different events. A blob that does not open is somebody else's
/// ticket, which is ordinary. A blob that opens under *this* key and then turns
/// out to be nonsense means this server sealed something it cannot read, which
/// is a bug in this server and not a client's doing.
#[test]
fn a_ticket_that_opens_and_is_not_a_ticket_is_an_error() {
    let key = TicketKey::generate().expect("a key");
    // Seal something that is not a ticket, using the same key and the same
    // associated data — which only this crate's own encoder normally produces.
    let mut wrong = contents();
    wrong.psk.clear();
    let sealed = wrong.seal(&key).expect("seal");

    let opened = TicketContents::open(&sealed, &keys(&key, &[])).expect("the key opened it");
    assert_eq!(opened, Err(TicketError::Malformed("it carries no key")));
}

/// A ticket key says nothing about itself.
#[test]
fn a_ticket_keys_debug_says_nothing_useful() {
    let key = TicketKey::new(&[0x11u8; TicketKey::LEN]).expect("a key");
    let rendered = format!("{key:?}");
    assert!(rendered.contains("redacted"), "{rendered}");
    assert!(!rendered.contains("11"), "{rendered}");
}

/// The reported age is exactly recoverable from the obfuscated one.
///
/// The obfuscation is an addition modulo 2³², so subtracting the addend loses
/// nothing — including across the wrap, which is the case an implementation
/// gets wrong and which a server that only ever saw small ages would never
/// meet.
#[test]
fn the_obfuscated_age_is_exactly_reversible() {
    let contents = contents();
    for age in [0u32, 1, 5_000, u32::MAX / 2, u32::MAX - 1, u32::MAX] {
        let obfuscated = age.wrapping_add(contents.age_add);
        assert_eq!(
            contents.reported_age_ms(obfuscated),
            age,
            "an age of {age} did not survive obfuscation"
        );
    }
}

/// Freshness of the reported age is judged against the server's own record,
/// in both directions, and does not wrap into passing.
///
/// A ticket whose issuing clock is wildly ahead of this one must *fail* the
/// check. Arithmetic that wrapped or went negative would turn the most
/// suspicious input into the most plausible-looking one.
#[test]
fn an_implausible_reported_age_is_rejected() {
    let contents = contents();
    let issued = contents.issued_at;
    let obfuscate = |age: u32| age.wrapping_add(contents.age_add);

    // Issued a moment ago, and the client says so.
    assert!(contents.age_is_plausible(obfuscate(0), issued, 60_000));
    assert!(contents.age_is_plausible(obfuscate(60_000), issued, 60_000));
    assert!(!contents.age_is_plausible(obfuscate(60_001), issued, 60_000));

    // Ten seconds later, and the client agrees it is ten seconds old.
    assert!(contents.age_is_plausible(obfuscate(10_000), issued + 10, 60_000));
    // ...and disagrees by an hour.
    assert!(!contents.age_is_plausible(obfuscate(3_600_000), issued + 10, 60_000));

    // A clock far behind the issuing one: the elapsed time is negative, which
    // must read as zero rather than as an enormous unsigned age.
    assert!(contents.age_is_plausible(obfuscate(0), issued - 10_000, 60_000));
    assert!(!contents.age_is_plausible(obfuscate(3_600_000), issued - 10_000, 60_000));

    // And an elapsed time that overflows the millisecond arithmetic must not
    // wrap into *agreeing* with the age a client reports. The value is chosen,
    // not arbitrary: 4_294_968 seconds is the smallest elapsed time whose
    // millisecond count exceeds 2³², and it wraps to 704 — so a server doing
    // `(elapsed as u32) * 1000` would find a client claiming 704 ms to be
    // perfectly plausible after a hundred and thirty-six years.
    assert!(!contents.age_is_plausible(obfuscate(704), issued + 4_294_968, 60_000));
    assert!(!contents.age_is_plausible(obfuscate(1_000), issued + 5_000_000, 60_000));
    // Beyond `u32`'s range in seconds, where the conversion itself must
    // saturate rather than truncate.
    assert!(!contents.age_is_plausible(obfuscate(0), issued + 10_000_000_000, 60_000));
}

/// A ticket at a layout version this code does not write is *ignored*, not
/// refused.
///
/// The distinction is the same one that makes key rotation survivable, applied
/// to a different kind of rotation. A server that aborted on its own
/// predecessor's tickets would turn every upgrade into an outage for whoever
/// was mid-session — and unlike an unopenable ticket, this one decrypts
/// perfectly, so nothing else would catch it.
///
/// `ring` is used directly because this crate's own encoder only ever writes
/// the current version, which is exactly the property under test.
#[test]
fn a_ticket_at_another_layout_version_is_ignored_rather_than_refused() {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};

    let secret = [0x77u8; TicketKey::LEN];
    let key = TicketKey::new(&secret).expect("a key");
    let raw = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &secret).expect("ring key"));

    // A plausible version-1 ticket: the layout this crate wrote before tickets
    // grew `ticket_age_add` and the client's chain.
    let mut plaintext = vec![1u8]; // version
    plaintext.extend_from_slice(&0x1301u16.to_be_bytes());
    plaintext.extend_from_slice(&0u32.to_be_bytes()); // issued_at, high
    plaintext.extend_from_slice(&1_800_000_000u32.to_be_bytes()); // issued_at, low
    plaintext.extend_from_slice(&7200u32.to_be_bytes()); // lifetime
    plaintext.push(0); // identity, empty
    plaintext.push(32); // psk
    plaintext.extend_from_slice(&[0xabu8; 32]);

    let nonce = [0x33u8; NONCE_LEN];
    let mut sealed = plaintext;
    raw.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(b"rusty_tls resumption ticket v1"),
        &mut sealed,
    )
    .expect("seal");
    let mut ticket = nonce.to_vec();
    ticket.extend_from_slice(&sealed);

    assert!(
        TicketContents::open(&ticket, &keys(&key, &[])).is_none(),
        "a ticket at an older layout version was refused rather than ignored, \
         which turns an upgrade into an outage"
    );
}

/// A client chain survives the round trip, including an empty one.
///
/// The empty case is not a formality: "this client presented nothing" and
/// "this ticket predates client chains" have to be distinguishable, and after
/// the version bump above they are — the second does not decode at all.
#[test]
fn a_client_chain_survives_the_round_trip() {
    let key = TicketKey::generate().expect("a key");

    let mut authenticated = contents();
    authenticated.client_certificates = vec![b"a client leaf".to_vec(), b"an issuer".to_vec()];
    let sealed = authenticated.seal(&key).expect("seal");
    let opened = TicketContents::open(&sealed, &keys(&key, &[]))
        .expect("opened")
        .expect("decoded");
    assert_eq!(opened, authenticated);

    let anonymous = contents();
    assert!(anonymous.client_certificates.is_empty());
    let sealed = anonymous.seal(&key).expect("seal");
    let opened = TicketContents::open(&sealed, &keys(&key, &[]))
        .expect("opened")
        .expect("decoded");
    assert_eq!(opened, anonymous);
}
