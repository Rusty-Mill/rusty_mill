//! Codes, statuses, and their trailers.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::{borrowed, h, value, Lcg};
use rusty_prost::rpc::ErrorInfo;
use rusty_prost::wkt::Any;
use rusty_prost::Message;
use rusty_tonic::{Code, MetadataMap, Status};

#[test]
fn every_code_round_trips_its_number() {
    for n in 0..=16 {
        assert_eq!(Code::from_i32(n).as_i32(), n);
    }
}

#[test]
fn a_number_outside_the_codes_is_unknown() {
    for n in [-1, 17, 18, 100, i32::MAX, i32::MIN] {
        assert_eq!(Code::from_i32(n), Code::Unknown, "{n}");
    }
}

#[test]
fn codes_have_their_canonical_names() {
    assert_eq!(Code::NotFound.to_string(), "NOT_FOUND");
    assert_eq!(Code::Ok.name(), "OK");
    assert_eq!(Code::DeadlineExceeded.name(), "DEADLINE_EXCEEDED");
    assert_eq!(Code::Unauthenticated.name(), "UNAUTHENTICATED");
}

#[test]
fn constructors_set_the_code_and_message() {
    let s = Status::not_found("task t-1");
    assert_eq!((s.code(), s.message()), (Code::NotFound, "task t-1"));
    assert_eq!(Status::unauthenticated("x").code(), Code::Unauthenticated);
    assert_eq!(
        Status::permission_denied("x").code(),
        Code::PermissionDenied
    );
    assert_eq!(
        Status::failed_precondition("x").code(),
        Code::FailedPrecondition
    );
    assert_eq!(Status::invalid_argument("x").code(), Code::InvalidArgument);
    assert_eq!(Status::unimplemented("x").code(), Code::Unimplemented);
    assert_eq!(Status::internal("x").code(), Code::Internal);
    assert_eq!(Status::unavailable("x").code(), Code::Unavailable);
    assert_eq!(
        Status::deadline_exceeded("x").code(),
        Code::DeadlineExceeded
    );
    assert_eq!(
        Status::resource_exhausted("x").code(),
        Code::ResourceExhausted
    );
    assert_eq!(Status::ok().code(), Code::Ok);
}

#[test]
fn display_shows_the_code_and_the_message() {
    assert_eq!(Status::not_found("gone").to_string(), "NOT_FOUND: gone");
    assert_eq!(Status::cancelled("").to_string(), "CANCELLED");
}

// ---- trailers ----

#[test]
fn trailers_carry_the_code_and_a_readable_message() {
    let t = Status::not_found("task not found").to_trailers();
    assert_eq!(
        t,
        [
            ("grpc-status".to_string(), b"5".to_vec()),
            ("grpc-message".to_string(), b"task not found".to_vec()),
        ]
    );
}

#[test]
fn ok_with_no_message_is_just_the_status() {
    assert_eq!(
        Status::ok().to_trailers(),
        [("grpc-status".to_string(), b"0".to_vec())]
    );
}

#[test]
fn the_message_is_percent_encoded_outside_printable_ascii_and_percent() {
    let t = Status::internal("100% sure\nh\u{e9} \u{2603}").to_trailers();
    assert_eq!(
        value(&t, "grpc-message"),
        Some("100%25 sure%0Ah%C3%A9 %E2%98%83")
    );
    // every other printable ASCII byte is left as it is
    let printable: String = (0x20u8..=0x7e)
        .filter(|b| *b != b'%')
        .map(char::from)
        .collect();
    assert_eq!(
        value(
            &Status::internal(printable.clone()).to_trailers(),
            "grpc-message"
        ),
        Some(printable.as_str())
    );
}

#[test]
fn a_message_survives_the_trip_through_trailers() {
    for msg in [
        "",
        "plain",
        "100%",
        "a%2Fb",
        "new\nline",
        "h\u{e9}llo \u{2603}",
        "\u{1f980}",
        "tab\there",
    ] {
        let t = Status::aborted(msg).to_trailers();
        let back = Status::from_headers(borrowed(&t)).unwrap();
        assert_eq!(
            (back.code(), back.message()),
            (Code::Aborted, msg),
            "{msg:?}"
        );
    }
}

#[test]
fn no_grpc_status_means_no_status() {
    assert!(Status::from_headers([h("grpc-message", "x"), h("x-other", "y")]).is_none());
    assert!(Status::from_headers(std::iter::empty()).is_none());
}

#[test]
fn an_unparseable_grpc_status_is_unknown() {
    for v in [
        "",
        "abc",
        "-1",
        "17",
        "99999999999999999999",
        " 5",
        "5 ",
        "+5",
        "5.0",
        "\u{ff}",
    ] {
        let s = Status::from_headers([h("grpc-status", v)]).unwrap();
        assert_eq!(s.code(), Code::Unknown, "{v:?}");
    }
    for (v, want) in [
        ("0", Code::Ok),
        ("5", Code::NotFound),
        ("16", Code::Unauthenticated),
    ] {
        assert_eq!(
            Status::from_headers([h("grpc-status", v)]).unwrap().code(),
            want
        );
    }
}

#[test]
fn a_bad_percent_escape_is_kept_and_bad_utf8_is_replaced() {
    let s = Status::from_headers([h("grpc-status", "2"), h("grpc-message", "50%zz")]).unwrap();
    assert_eq!(s.message(), "50%zz");
    let s = Status::from_headers([h("grpc-status", "2"), h("grpc-message", "%E2%98")]).unwrap();
    assert!(s.message().contains('\u{fffd}'));
}

// ---- details ----

