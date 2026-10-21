//! Our trailers read by `tonic`, and `tonic`'s read by us. (`tonic` is a dev-dependency oracle.)
#![allow(clippy::unwrap_used)]

mod common;

use common::{borrowed, Lcg};
use rusty_prost::rpc::ErrorInfo;
use rusty_prost::wkt::Any;
use rusty_prost::Message;
use rusty_tonic::{Code, Status};
use tonic::codegen::http::{HeaderMap, HeaderName, HeaderValue};
use tonic::codegen::Bytes;

fn map(pairs: &[(String, Vec<u8>)]) -> HeaderMap {
    let mut m = HeaderMap::new();
    for (k, v) in pairs {
        m.append(
            HeaderName::from_bytes(k.as_bytes()).unwrap(),
            HeaderValue::from_bytes(v).unwrap(),
        );
    }
    m
}

fn pairs(m: &HeaderMap) -> Vec<(String, Vec<u8>)> {
    m.iter()
        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
        .collect()
}

const MESSAGES: &[&str] = &[
    "",
    "task not found",
    "h\u{e9}llo \u{2603}",
    "100% sure",
    "new\nline",
    "a b",
    "%2F",
    "\u{1f980}",
];

fn random_status(rng: &mut Lcg) -> Status {
    let msg = MESSAGES[rng.below(MESSAGES.len())];
    let code = Code::from_i32(rng.below(17) as i32);
    let mut s = Status::new(code, msg);
    if rng.chance() {
        let info = ErrorInfo {
            reason: "R".into(),
            domain: "d".into(),
            metadata: [("k".to_string(), "v".to_string())].into(),
        };
        s = s.with_details(vec![Any::pack(&info).unwrap()]);
    }
    if rng.chance() {
        let mut md = rusty_tonic::MetadataMap::new();
        md.insert("x-trace-id", "abc").unwrap();
        s = s.with_metadata(md);
    }
    s
}

/// What `tonic` should see as `details()`: the `google.rpc.Status` we put in the header.
fn rpc_status_bytes(s: &Status) -> Vec<u8> {
    rusty_prost::rpc::Status {
        code: s.code().as_i32(),
        message: s.message().to_string(),
        details: s.details().to_vec(),
    }
    .encode_to_vec()
    .unwrap()
}

#[test]
fn tonic_reads_our_trailers() {
    let mut rng = Lcg(1);
    for i in 0..2000 {
        let ours = random_status(&mut rng);
        let theirs = tonic::Status::from_header_map(&map(&ours.to_trailers())).unwrap();
        assert_eq!(theirs.code() as i32, ours.code().as_i32(), "{i}: {ours:?}");
        assert_eq!(theirs.message(), ours.message(), "{i}");
        let want = if ours.code() == Code::Ok || ours.details().is_empty() {
            Vec::new()
        } else {
            rpc_status_bytes(&ours)
        };
        assert_eq!(theirs.details(), want.as_slice(), "{i}");
        for (k, v) in ours.metadata().iter() {
            let rusty_tonic::ValueRef::Ascii(v) = v else {
                continue;
            };
            assert_eq!(
                theirs.metadata().get(k).and_then(|x| x.to_str().ok()),
                Some(v),
                "{i}"
            );
        }
    }
}

#[test]
fn we_read_tonics_trailers() {
    let mut rng = Lcg(2);
    for i in 0..2000 {
        let msg = MESSAGES[rng.below(MESSAGES.len())];
        let code = tonic::Code::from_i32(rng.below(17) as i32);
        let details = if rng.chance() {
            let info = ErrorInfo {
                reason: "R".into(),
                domain: "d".into(),
                ..Default::default()
            };
            let rpc = rusty_prost::rpc::Status {
                code: code as i32,
                message: msg.to_string(),
                details: vec![Any::pack(&info).unwrap()],
            };
            Some(rpc.encode_to_vec().unwrap())
        } else {
            None
        };
        let theirs = match &details {
            Some(d) => tonic::Status::with_details(code, msg, Bytes::from(d.clone())),
            None => tonic::Status::new(code, msg),
        };
        let mut m = HeaderMap::new();
        theirs.add_header(&mut m).unwrap();
        let ours = Status::from_headers(borrowed(&pairs(&m))).unwrap();
        assert_eq!(ours.code().as_i32(), code as i32, "{i}");
        assert_eq!(ours.message(), msg, "{i}");
        let keeps_details = details.is_some() && code != tonic::Code::Ok;
        assert_eq!(ours.details().len(), usize::from(keeps_details), "{i}");
        assert_eq!(ours.error_info().is_some(), keeps_details, "{i}");
    }
}
