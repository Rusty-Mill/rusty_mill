//! The header lists of a request and a response, and the mapping from HTTP statuses.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{borrowed, h, value, Lcg};
use rusty_tonic::{
    code_from_http_status, split_path, Code, MetadataMap, RequestHead, ResponseHead, ResponseStart,
    Scheme, Status,
};

#[test]
fn a_path_is_service_and_method() {
    assert_eq!(split_path("/pkg.Svc/Method"), Some(("pkg.Svc", "Method")));
    assert_eq!(split_path("/Svc/M"), Some(("Svc", "M")));
    assert_eq!(
        split_path("/lf.a2a.v1.A2AService/SendMessage"),
        Some(("lf.a2a.v1.A2AService", "SendMessage"))
    );
    for bad in [
        "",
        "/",
        "pkg.Svc/Method",
        "/pkg.Svc",
        "/pkg.Svc/",
        "//Method",
        "/a/b/c",
        "/a b/c",
        "/a/b c",
        "/\u{e9}/m",
        "/a/m\n",
    ] {
        assert_eq!(split_path(bad), None, "{bad:?}");
    }
}

#[test]
fn http_statuses_map_to_codes_as_the_spec_says() {
    for (http, want) in [
        (400, Code::Internal),
        (401, Code::Unauthenticated),
        (403, Code::PermissionDenied),
        (404, Code::Unimplemented),
        (429, Code::Unavailable),
        (502, Code::Unavailable),
        (503, Code::Unavailable),
        (504, Code::Unavailable),
        (200, Code::Unknown),
        (500, Code::Unknown),
        (418, Code::Unknown),
        (0, Code::Unknown),
    ] {
        assert_eq!(code_from_http_status(http), want, "{http}");
    }
}

fn full_request() -> RequestHead {
    let mut head = RequestHead::new("/lf.a2a.v1.A2AService/SendMessage", "agent.example:443");
    head.scheme = Scheme::Https;
    head.timeout = Some(Duration::from_millis(1500));
    head.user_agent = Some("rusty-tonic/0.1".into());
    let mut md = MetadataMap::new();
    md.insert("a2a-version", "1.0").unwrap();
    md.insert("authorization", "Bearer t").unwrap();
    md.insert_bin("trace-bin", &[1, 2, 3]).unwrap();
    head.metadata = md;
    head
}

#[test]
fn a_request_head_lists_pseudo_headers_first_then_the_protocol_headers() {
    let headers = full_request().to_headers();
    let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(&names[..4], [":method", ":scheme", ":path", ":authority"]);
    assert_eq!(value(&headers, ":method"), Some("POST"));
    assert_eq!(value(&headers, ":scheme"), Some("https"));
    assert_eq!(
        value(&headers, ":path"),
        Some("/lf.a2a.v1.A2AService/SendMessage")
    );
    assert_eq!(value(&headers, ":authority"), Some("agent.example:443"));
    assert_eq!(value(&headers, "content-type"), Some("application/grpc"));
    assert_eq!(value(&headers, "te"), Some("trailers"));
    assert_eq!(value(&headers, "grpc-accept-encoding"), Some("identity"));
    assert_eq!(value(&headers, "grpc-timeout"), Some("1500000u"));
    assert_eq!(value(&headers, "user-agent"), Some("rusty-tonic/0.1"));
    assert_eq!(value(&headers, "a2a-version"), Some("1.0"));
    assert_eq!(value(&headers, "trace-bin"), Some("AQID"));
}

#[test]
fn no_timeout_and_no_user_agent_means_no_such_headers() {
    let headers = RequestHead::new("/s.S/M", "h").to_headers();
    assert!(value(&headers, "grpc-timeout").is_none() && value(&headers, "user-agent").is_none());
    assert_eq!(value(&headers, ":scheme"), Some("http"));
}

#[test]
fn a_request_head_round_trips() {
    let sent = full_request();
    let got = RequestHead::from_headers(borrowed(&sent.to_headers())).unwrap();
    // the timeout comes back rounded up to the unit it was sent in, here exactly
    assert_eq!(got, sent);
    assert_eq!(got.metadata_dropped, 0);
}

