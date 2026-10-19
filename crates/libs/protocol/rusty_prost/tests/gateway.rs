//! The agent gateway's `ext_mcp` messages (`agentgateway-mcp/src/guardrails/wire.rs`) written on
//! `rusty_prost`, checked against the `prost`-derived originals. This is the proof that the
//! trait is enough for a real consumer: eight messages, two oneofs, an enum, `Option<bytes>`,
//! and `Struct` metadata.
#![allow(clippy::unwrap_used)]

mod common;

use common::Lcg;
use prost::Message as _;
use rusty_prost::wkt::{Struct, Value};
use rusty_prost::{field, Field, Message, Result, Writer};

/// The originals, copied from `wire.rs` (derive and all) with `prost_types::Struct` as is.
mod oracle {
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    pub struct McpRequest {
        #[prost(string, repeated, tag = "1")]
        pub service_names: Vec<String>,
        #[prost(string, tag = "2")]
        pub method: String,
        #[prost(message, optional, tag = "3")]
        pub metadata_context: Option<prost_types::Struct>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub mcp_request: Option<Vec<u8>>,
        #[prost(message, repeated, tag = "5")]
        pub headers: Vec<McpHeader>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct McpResponse {
        #[prost(string, repeated, tag = "1")]
        pub service_names: Vec<String>,
        #[prost(string, tag = "2")]
        pub method: String,
        #[prost(message, optional, tag = "3")]
        pub metadata_context: Option<prost_types::Struct>,
        #[prost(bytes = "vec", tag = "4")]
        pub mcp_response: Vec<u8>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct McpRequestResult {
        #[prost(oneof = "request_result::Result", tags = "1, 2, 3")]
        pub result: Option<request_result::Result>,
        #[prost(message, optional, tag = "4")]
        pub header_mutation: Option<HeaderMutation>,
        #[prost(message, optional, tag = "5")]
        pub metadata: Option<prost_types::Struct>,
    }

    pub mod request_result {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Result {
            #[prost(message, tag = "1")]
            Pass(super::Pass),
            #[prost(bytes = "vec", tag = "2")]
            Mutated(Vec<u8>),
            #[prost(message, tag = "3")]
            Error(super::AuthorizationError),
        }
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct McpResponseResult {
        #[prost(oneof = "response_result::Result", tags = "1, 2, 3")]
        pub result: Option<response_result::Result>,
    }

    pub mod response_result {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Result {
            #[prost(message, tag = "1")]
            Pass(super::Pass),
            #[prost(bytes = "vec", tag = "2")]
            Mutated(Vec<u8>),
            #[prost(message, tag = "3")]
            Error(super::AuthorizationError),
        }
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct HeaderMutation {
        #[prost(message, repeated, tag = "1")]
        pub set: Vec<McpHeader>,
        #[prost(string, repeated, tag = "2")]
        pub remove: Vec<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct McpHeader {
        #[prost(string, tag = "1")]
        pub key: String,
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Message)]
    pub struct Pass {}

    #[derive(Clone, PartialEq, Message)]
    pub struct AuthorizationError {
        #[prost(int32, tag = "1")]
        pub code: i32,
        #[prost(string, tag = "2")]
        pub reason: String,
        #[prost(bytes = "vec", optional, tag = "3")]
        pub mcp_error: Option<Vec<u8>>,
    }
}

/// The same messages on `rusty_prost`.
mod ours {
    use super::*;

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct McpHeader {
        pub key: String,
        pub value: Vec<u8>,
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct Pass;

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct AuthorizationError {
        pub code: i32,
        pub reason: String,
        pub mcp_error: Option<Vec<u8>>,
    }

    /// The `result` oneof, which both result messages share.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Outcome {
        Pass(Pass),
        Mutated(Vec<u8>),
        Error(AuthorizationError),
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct HeaderMutation {
        pub set: Vec<McpHeader>,
        pub remove: Vec<String>,
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct McpRequest {
        pub service_names: Vec<String>,
        pub method: String,
        pub metadata_context: Option<Struct>,
        pub mcp_request: Option<Vec<u8>>,
        pub headers: Vec<McpHeader>,
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct McpResponse {
        pub service_names: Vec<String>,
        pub method: String,
        pub metadata_context: Option<Struct>,
        pub mcp_response: Vec<u8>,
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct McpRequestResult {
        pub result: Option<Outcome>,
        pub header_mutation: Option<HeaderMutation>,
        pub metadata: Option<Struct>,
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct McpResponseResult {
        pub result: Option<Outcome>,
    }

    impl Message for McpHeader {
        fn encode(&self, w: &mut Writer) {
            if !self.key.is_empty() {
                w.string(1, &self.key);
            }
            if !self.value.is_empty() {
                w.bytes(2, &self.value);
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            match f.number {
                1 => self.key = f.str()?.to_owned(),
                2 => self.value = f.bytes()?.to_vec(),
                _ => {}
            }
            Ok(())
        }
    }

    impl Message for Pass {
        fn encode(&self, _: &mut Writer) {}
        fn merge_field(&mut self, _: Field<'_>) -> Result<()> {
            Ok(())
        }
    }

    impl Message for AuthorizationError {
        fn encode(&self, w: &mut Writer) {
            if self.code != 0 {
                w.int32(1, self.code);
            }
            if !self.reason.is_empty() {
                w.string(2, &self.reason);
            }
            if let Some(e) = &self.mcp_error {
                w.bytes(3, e);
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            match f.number {
                1 => self.code = f.int32()?,
                2 => self.reason = f.str()?.to_owned(),
                3 => self.mcp_error = Some(f.bytes()?.to_vec()),
                _ => {}
            }
            Ok(())
        }
    }

    impl Outcome {
        /// Writes the member under its field number; a message member is written even if empty.
        fn encode(&self, w: &mut Writer) {
            match self {
                Outcome::Pass(p) => w.message(1, |w| p.encode(w)),
                Outcome::Mutated(b) => w.bytes(2, b),
                Outcome::Error(e) => w.message(3, |w| e.encode(w)),
            }
        }

        /// Reads field 1, 2 or 3 into the oneof; any other number is not a member. A message
        /// member that repeats merges into the one already there; a different member replaces it.
        fn merge_field(slot: &mut Option<Outcome>, f: &Field<'_>) -> Result<bool> {
            match f.number {
                1 => {
                    let mut p = match slot.take() {
                        Some(Outcome::Pass(p)) => Some(p),
                        _ => None,
                    };
                    field::message(&mut p, f)?;
                    *slot = Some(Outcome::Pass(p.unwrap_or_default()));
                }
                2 => *slot = Some(Outcome::Mutated(f.bytes()?.to_vec())),
                3 => {
                    let mut e = match slot.take() {
                        Some(Outcome::Error(e)) => Some(e),
                        _ => None,
                    };
                    field::message(&mut e, f)?;
                    *slot = Some(Outcome::Error(e.unwrap_or_default()));
                }
                _ => return Ok(false),
            }
            Ok(true)
        }
    }

    impl Message for HeaderMutation {
        fn encode(&self, w: &mut Writer) {
            for h in &self.set {
                w.message(1, |w| h.encode(w));
            }
            for r in &self.remove {
                w.string(2, r);
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            match f.number {
                1 => field::push_message(&mut self.set, &f)?,
                2 => self.remove.push(f.str()?.to_owned()),
                _ => {}
            }
            Ok(())
        }
    }

    impl Message for McpRequest {
        fn encode(&self, w: &mut Writer) {
            for s in &self.service_names {
                w.string(1, s);
            }
            if !self.method.is_empty() {
                w.string(2, &self.method);
            }
            if let Some(m) = &self.metadata_context {
                w.message(3, |w| m.encode(w));
            }
            if let Some(b) = &self.mcp_request {
                w.bytes(4, b);
            }
            for h in &self.headers {
                w.message(5, |w| h.encode(w));
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            match f.number {
                1 => self.service_names.push(f.str()?.to_owned()),
                2 => self.method = f.str()?.to_owned(),
                3 => field::message(&mut self.metadata_context, &f)?,
                4 => self.mcp_request = Some(f.bytes()?.to_vec()),
                5 => field::push_message(&mut self.headers, &f)?,
                _ => {}
            }
            Ok(())
        }
    }

    impl Message for McpResponse {
        fn encode(&self, w: &mut Writer) {
            for s in &self.service_names {
                w.string(1, s);
            }
            if !self.method.is_empty() {
                w.string(2, &self.method);
            }
            if let Some(m) = &self.metadata_context {
                w.message(3, |w| m.encode(w));
            }
            if !self.mcp_response.is_empty() {
                w.bytes(4, &self.mcp_response);
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            match f.number {
                1 => self.service_names.push(f.str()?.to_owned()),
                2 => self.method = f.str()?.to_owned(),
                3 => field::message(&mut self.metadata_context, &f)?,
                4 => self.mcp_response = f.bytes()?.to_vec(),
                _ => {}
            }
            Ok(())
        }
    }

    impl Message for McpRequestResult {
        fn encode(&self, w: &mut Writer) {
            if let Some(o) = &self.result {
                o.encode(w);
            }
            if let Some(h) = &self.header_mutation {
                w.message(4, |w| h.encode(w));
            }
            if let Some(m) = &self.metadata {
                w.message(5, |w| m.encode(w));
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            if Outcome::merge_field(&mut self.result, &f)? {
                return Ok(());
            }
            match f.number {
                4 => field::message(&mut self.header_mutation, &f)?,
                5 => field::message(&mut self.metadata, &f)?,
                _ => {}
            }
            Ok(())
        }
    }

    impl Message for McpResponseResult {
        fn encode(&self, w: &mut Writer) {
            if let Some(o) = &self.result {
                o.encode(w);
            }
        }
        fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
            Outcome::merge_field(&mut self.result, &f)?;
            Ok(())
        }
    }
}

use ours::*;

// ---- generator ----

fn text(rng: &mut Lcg) -> String {
    ["", "alpha", "tools/call", "h\u{e9}llo", &"z".repeat(129)][rng.below(5)].into()
}

fn blob(rng: &mut Lcg) -> Vec<u8> {
    (0..rng.below(140)).map(|_| rng.next() as u8).collect()
}

fn metadata(rng: &mut Lcg) -> Option<Struct> {
    rng.chance().then(|| Struct {
        fields: (0..rng.below(4))
            .map(|i| (format!("key{i}"), Value::String(text(rng))))
            .collect(),
    })
}

fn headers(rng: &mut Lcg) -> Vec<McpHeader> {
    (0..rng.below(4))
        .map(|_| McpHeader {
            key: text(rng),
            value: blob(rng),
        })
        .collect()
}

fn outcome(rng: &mut Lcg) -> Option<Outcome> {
    match rng.below(4) {
        0 => None,
        1 => Some(Outcome::Pass(Pass)),
        2 => Some(Outcome::Mutated(blob(rng))),
        _ => Some(Outcome::Error(AuthorizationError {
            code: rng.below(4) as i32,
            reason: text(rng),
            mcp_error: rng.chance().then(|| blob(rng)),
        })),
    }
}

/// Encodes `m`, then checks `prost` reads the bytes and writes the same ones back (so it
/// parsed them as we meant them), and that we read our own bytes back.
fn agrees<M, O>(m: &M)
where
    M: Message + PartialEq + std::fmt::Debug,
    O: prost::Message + Default,
{
    let bytes = m.encode_to_vec().unwrap();
    let theirs = O::decode(bytes.as_slice()).unwrap();
    assert_eq!(
        theirs.encode_to_vec(),
        bytes,
        "prost re-encodes differently: {m:?}"
    );
    assert_eq!(
        &M::decode(&bytes).unwrap(),
        m,
        "we do not read our own bytes"
    );
}

#[test]
fn every_message_agrees_with_the_prost_original() {
    let mut rng = Lcg(11);
    for _ in 0..1500 {
        agrees::<McpRequest, oracle::McpRequest>(&McpRequest {
            service_names: (0..rng.below(3)).map(|_| text(&mut rng)).collect(),
            method: text(&mut rng),
            metadata_context: metadata(&mut rng),
            mcp_request: rng.chance().then(|| blob(&mut rng)),
            headers: headers(&mut rng),
        });
        agrees::<McpResponse, oracle::McpResponse>(&McpResponse {
            service_names: (0..rng.below(3)).map(|_| text(&mut rng)).collect(),
            method: text(&mut rng),
            metadata_context: metadata(&mut rng),
            mcp_response: blob(&mut rng),
        });
        agrees::<McpRequestResult, oracle::McpRequestResult>(&McpRequestResult {
            result: outcome(&mut rng),
            header_mutation: rng.chance().then(|| HeaderMutation {
                set: headers(&mut rng),
                remove: (0..rng.below(3)).map(|_| text(&mut rng)).collect(),
            }),
            metadata: metadata(&mut rng),
        });
        agrees::<McpResponseResult, oracle::McpResponseResult>(&McpResponseResult {
            result: outcome(&mut rng),
        });
    }
}

// The gateway's own tests, ported.

#[test]
fn field_numbers_match_the_schema() {
    let encoded = McpRequest {
        service_names: vec!["alpha".into()],
        method: "tools/call".into(),
        ..Default::default()
    }
    .encode_to_vec()
    .unwrap();
    assert_eq!(encoded[0], 0x0a, "service_names is field 1");
    assert_eq!(&encoded[2..7], b"alpha");
    assert_eq!(encoded[7], 0x12, "method is field 2");
    assert_eq!(&encoded[9..19], b"tools/call");

    // and they are the bytes prost writes
    let theirs = oracle::McpRequest {
        service_names: vec!["alpha".into()],
        method: "tools/call".into(),
        ..Default::default()
    };
    assert_eq!(encoded, theirs.encode_to_vec());
}

#[test]
fn an_absent_body_is_distinguishable_from_an_empty_one() {
    let absent = McpRequest::decode(&McpRequest::default().encode_to_vec().unwrap()).unwrap();
    assert_eq!(absent.mcp_request, None);
    let empty = McpRequest {
        mcp_request: Some(Vec::new()),
        ..Default::default()
    };
    let bytes = empty.encode_to_vec().unwrap();
    assert_eq!(bytes, [0x22, 0x00], "field 4, length 0");
    assert_eq!(
        McpRequest::decode(&bytes).unwrap().mcp_request,
        Some(Vec::new())
    );
}

#[test]
fn each_result_variant_round_trips_and_pass_is_written_though_empty() {
    for result in [
        Outcome::Pass(Pass),
        Outcome::Mutated(b"{}".to_vec()),
        Outcome::Error(AuthorizationError {
            code: 1,
            reason: "nope".into(),
            mcp_error: None,
        }),
    ] {
        let m = McpRequestResult {
            result: Some(result.clone()),
            ..Default::default()
        };
        assert_eq!(
            McpRequestResult::decode(&m.encode_to_vec().unwrap())
                .unwrap()
                .result,
            Some(result)
        );
    }
    let pass = McpResponseResult {
        result: Some(Outcome::Pass(Pass)),
    };
    assert_eq!(pass.encode_to_vec().unwrap(), [0x0a, 0x00]);
    assert_eq!(
        oracle::McpResponseResult {
            result: Some(oracle::response_result::Result::Pass(oracle::Pass {}))
        }
        .encode_to_vec(),
        [0x0a, 0x00]
    );
}

#[test]
fn a_oneof_keeps_its_last_member() {
    // Mutated(b"a") then Error{code: 3}: the later member replaces the earlier one.
    let bytes = [0x12, 0x01, b'a', 0x1a, 0x02, 0x08, 0x03];
    let r = McpResponseResult::decode(&bytes).unwrap();
    let Some(Outcome::Error(e)) = r.result else {
        panic!("expected the last member")
    };
    assert_eq!(e.code, 3);
    let theirs = oracle::McpResponseResult::decode(bytes.as_slice()).unwrap();
    assert!(matches!(
        theirs.result,
        Some(oracle::response_result::Result::Error(ref e)) if e.code == 3
    ));
}

// ---- oneof members that are messages (review finding on 49830e7e) ----

/// `1a0208031a03120178`: the Error member twice, `{code: 3}` then `{reason: "x"}`. Same variant,
/// so the second occurrence merges into the first.
#[test]
fn a_repeated_message_member_of_a_oneof_merges() {
    let bytes = [0x1a, 0x02, 0x08, 0x03, 0x1a, 0x03, 0x12, 0x01, b'x'];
    let Some(Outcome::Error(e)) = McpResponseResult::decode(&bytes).unwrap().result else {
        panic!("expected the Error member")
    };
    assert_eq!((e.code, e.reason.as_str()), (3, "x"));
    let theirs = oracle::McpResponseResult::decode(bytes.as_slice()).unwrap();
    let Some(oracle::response_result::Result::Error(t)) = theirs.result else {
        panic!()
    };
    assert_eq!((t.code, t.reason.as_str()), (3, "x"));
}

/// A different member in between resets it: `Error{code: 3}`, `Mutated`, `Error{reason: "x"}`.
#[test]
fn a_member_that_changed_in_between_starts_fresh() {
    let bytes = [
        0x1a, 0x02, 0x08, 0x03, 0x12, 0x01, b'm', 0x1a, 0x03, 0x12, 0x01, b'x',
    ];
    let ours = McpResponseResult::decode(&bytes).unwrap();
    let Some(Outcome::Error(e)) = &ours.result else {
        panic!("{ours:?}")
    };
    assert_eq!((e.code, e.reason.as_str()), (0, "x"));
    let theirs = oracle::McpResponseResult::decode(bytes.as_slice()).unwrap();
    assert_eq!(ours.encode_to_vec().unwrap(), theirs.encode_to_vec());
}

#[test]
fn the_request_result_merges_its_oneof_the_same_way() {
    let bytes = [
        0x1a, 0x02, 0x08, 0x03, 0x1a, 0x03, 0x12, 0x01, b'x', 0x2a, 0x00,
    ];
    let ours = McpRequestResult::decode(&bytes).unwrap();
    let theirs = oracle::McpRequestResult::decode(bytes.as_slice()).unwrap();
    assert_eq!(ours.encode_to_vec().unwrap(), theirs.encode_to_vec());
}
