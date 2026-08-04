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
        identity: TicketContents::identity_of(&[b"a certificate".to_vec()]),
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
