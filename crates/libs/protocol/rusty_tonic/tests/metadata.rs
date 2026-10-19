//! Metadata: validated keys and values, and how they go on and off the wire.
#![allow(clippy::unwrap_used)]

mod common;

use common::{h, Lcg};
use rusty_tonic::{MetadataError, MetadataMap, ValueRef};

fn map(pairs: &[(&str, &str)]) -> MetadataMap {
    let mut m = MetadataMap::new();
    for (k, v) in pairs {
        m.append(k, v).unwrap();
    }
    m
}

#[test]
fn insert_replaces_and_append_keeps() {
    let mut m = MetadataMap::new();
    m.append("x-k", "a").unwrap();
    m.append("x-k", "b").unwrap();
    assert_eq!(m.get("x-k"), Some("a"));
    assert_eq!(m.get_all("x-k").collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(m.len(), 2);
    m.insert("x-k", "c").unwrap();
    assert_eq!(m.get_all("x-k").collect::<Vec<_>>(), ["c"]);
    assert_eq!(m.len(), 1);
    assert!(m.contains_key("x-k"));
    m.remove("x-k");
    assert!(m.is_empty() && !m.contains_key("x-k") && m.get("x-k").is_none());
}

#[test]
fn order_is_the_order_added() {
    let m = map(&[("b", "1"), ("a", "2"), ("b", "3")]);
    let seen: Vec<_> = m.iter().map(|(k, v)| (k.to_string(), v)).collect();
    assert_eq!(
        seen,
        [
            ("b".to_string(), ValueRef::Ascii("1")),
            ("a".to_string(), ValueRef::Ascii("2")),
            ("b".to_string(), ValueRef::Ascii("3")),
        ]
    );
}

#[test]
fn keys_are_lowercase_visible_names() {
    let mut m = MetadataMap::new();
    for ok in [
        "a",
        "x-trace-id",
        "a2a-version",
        "a.b_c-1",
        "authorization",
        "x-grpc",
        "grpcx",
    ] {
        m.insert(ok, "v").unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for bad in [
        "",
        "Foo",
        "a b",
        "a:b",
        "\u{fc}ber",
        "a\n",
        "a/b",
        "A",
        ":path",
    ] {
        assert_eq!(
            m.insert(bad, "v"),
            Err(MetadataError::InvalidKey),
            "{bad:?}"
        );
    }
}

#[test]
fn protocol_names_are_reserved() {
    let mut m = MetadataMap::new();
    for reserved in [
        "grpc-status",
        "grpc-message",
        "grpc-timeout",
        "grpc-anything",
        "content-type",
        "te",
        "user-agent",
        "host",
        "content-length",
        "content-encoding",
        "accept-encoding",
        "connection",
        "transfer-encoding",
        "keep-alive",
        "upgrade",
        "trailer",
    ] {
        assert_eq!(
            m.insert(reserved, "v"),
            Err(MetadataError::ReservedKey),
            "{reserved}"
        );
    }
}

#[test]
fn ascii_values_are_printable_ascii_and_may_be_empty() {
    let mut m = MetadataMap::new();
    m.insert("k", "").unwrap();
    m.insert("k", " ~ plain, with: punctuation! ").unwrap();
    for bad in ["a\nb", "tab\t", "del\u{7f}", "caf\u{e9}", "nul\0"] {
        assert_eq!(
            m.insert("k", bad),
            Err(MetadataError::InvalidValue),
            "{bad:?}"
        );
    }
}

#[test]
fn binary_values_need_a_bin_key_and_ascii_values_do_not_get_one() {
    let mut m = MetadataMap::new();
    m.insert_bin("trace-bin", &[0, 1, 0xff]).unwrap();
    assert_eq!(m.get_bin("trace-bin"), Some(&[0u8, 1, 0xff][..]));
    assert_eq!(
        m.get("trace-bin"),
        None,
        "an ascii read of a binary value finds nothing"
    );
    assert_eq!(m.insert("other-bin", "x"), Err(MetadataError::WrongKind));
    assert_eq!(m.insert_bin("other", &[1]), Err(MetadataError::WrongKind));
    assert_eq!(
        m.append_bin("grpc-status-details-bin", &[1]),
        Err(MetadataError::ReservedKey)
    );
    m.append_bin("trace-bin", &[2]).unwrap();
    assert_eq!(m.len(), 2);
}

#[test]
fn a_rejected_call_changes_nothing() {
    let mut m = map(&[("a", "1")]);
    let before = m.clone();
    assert!(m.insert("A", "x").is_err());
    assert!(m.append("b", "bad\n").is_err());
    assert!(m.insert_bin("b", &[1]).is_err());
    assert_eq!(m, before);
}

#[test]
fn headers_carry_ascii_as_is_and_binary_as_unpadded_base64() {
    let mut m = map(&[("a", "1")]);
    m.insert_bin("b-bin", &[0xff, 0xfe]).unwrap();
    m.insert_bin("c-bin", &[]).unwrap();
    m.insert_bin("d-bin", b"hello").unwrap();
    assert_eq!(
        m.to_headers(),
        [
            ("a".to_string(), b"1".to_vec()),
            ("b-bin".to_string(), b"//4".to_vec()),
            ("c-bin".to_string(), Vec::new()),
            ("d-bin".to_string(), b"aGVsbG8".to_vec()),
        ]
    );
}

#[test]
fn what_we_send_we_read_back() {
    let mut m = map(&[("a", "1"), ("a", "2"), ("z", "")]);
    m.insert_bin("b-bin", &[0, 255, 7]).unwrap();
    m.append_bin("b-bin", &[1]).unwrap();
    let headers = m.to_headers();
    let borrowed = common::borrowed(&headers);
    let (back, dropped) = MetadataMap::from_headers(borrowed);
    assert_eq!(back, m);
    assert_eq!(dropped, 0);
}

#[test]
fn padded_and_unpadded_binary_values_are_both_read() {
    let (m, dropped) = MetadataMap::from_headers([
        h("a-bin", "//4="),
        h("b-bin", "//4"),
        h("c-bin", "aGVsbG8="),
        h("d-bin", "aGVsbG8"),
    ]);
    assert_eq!(dropped, 0);
    assert_eq!(m.get_bin("a-bin"), Some(&[0xff, 0xfe][..]));
    assert_eq!(m.get_bin("b-bin"), Some(&[0xff, 0xfe][..]));
    assert_eq!(m.get_bin("c-bin"), Some(&b"hello"[..]));
    assert_eq!(m.get_bin("d-bin"), Some(&b"hello"[..]));
}

#[test]
fn received_headers_that_cannot_be_metadata_are_skipped_and_counted() {
    let (m, dropped) = MetadataMap::from_headers([
        h("ok", "v"),
        h("Upper", "x"),
        h("sp ace", "x"),
        h("bad-value", "a\nb"),
        h("bad-bin", "!!!"),
        h("also-ok", "w"),
    ]);
    assert_eq!(m.len(), 2);
    assert_eq!((m.get("ok"), m.get("also-ok")), (Some("v"), Some("w")));
    assert_eq!(dropped, 4);
}

#[test]
fn protocol_headers_among_received_ones_are_ignored_without_being_counted() {
    let (m, dropped) = MetadataMap::from_headers([
        h("content-type", "application/grpc"),
        h("grpc-foo", "1"),
        h("te", "trailers"),
        h("x-real", "v"),
    ]);
    assert_eq!(m.len(), 1);
    assert_eq!(dropped, 0);
}

#[test]
fn received_non_ascii_bytes_are_skipped() {
    let (m, dropped) = MetadataMap::from_headers([("k", &b"caf\xc3\xa9"[..]), ("j", b"fine")]);
    assert_eq!((m.len(), dropped), (1, 1));
}

#[test]
fn random_headers_never_panic() {
    let mut rng = Lcg(6);
    let names = ["a", "b-bin", "Bad", "", "grpc-x", "x y"];
    for _ in 0..20_000 {
        let owned: Vec<(String, Vec<u8>)> = (0..rng.below(6))
            .map(|_| (names[rng.below(names.len())].to_string(), rng.bytes(20)))
            .collect();
        let (m, dropped) = MetadataMap::from_headers(common::borrowed(&owned));
        assert!(m.len() + dropped <= owned.len());
        let _ = m.to_headers();
    }
}