fn info() -> ErrorInfo {
    ErrorInfo {
        reason: "TASK_NOT_FOUND".into(),
        domain: "a2a-protocol.org".into(),
        metadata: BTreeMap::from([("task".to_string(), "t-1".to_string())]),
    }
}

fn with_info() -> Status {
    Status::with_error_info(
        Code::NotFound,
        "task not found",
        "TASK_NOT_FOUND",
        "a2a-protocol.org",
        info().metadata,
    )
}

#[test]
fn error_info_is_a_detail_and_comes_back_out() {
    let s = with_info();
    assert_eq!(s.details().len(), 1);
    assert_eq!(
        s.details()[0].type_url,
        "type.googleapis.com/google.rpc.ErrorInfo"
    );
    assert_eq!(s.error_info(), Some(info()));
    assert_eq!(Status::not_found("x").error_info(), None);
}

#[test]
fn details_travel_as_unpadded_base64_of_a_google_rpc_status() {
    let t = with_info().to_trailers();
    let bin = value(&t, "grpc-status-details-bin").unwrap();
    assert!(!bin.contains('='), "padding is omitted on send: {bin}");
    let raw = rusty_base64::decode_standard_lenient(bin.as_bytes()).unwrap();
    let rpc = rusty_prost::rpc::Status::decode(&raw).unwrap();
    assert_eq!((rpc.code, rpc.message.as_str()), (5, "task not found"));
    assert_eq!(rpc.details[0].unpack::<ErrorInfo>().unwrap(), Some(info()));
}

#[test]
fn no_details_means_no_details_header() {
    assert!(value(
        &Status::not_found("x").to_trailers(),
        "grpc-status-details-bin"
    )
    .is_none());
}

#[test]
fn details_round_trip_through_trailers() {
    let back = Status::from_headers(borrowed(&with_info().to_trailers())).unwrap();
    assert_eq!(back, with_info());
    assert_eq!(back.error_info(), Some(info()));
}

#[test]
fn padded_details_are_accepted_too() {
    let t = with_info().to_trailers();
    let mut bin = value(&t, "grpc-status-details-bin").unwrap().to_string();
    while bin.len() % 4 != 0 {
        bin.push('=');
    }
    let back = Status::from_headers([
        h("grpc-status", "5"),
        h("grpc-message", "task not found"),
        h("grpc-status-details-bin", &bin),
    ])
    .unwrap();
    assert_eq!(back.error_info(), Some(info()));
}

#[test]
fn malformed_details_are_dropped_and_the_call_keeps_its_code_and_message() {
    // not base64; base64 that is not a google.rpc.Status; a truncated one
    for bad in [
        "!!!",
        "////",
        "CAUS",
        "ChoKFHR5cGUuZ29vZ2xlYXBpcy5jb20vZxICAQ",
    ] {
        let s = Status::from_headers([
            h("grpc-status", "5"),
            h("grpc-message", "m"),
            h("grpc-status-details-bin", bad),
        ])
        .unwrap();
        assert_eq!((s.code(), s.message()), (Code::NotFound, "m"), "{bad}");
        assert!(s.details().is_empty(), "{bad}");
    }
}

#[test]
fn the_header_code_wins_over_the_code_inside_the_details() {
    let mut t = with_info().to_trailers();
    t.iter_mut().find(|(k, _)| k == "grpc-status").unwrap().1 = b"14".to_vec();
    let s = Status::from_headers(borrowed(&t)).unwrap();
    assert_eq!(s.code(), Code::Unavailable);
    assert_eq!(s.message(), "task not found");
}

#[test]
fn an_error_info_detail_that_does_not_decode_is_skipped() {
    let broken = Any {
        type_url: "type.googleapis.com/google.rpc.ErrorInfo".into(),
        value: vec![0x0a, 0x09],
    };
    let good = Any::pack(&info()).unwrap();
    let s = Status::new(Code::NotFound, "m").with_details(vec![broken, good]);
    assert_eq!(s.error_info(), Some(info()));
    let only_broken = Status::new(Code::NotFound, "m").with_details(vec![Any {
        type_url: "x/google.rpc.ErrorInfo".into(),
        value: vec![0x0a, 0x09],
    }]);
    assert_eq!(only_broken.error_info(), None);
}

// ---- metadata in trailers ----

#[test]
fn trailing_metadata_goes_after_the_status_and_comes_back() {
    let mut md = MetadataMap::new();
    md.insert("x-trace-id", "abc").unwrap();
    md.insert_bin("blob-bin", &[0xff, 0xfe]).unwrap();
    let s = Status::internal("boom").with_metadata(md.clone());
    let t = s.to_trailers();
    assert_eq!(t[0].0, "grpc-status");
    assert_eq!(value(&t, "x-trace-id"), Some("abc"));
    assert_eq!(value(&t, "blob-bin"), Some("//4"));
    let back = Status::from_headers(borrowed(&t)).unwrap();
    assert_eq!(back.metadata(), &md);
}

// ---- hostile ----

#[test]
fn random_headers_never_panic() {
    let mut rng = Lcg(5);
    let names = [
        "grpc-status",
        "grpc-message",
        "grpc-status-details-bin",
        "x-a",
        "trace-bin",
        "Bad Name",
        "",
    ];
    for _ in 0..20_000 {
        let owned: Vec<(String, Vec<u8>)> = (0..rng.below(5))
            .map(|_| (names[rng.below(names.len())].to_string(), rng.bytes(40)))
            .collect();
        if let Some(s) = Status::from_headers(borrowed(&owned)) {
            let _ = s.to_trailers();
            let _ = s.error_info();
            let _ = s.to_string();
        }
    }
}
