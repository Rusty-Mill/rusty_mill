//! A slice of the A2A schema (`rusty_a2a/spec/a2a.proto`) on `rusty_prost`, checked against
//! `prost`-derived originals: a oneof holding a `google.protobuf.Value`, `Struct` metadata,
//! a map of messages, proto3 `optional`, an enum, and a `Timestamp` in a nested message.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::Lcg;
use rusty_prost::wkt::{Struct, Timestamp, Value};
use rusty_prost::{field, Field, Message, Result, Writer};

mod oracle {
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    pub struct Part {
        #[prost(oneof = "part::Content", tags = "1, 2, 3, 4")]
        pub content: Option<part::Content>,
        #[prost(message, optional, tag = "5")]
        pub metadata: Option<prost_types::Struct>,
        #[prost(string, tag = "6")]
        pub filename: String,
        #[prost(string, tag = "7")]
        pub media_type: String,
    }

    pub mod part {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Content {
            #[prost(string, tag = "1")]
            Text(String),
            #[prost(bytes = "vec", tag = "2")]
            Raw(Vec<u8>),
            #[prost(string, tag = "3")]
            Url(String),
            #[prost(message, tag = "4")]
            Data(prost_types::Value),
        }
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Message_ {
        #[prost(string, tag = "1")]
        pub message_id: String,
        #[prost(string, tag = "2")]
        pub context_id: String,
        #[prost(string, tag = "3")]
        pub task_id: String,
        #[prost(int32, tag = "4")]
        pub role: i32,
        #[prost(message, repeated, tag = "5")]
        pub parts: Vec<Part>,
        #[prost(message, optional, tag = "6")]
        pub metadata: Option<prost_types::Struct>,
        #[prost(string, repeated, tag = "7")]
        pub extensions: Vec<String>,
        #[prost(string, repeated, tag = "8")]
        pub reference_task_ids: Vec<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct TaskStatus {
        #[prost(int32, tag = "1")]
        pub state: i32,
        #[prost(message, optional, tag = "2")]
        pub message: Option<Message_>,
        #[prost(message, optional, tag = "3")]
        pub timestamp: Option<prost_types::Timestamp>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SendMessageConfiguration {
        #[prost(string, repeated, tag = "1")]
        pub accepted_output_modes: Vec<String>,
        #[prost(int32, optional, tag = "3")]
        pub history_length: Option<i32>,
        #[prost(bool, tag = "4")]
        pub return_immediately: bool,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct StringList {
        #[prost(string, repeated, tag = "1")]
        pub list: Vec<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SecurityRequirement {
        #[prost(btree_map = "string, message", tag = "1")]
        pub schemes: std::collections::BTreeMap<String, StringList>,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Content {
    Text(String),
    Raw(Vec<u8>),
    Url(String),
    Data(Value),
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Part {
    content: Option<Content>,
    metadata: Option<Struct>,
    filename: String,
    media_type: String,
}

impl Message for Part {
    fn encode(&self, w: &mut Writer) {
        match &self.content {
            Some(Content::Text(t)) => w.string(1, t),
            Some(Content::Raw(b)) => w.bytes(2, b),
            Some(Content::Url(u)) => w.string(3, u),
            Some(Content::Data(v)) => w.message(4, |w| v.encode(w)),
            None => {}
        }
        if let Some(m) = &self.metadata {
            w.message(5, |w| m.encode(w));
        }
        if !self.filename.is_empty() {
            w.string(6, &self.filename);
        }
        if !self.media_type.is_empty() {
            w.string(7, &self.media_type);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.content = Some(Content::Text(f.str()?.to_owned())),
            2 => self.content = Some(Content::Raw(f.bytes()?.to_vec())),
            3 => self.content = Some(Content::Url(f.str()?.to_owned())),
            4 => {
                // A repeated `data` merges into the Value already there only if it is one.
                let mut v = match self.content.take() {
                    Some(Content::Data(v)) => Some(v),
                    _ => None,
                };
                field::message(&mut v, &f)?;
                self.content = v.map(Content::Data);
            }
            5 => field::message(&mut self.metadata, &f)?,
            6 => self.filename = f.str()?.to_owned(),
            7 => self.media_type = f.str()?.to_owned(),
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct A2aMessage {
    message_id: String,
    context_id: String,
    task_id: String,
    role: i32,
    parts: Vec<Part>,
    metadata: Option<Struct>,
    extensions: Vec<String>,
    reference_task_ids: Vec<String>,
}

impl Message for A2aMessage {
    fn encode(&self, w: &mut Writer) {
        for (n, s) in [
            (1, &self.message_id),
            (2, &self.context_id),
            (3, &self.task_id),
        ] {
            if !s.is_empty() {
                w.string(n, s);
            }
        }
        if self.role != 0 {
            w.int32(4, self.role);
        }
        for p in &self.parts {
            w.message(5, |w| p.encode(w));
        }
        if let Some(m) = &self.metadata {
            w.message(6, |w| m.encode(w));
        }
        for e in &self.extensions {
            w.string(7, e);
        }
        for r in &self.reference_task_ids {
            w.string(8, r);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.message_id = f.str()?.to_owned(),
            2 => self.context_id = f.str()?.to_owned(),
            3 => self.task_id = f.str()?.to_owned(),
            4 => self.role = f.int32()?,
            5 => field::push_message(&mut self.parts, &f)?,
            6 => field::message(&mut self.metadata, &f)?,
            7 => self.extensions.push(f.str()?.to_owned()),
            8 => self.reference_task_ids.push(f.str()?.to_owned()),
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct TaskStatus {
    state: i32,
    message: Option<A2aMessage>,
    timestamp: Option<Timestamp>,
}

impl Message for TaskStatus {
    fn encode(&self, w: &mut Writer) {
        if self.state != 0 {
            w.int32(1, self.state);
        }
        if let Some(m) = &self.message {
            w.message(2, |w| m.encode(w));
        }
        if let Some(t) = &self.timestamp {
            w.message(3, |w| t.encode(w));
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.state = f.int32()?,
            2 => field::message(&mut self.message, &f)?,
            3 => field::message(&mut self.timestamp, &f)?,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct SendMessageConfiguration {
    accepted_output_modes: Vec<String>,
    history_length: Option<i32>,
    return_immediately: bool,
}

impl Message for SendMessageConfiguration {
    fn encode(&self, w: &mut Writer) {
        for m in &self.accepted_output_modes {
            w.string(1, m);
        }
        if let Some(n) = self.history_length {
            w.int32(3, n);
        }
        if self.return_immediately {
            w.bool(4, true);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.accepted_output_modes.push(f.str()?.to_owned()),
            3 => self.history_length = Some(f.int32()?),
            4 => self.return_immediately = f.bool()?,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct StringList {
    list: Vec<String>,
}

impl Message for StringList {
    fn encode(&self, w: &mut Writer) {
        for s in &self.list {
            w.string(1, s);
        }
    }
    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        if f.number == 1 {
            self.list.push(f.str()?.to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct SecurityRequirement {
    schemes: BTreeMap<String, StringList>,
}

impl Message for SecurityRequirement {
    fn encode(&self, w: &mut Writer) {
        for (k, v) in &self.schemes {
            w.message(1, |w| {
                w.string(1, k);
                w.message(2, |w| v.encode(w));
            });
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        if f.number == 1 {
            field::map_entry(
                &mut self.schemes,
                &f,
                |k| Ok(k.str()?.to_owned()),
                |v| {
                    let mut l = StringList::default();
                    l.merge(v.message()?)?;
                    Ok(l)
                },
            )?;
        }
        Ok(())
    }
}

// ---- generator ----

fn text(rng: &mut Lcg) -> String {
    ["", "a", "h\u{e9}llo", "m-1", &"q".repeat(131)][rng.below(5)].into()
}

fn metadata(rng: &mut Lcg) -> Option<Struct> {
    rng.chance().then(|| Struct {
        fields: (0..rng.below(3))
            .map(|i| (format!("k{i}"), Value::Number((rng.next() as i32) as f64)))
            .collect(),
    })
}

fn part(rng: &mut Lcg) -> Part {
    Part {
        content: match rng.below(5) {
            0 => None,
            1 => Some(Content::Text(text(rng))),
            2 => Some(Content::Raw(
                (0..rng.below(150)).map(|_| rng.next() as u8).collect(),
            )),
            3 => Some(Content::Url(text(rng))),
            _ => Some(Content::Data(Value::List(vec![
                Value::Bool(rng.chance()),
                Value::Null,
            ]))),
        },
        metadata: metadata(rng),
        filename: text(rng),
        media_type: text(rng),
    }
}

fn message(rng: &mut Lcg) -> A2aMessage {
    A2aMessage {
        message_id: text(rng),
        context_id: text(rng),
        task_id: text(rng),
        role: rng.below(3) as i32,
        parts: (0..rng.below(4)).map(|_| part(rng)).collect(),
        metadata: metadata(rng),
        extensions: (0..rng.below(3)).map(|_| text(rng)).collect(),
        reference_task_ids: (0..rng.below(3)).map(|_| text(rng)).collect(),
    }
}

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
fn the_slice_agrees_with_prost() {
    let mut rng = Lcg(21);
    for _ in 0..1500 {
        agrees::<Part, oracle::Part>(&part(&mut rng));
        agrees::<A2aMessage, oracle::Message_>(&message(&mut rng));
        agrees::<TaskStatus, oracle::TaskStatus>(&TaskStatus {
            state: rng.below(9) as i32,
            message: rng.chance().then(|| message(&mut rng)),
            timestamp: rng.chance().then(|| Timestamp {
                seconds: 1_700_000_000 + rng.below(1_000_000) as i64,
                nanos: rng.below(1_000_000_000) as i32,
            }),
        });
        agrees::<SendMessageConfiguration, oracle::SendMessageConfiguration>(
            &SendMessageConfiguration {
                accepted_output_modes: (0..rng.below(3)).map(|_| text(&mut rng)).collect(),
                history_length: rng.chance().then(|| rng.below(3) as i32),
                return_immediately: rng.chance(),
            },
        );
        agrees::<SecurityRequirement, oracle::SecurityRequirement>(&SecurityRequirement {
            schemes: (0..rng.below(3))
                .map(|i| {
                    (
                        format!("scheme{i}"),
                        StringList {
                            list: (0..1 + rng.below(3)).map(|_| text(&mut rng)).collect(),
                        },
                    )
                })
                .collect(),
        });
    }
}

#[test]
fn proto3_optional_keeps_zero_distinct_from_absent() {
    let zero = SendMessageConfiguration {
        history_length: Some(0),
        ..Default::default()
    };
    assert_eq!(
        zero.encode_to_vec().unwrap(),
        [0x18, 0x00],
        "field 3, explicit zero"
    );
    assert_eq!(
        SendMessageConfiguration::decode(&[0x18, 0x00])
            .unwrap()
            .history_length,
        Some(0)
    );
    let absent = SendMessageConfiguration::default();
    assert_eq!(absent.encode_to_vec().unwrap(), b"");
    assert_eq!(
        SendMessageConfiguration::decode(b"")
            .unwrap()
            .history_length,
        None
    );
}

#[test]
fn a_data_part_holds_any_json_value() {
    let p = Part {
        content: Some(Content::Data(Value::Number(0.0))),
        ..Default::default()
    };
    let bytes = p.encode_to_vec().unwrap();
    // field 4, a Value holding number_value 0 (written, not omitted)
    assert_eq!(bytes, [0x22, 0x09, 0x11, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Part::decode(&bytes).unwrap(), p);
}
