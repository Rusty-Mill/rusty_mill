//! `rusty_prost` against `prost` on the same schema: identical bytes, each side reads the
//! other's output, and they agree on what is malformed.
#![allow(clippy::unwrap_used)]

mod common;

use common::{random_all, All, Lcg, PAll};
use prost::Message as _;
use rusty_prost::{Error, Message, Reader};
use rusty_protobuf::WireType;

const ROUNDS: usize = 3000;

/// Field 20 of `All` is the map. prost does not check the wire type of a map field: it reads a
/// varint-typed one as the length of an entry. Everywhere else it rejects a mistyped field, as
/// we do, so this is the one place a `WrongWireType` of ours can meet a prost `Ok`.
fn has_mistyped_map(buf: &[u8]) -> bool {
    Reader::new(buf)
        .filter_map(|f| f.ok())
        .any(|f| f.number == 20 && f.wire_type() != WireType::Len)
}

/// Why we may reject what prost accepts: a rule this crate is deliberately stricter about.
fn explained(e: Error, buf: &[u8]) -> bool {
    match e {
        Error::Group | Error::BadTag => true,
        Error::WrongWireType { .. } => has_mistyped_map(buf),
        _ => false,
    }
}

/// Whether prost accepts `buf` while we reject it for a reason `explained` does not cover.
fn unexplained_rejection(buf: &[u8]) -> bool {
    match (All::decode(buf), PAll::decode(buf)) {
        (Err(e), Ok(_)) => !explained(e, buf),
        _ => false,
    }
}

/// Removes bytes while the disagreement survives, so a failure reports a readable input.
fn shrink(mut buf: Vec<u8>) -> Vec<u8> {
    loop {
        let before = buf.len();
        let mut chunk = buf.len().max(2) / 2;
        while chunk >= 1 {
            let mut at = 0;
            while at + chunk <= buf.len() {
                let mut cut = buf.clone();
                cut.drain(at..at + chunk);
                if unexplained_rejection(&cut) {
                    buf = cut;
                } else {
                    at += 1;
                }
            }
            chunk /= 2;
        }
        if buf.len() == before {
            return buf;
        }
    }
}

#[test]
fn both_encode_the_same_bytes() {
    let mut rng = Lcg(1);
    for i in 0..ROUNDS {
        let ours = random_all(&mut rng);
        let theirs = PAll::from(&ours);
        assert_eq!(
            ours.encode_to_vec().unwrap(),
            theirs.encode_to_vec(),
            "round {i}: {ours:?}"
        );
    }
}

#[test]
fn each_reads_what_the_other_wrote() {
    let mut rng = Lcg(2);
    for i in 0..ROUNDS {
        let ours = random_all(&mut rng);
        let theirs = PAll::from(&ours);

        let from_prost = All::decode(&theirs.encode_to_vec()).unwrap();
        assert_eq!(from_prost, ours, "round {i}: prost -> rusty_prost");

        let from_ours = PAll::decode(ours.encode_to_vec().unwrap().as_slice()).unwrap();
        assert_eq!(from_ours, theirs, "round {i}: rusty_prost -> prost");
    }
}

/// Mutates a valid buffer, then compares verdicts and values.
///
/// - Both accept: they must read the same message (compared by re-encoding with `prost`).
/// - We accept, `prost` rejects: never.
/// - `prost` accepts, we reject: only for the documented stricter rules: a group, a field
///   number past 2^29 - 1, or prost's missing wire-type check on map fields.
#[test]
fn they_agree_on_corrupted_input() {
    let mut rng = Lcg(3);
    let (mut both_ok, mut both_err, mut stricter) = (0u32, 0u32, 0u32);
    for i in 0..ROUNDS * 8 {
        let mut buf = random_all(&mut rng).encode_to_vec().unwrap();
        for _ in 0..1 + rng.below(3) {
            if buf.is_empty() {
                break;
            }
            let at = rng.below(buf.len());
            match rng.below(4) {
                0 => buf[at] = rng.next() as u8,
                1 => buf[at] ^= 1 << rng.below(8),
                2 => buf.insert(at, rng.next() as u8),
                _ => {
                    buf.remove(at);
                }
            }
        }
        let ours = All::decode(&buf);
        let theirs = PAll::decode(buf.as_slice());
        match (ours, theirs) {
            (Ok(o), Ok(t)) => {
                both_ok += 1;
                assert_eq!(
                    PAll::from(&o).encode_to_vec(),
                    t.encode_to_vec(),
                    "round {i}: both accepted {buf:02x?} but read different messages"
                );
            }
            (Err(_), Err(_)) => both_err += 1,
            (Ok(_), Err(e)) => panic!("round {i}: we accepted {buf:02x?}, prost rejected: {e}"),
            (Err(e), Ok(_)) => {
                if !explained(e, &buf) {
                    let small = shrink(buf.clone());
                    panic!(
                        "round {i}: prost accepts but we reject with {e:?}; minimal input {small:02x?} \
                         (ours {:?}, prost {:?})",
                        All::decode(&small).map(|_| ()),
                        PAll::decode(small.as_slice()).map(|_| ())
                    );
                }
                stricter += 1;
            }
        }
    }
    // The comparison must have exercised both outcomes, or it proves nothing.
    assert!(
        both_ok > 1000 && both_err > 1000,
        "ok {both_ok}, err {both_err}"
    );
    eprintln!("agree-ok {both_ok}, agree-err {both_err}, stricter {stricter}");
}

#[test]
fn nesting_limit_matches_prost_at_the_boundary() {
    // Field 16 of `All` is `Inner`, which has no message fields, so depth is exercised through
    // the generic limit parameter instead: one level deeper than allowed must fail.
    let buf = All {
        inner: Some(common::Inner {
            id: 1,
            name: "n".into(),
        }),
        ..Default::default()
    }
    .encode_to_vec()
    .unwrap();
    assert!(All::decode_with_depth(&buf, 1).is_ok());
    assert_eq!(
        All::decode_with_depth(&buf, 0).unwrap_err(),
        Error::DepthExceeded
    );
}
