//! The `ext_mcp` messages against vectors produced by Google's Python protobuf runtime
//! (`tests/golden/README.md` says how they are made): every message of `proto/ext_mcp.proto`
//! decodes and re-encodes to the reference's bytes, whatever order, repetition or unknown fields
//! the input has, and hostile bytes are an error rather than a panic.
#![allow(clippy::unwrap_used)]

use agentgateway_mcp::{
    AuthorizationError, HeaderMutation, McpHeader, McpRequest, McpRequestResult, McpResponse,
    McpResponseResult, Pass,
};
use rusty_prost::Message;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// `(type, name, input, canonical)` for each vector.
fn vectors() -> Vec<(String, String, Vec<u8>, Vec<u8>)> {
    let file: serde_json::Value =
        serde_json::from_str(include_str!("golden/ext_mcp_vectors.json")).unwrap();
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap().to_owned();
    file["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                text(v, "type"),
                text(v, "name"),
                hex(&text(v, "input")),
                hex(&text(v, "canonical")),
            )
        })
        .collect()
}

macro_rules! messages {
    ($($proto:literal => $ty:ty),* $(,)?) => {
        const NAMES: &[&str] = &[$($proto),*];

        /// Decodes `bytes` as the named message and encodes it back.
        fn reencode(name: &str, bytes: &[u8]) -> Result<Vec<u8>, rusty_prost::Error> {
            match name {
                $($proto => <$ty>::decode(bytes)?.encode_to_vec(),)*
                other => panic!("no type for {other}"),
            }
        }
    };
}

messages! {
    "McpRequest" => McpRequest,
    "McpResponse" => McpResponse,
    "McpRequestResult" => McpRequestResult,
    "McpResponseResult" => McpResponseResult,
    "HeaderMutation" => HeaderMutation,
    "McpHeader" => McpHeader,
    "Pass" => Pass,
    "AuthorizationError" => AuthorizationError,
}

#[test]
fn every_message_has_vectors() {
    let all = vectors();
    for name in NAMES {
        let n = all.iter().filter(|(t, ..)| t == name).count();
        assert!(n >= 6, "{name} has {n} vectors");
    }
    assert!(all.iter().all(|(t, ..)| NAMES.contains(&t.as_str())));
}

#[test]
fn every_vector_decodes_and_reencodes_to_the_reference_bytes() {
    for (ty, name, input, canonical) in vectors() {
        assert_eq!(reencode(&ty, &input).unwrap(), canonical, "{ty}: {name}");
    }
}

#[test]
fn the_reference_bytes_are_a_fixed_point() {
    for (ty, name, _, canonical) in vectors() {
        assert_eq!(
            reencode(&ty, &canonical).unwrap(),
            canonical,
            "{ty}: {name}"
        );
    }
}

#[test]
fn a_truncated_vector_is_an_error_or_a_message_never_a_panic() {
    for (ty, _, input, _) in vectors() {
        for end in 0..input.len() {
            let _ = reencode(&ty, &input[..end]);
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for name in NAMES {
        for _ in 0..500 {
            let len = (next() % 64) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = reencode(name, &bytes);
        }
    }
}

#[test]
fn a_string_that_is_not_utf8_is_refused() {
    // `method` (field 2) holding 0xff.
    assert!(McpRequest::decode(&[0x12, 0x01, 0xff]).is_err());
}

#[test]
fn a_length_past_the_end_is_refused() {
    assert!(McpRequest::decode(&[0x12, 0x09, b'x']).is_err());
}
