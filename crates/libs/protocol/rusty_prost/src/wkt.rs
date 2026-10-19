//! The well-known types the consumers use: `Empty`, `Timestamp`, `Any`, `Struct` and `Value`.

use std::collections::BTreeMap;

use crate::{field, Field, Message, Name, Result, Writer};

/// `google.protobuf.Empty`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Empty;

/// `google.protobuf.Timestamp`: seconds and nanoseconds since the Unix epoch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub seconds: i64,
    /// Nanoseconds of the second, 0..=999_999_999.
    pub nanos: i32,
}

/// `google.protobuf.Any`: a message of another type, with the URL that names it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Any {
    /// `type.googleapis.com/<full name>`.
    pub type_url: String,
    /// The encoded message.
    pub value: Vec<u8>,
}

/// `google.protobuf.Value`: a JSON-shaped value. A buffer that sets no kind decodes to `Null`.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum Value {
    /// `null_value`.
    #[default]
    Null,
    /// `number_value`.
    Number(f64),
    /// `string_value`.
    String(String),
    /// `bool_value`.
    Bool(bool),
    /// `struct_value`.
    Struct(Struct),
    /// `list_value`.
    List(Vec<Value>),
}

/// `google.protobuf.Struct`: a JSON object.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Struct {
    /// The members, in key order.
    pub fields: BTreeMap<String, Value>,
}

/// The message name in a type URL: what follows the last `/`.
fn type_name(url: &str) -> Option<&str> {
    url.rsplit_once('/').map(|(_, name)| name)
}

impl Any {
    /// Packs `m` under its [`Name::TYPE_URL`].
    pub fn pack<M: Message + Name>(m: &M) -> Result<Any> {
        Ok(Any {
            type_url: M::TYPE_URL.to_owned(),
            value: m.encode_to_vec()?,
        })
    }

    /// Whether the URL names `M`. Only the last path segment counts (the part after the final
    /// `/` is the message's full name), so a different prefix still matches. A URL with no `/`
    /// names no type, and neither does one whose name is empty or dotted, because no `M` has
    /// such a name.
    pub fn is<M: Name>(&self) -> bool {
        match (type_name(&self.type_url), type_name(M::TYPE_URL)) {
            (Some(actual), Some(wanted)) => actual == wanted,
            _ => false,
        }
    }

    /// Unpacks the message if the URL names `M` (see [`Any::is`]), or `Ok(None)` if it names
    /// another type or is malformed.
    pub fn unpack<M: Message + Name>(&self) -> Result<Option<M>> {
        if !self.is::<M>() {
            return Ok(None);
        }
        M::decode(&self.value).map(Some)
    }
}

impl Name for Empty {
    const TYPE_URL: &'static str = "type.googleapis.com/google.protobuf.Empty";
}

impl Name for Timestamp {
    const TYPE_URL: &'static str = "type.googleapis.com/google.protobuf.Timestamp";
}

impl Name for Any {
    const TYPE_URL: &'static str = "type.googleapis.com/google.protobuf.Any";
}

impl Name for Struct {
    const TYPE_URL: &'static str = "type.googleapis.com/google.protobuf.Struct";
}

impl Name for Value {
    const TYPE_URL: &'static str = "type.googleapis.com/google.protobuf.Value";
}

impl Message for Empty {
    fn encode(&self, _: &mut Writer) {}

    fn merge_field(&mut self, _: Field<'_>) -> Result<()> {
        Ok(())
    }
}

impl Message for Timestamp {
    fn encode(&self, w: &mut Writer) {
        if self.seconds != 0 {
            w.int64(1, self.seconds);
        }
        if self.nanos != 0 {
            w.int32(2, self.nanos);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.seconds = f.int64()?,
            2 => self.nanos = f.int32()?,
            _ => {}
        }
        Ok(())
    }
}

impl Message for Any {
    fn encode(&self, w: &mut Writer) {
        if !self.type_url.is_empty() {
            w.string(1, &self.type_url);
        }
        if !self.value.is_empty() {
            w.bytes(2, &self.value);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.type_url = f.str()?.to_owned(),
            2 => self.value = f.bytes()?.to_vec(),
            _ => {}
        }
        Ok(())
    }
}

impl Message for Value {
    fn encode(&self, w: &mut Writer) {
        // A kind that is set is written even when it is zero: it is a oneof member, not a
        // proto3 default.
        match self {
            Value::Null => w.int32(1, 0),
            Value::Number(n) => w.double(2, *n),
            Value::String(s) => w.string(3, s),
            Value::Bool(b) => w.bool(4, *b),
            Value::Struct(s) => w.message(5, |w| s.encode(w)),
            Value::List(items) => w.message(6, |w| {
                for v in items {
                    w.message(1, |w| v.encode(w));
                }
            }),
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => {
                f.int32()?;
                *self = Value::Null;
            }
            2 => *self = Value::Number(f.double()?),
            3 => *self = Value::String(f.str()?.to_owned()),
            4 => *self = Value::Bool(f.bool()?),
            5 => {
                let mut s = match std::mem::take(self) {
                    Value::Struct(s) => Some(s),
                    _ => None,
                };
                field::message(&mut s, &f)?;
                *self = Value::Struct(s.unwrap_or_default());
            }
            6 => {
                // `ListValue { repeated Value values = 1; }`: a second occurrence appends.
                let mut items = match std::mem::take(self) {
                    Value::List(items) => items,
                    _ => Vec::new(),
                };
                for e in f.message()? {
                    let e = e?;
                    if e.number == 1 {
                        field::push_message(&mut items, &e)?;
                    }
                }
                *self = Value::List(items);
            }
            _ => {}
        }
        Ok(())
    }
}

impl Message for Struct {
    fn encode(&self, w: &mut Writer) {
        for (k, v) in &self.fields {
            w.message(1, |w| {
                w.string(1, k);
                w.message(2, |w| v.encode(w));
            });
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        if f.number == 1 {
            field::map_entry(
                &mut self.fields,
                &f,
                |k, f| {
                    *k = f.str()?.to_owned();
                    Ok(())
                },
                field::merge_message,
            )?;
        }
        Ok(())
    }
}