fn base() -> Vec<(&'static str, &'static [u8])> {
    vec![
        h(":method", "POST"),
        h(":scheme", "http"),
        h(":path", "/s.S/M"),
        h(":authority", "host"),
        h("content-type", "application/grpc"),
        h("te", "trailers"),
    ]
}

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static [u8])> {
    let mut v = base();
    v.extend(extra.iter().map(|(k, val)| h(k, val)));
    v
}

fn without(name: &str) -> Vec<(&'static str, &'static [u8])> {
    base().into_iter().filter(|(k, _)| *k != name).collect()
}

#[test]
fn a_minimal_call_parses() {
    let head = RequestHead::from_headers(base()).unwrap();
    assert_eq!(
        (head.path.as_str(), head.authority.as_str(), head.scheme),
        ("/s.S/M", "host", Scheme::Http)
    );
    assert_eq!((head.timeout, head.user_agent), (None, None));
    assert!(head.metadata.is_empty());
}

#[test]
fn content_subtypes_other_than_proto_are_unimplemented() {
    for ok in [
        "application/grpc",
        "application/grpc+proto",
        "application/grpc;charset=utf-8",
        "application/grpc+proto;x=1",
    ] {
        let mut v = without("content-type");
        v.push(h("content-type", ok));
        assert!(RequestHead::from_headers(v).is_ok(), "{ok}");
    }
    for bad in [
        "application/json",
        "application/grpcx",
        "application/grpc+json",
        "text/plain",
        "",
    ] {
        let mut v = without("content-type");
        v.push(h("content-type", bad));
        assert_eq!(
            RequestHead::from_headers(v).unwrap_err().code(),
            Code::Unimplemented,
            "{bad:?}"
        );
    }
    assert_eq!(
        RequestHead::from_headers(without("content-type"))
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
}

#[test]
fn only_post_is_a_call_and_the_path_must_be_a_method() {
    let mut get = without(":method");
    get.push(h(":method", "GET"));
    assert_eq!(
        RequestHead::from_headers(get).unwrap_err().code(),
        Code::Internal
    );
    assert_eq!(
        RequestHead::from_headers(without(":method"))
            .unwrap_err()
            .code(),
        Code::Internal
    );
    assert_eq!(
        RequestHead::from_headers(without(":path"))
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
    for bad in ["/", "/a", "/a/b/c", "x"] {
        let mut v = without(":path");
        v.push(h(":path", bad));
        assert_eq!(
            RequestHead::from_headers(v).unwrap_err().code(),
            Code::Unimplemented,
            "{bad}"
        );
    }
}

#[test]
fn timeouts_and_encodings_are_checked() {
    let head = RequestHead::from_headers(with(&[("grpc-timeout", "250m")])).unwrap();
    assert_eq!(head.timeout, Some(Duration::from_millis(250)));
    for bad in ["", "abc", "9999999999S", "5x", "-1S"] {
        assert_eq!(
            RequestHead::from_headers(with(&[("grpc-timeout", bad)]))
                .unwrap_err()
                .code(),
            Code::Internal,
            "{bad:?}"
        );
    }
    assert!(RequestHead::from_headers(with(&[("grpc-encoding", "identity")])).is_ok());
    assert_eq!(
        RequestHead::from_headers(with(&[("grpc-encoding", "gzip")]))
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
    // what a client may offer is not our concern
    assert!(RequestHead::from_headers(with(&[("grpc-accept-encoding", "gzip,identity")])).is_ok());
}

#[test]
fn everything_else_is_metadata_and_protocol_headers_are_not() {
    let head = RequestHead::from_headers(with(&[
        ("user-agent", "grpc-go/1.0"),
        ("accept-encoding", "gzip"),
        ("authorization", "Bearer x"),
        ("a2a-version", "1.0"),
        ("Bad-Case", "x"),
    ]))
    .unwrap();
    assert_eq!(head.user_agent.as_deref(), Some("grpc-go/1.0"));
    assert_eq!(head.metadata.len(), 2);
    assert_eq!(head.metadata.get("authorization"), Some("Bearer x"));
    assert_eq!(head.metadata_dropped, 1);
}

// ---- responses ----

#[test]
fn a_response_head_is_a_200_with_a_content_type() {
    let mut r = ResponseHead::default();
    r.metadata.insert("x-served-by", "n1").unwrap();
    let headers = r.to_headers();
    assert_eq!(value(&headers, ":status"), Some("200"));
    assert_eq!(value(&headers, "content-type"), Some("application/grpc"));
    assert_eq!(value(&headers, "x-served-by"), Some("n1"));
}

#[test]
fn a_normal_response_start_round_trips() {
    let mut r = ResponseHead::default();
    r.metadata.insert("x-served-by", "n1").unwrap();
    let start = ResponseHead::from_headers(200, borrowed(&r.to_headers())).unwrap();
    assert_eq!(start, ResponseStart::Headers(r));
}

#[test]
fn a_response_that_already_carries_a_status_is_trailers_only() {
    let start = ResponseHead::from_headers(
        200,
        [
            h("content-type", "application/grpc"),
            h("grpc-status", "5"),
            h("grpc-message", "gone"),
        ],
    )
    .unwrap();
    let ResponseStart::TrailersOnly(s) = start else {
        panic!("{start:?}")
    };
    assert_eq!((s.code(), s.message()), (Code::NotFound, "gone"));
}

#[test]
fn trailers_only_responses_round_trip_a_status() {
    let sent = Status::unauthenticated("no token");
    let mut headers = vec![
        (":status".to_string(), b"200".to_vec()),
        ("content-type".to_string(), b"application/grpc".to_vec()),
    ];
    headers.extend(sent.to_trailers());
    let ResponseStart::TrailersOnly(got) =
        ResponseHead::from_headers(200, borrowed(&headers)).unwrap()
    else {
        panic!()
    };
    assert_eq!(got, sent);
}

#[test]
fn a_non_200_response_is_mapped_from_its_http_status_unless_it_has_a_grpc_status() {
    assert_eq!(
        ResponseHead::from_headers(503, []).unwrap_err().code(),
        Code::Unavailable
    );
    assert_eq!(
        ResponseHead::from_headers(404, [h("content-type", "text/html")])
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
    let start =
        ResponseHead::from_headers(503, [h("grpc-status", "8"), h("grpc-message", "slow down")])
            .unwrap();
    let ResponseStart::TrailersOnly(s) = start else {
        panic!()
    };
    assert_eq!(s.code(), Code::ResourceExhausted);
}

#[test]
fn a_200_that_is_not_grpc_is_unknown() {
    assert_eq!(
        ResponseHead::from_headers(200, [h("content-type", "text/html")])
            .unwrap_err()
            .code(),
        Code::Unknown
    );
    assert_eq!(
        ResponseHead::from_headers(200, []).unwrap_err().code(),
        Code::Unknown
    );
}

#[test]
fn a_compressed_response_is_unimplemented() {
    let e = ResponseHead::from_headers(
        200,
        [
            h("content-type", "application/grpc"),
            h("grpc-encoding", "gzip"),
        ],
    )
    .unwrap_err();
    assert_eq!(e.code(), Code::Unimplemented);
    assert!(ResponseHead::from_headers(
        200,
        [
            h("content-type", "application/grpc"),
            h("grpc-encoding", "identity")
        ]
    )
    .is_ok());
}

#[test]
fn pseudo_headers_in_a_response_list_are_not_metadata() {
    let start = ResponseHead::from_headers(
        200,
        [
            h(":status", "200"),
            h("content-type", "application/grpc+proto"),
            h("x", "1"),
        ],
    )
    .unwrap();
    let ResponseStart::Headers(r) = start else {
        panic!()
    };
    assert_eq!((r.metadata.len(), r.metadata_dropped), (1, 0));
}

// ---- hostile ----

#[test]
fn random_header_lists_never_panic() {
    let mut rng = Lcg(8);
    let names = [
        ":method",
        ":path",
        "content-type",
        "grpc-timeout",
        "grpc-status",
        "grpc-encoding",
        "x-a",
        "Bad",
        "te",
        "",
    ];
    let ct = [
        "application/grpc",
        "application/grpc+proto",
        "text/html",
        "",
    ];
    for _ in 0..20_000 {
        let owned: Vec<(String, Vec<u8>)> = (0..rng.below(7))
            .map(|_| {
                let name = names[rng.below(names.len())];
                let v = if name == "content-type" {
                    ct[rng.below(ct.len())].as_bytes().to_vec()
                } else {
                    rng.bytes(24)
                };
                (name.to_string(), v)
            })
            .collect();
        let _ = RequestHead::from_headers(borrowed(&owned));
        let _ = ResponseHead::from_headers(rng.below(700) as u16, borrowed(&owned));
    }
}
