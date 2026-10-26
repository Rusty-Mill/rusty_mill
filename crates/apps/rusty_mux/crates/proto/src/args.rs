//! The named, typed arguments of a [`Message::BlockCall`](crate::Message),
//! packed into one field: per argument, a length-prefixed UTF-8 name, a
//! one-byte type tag, then the value.

use rusty_uuid::Uuid;
use rusty_wire::{Reader, Writer};

use crate::schema::Value;
use crate::varint;

mod tag {
    pub const ID: u8 = 1;
    pub const TEXT: u8 = 2;
    pub const BYTES: u8 = 3;
    pub const CELLS: u8 = 4;
    pub const LINES: u8 = 5;
}

pub(crate) fn encode(args: &[(String, Value)]) -> Vec<u8> {
    let mut w = Writer::new();
    for (name, value) in args {
        put_bytes(&mut w, name.as_bytes());
        match value {
            Value::Id(id) => {
                w.write_u8(tag::ID);
                w.write_bytes(id.as_bytes());
            }
            Value::Text(text) => {
                w.write_u8(tag::TEXT);
                put_bytes(&mut w, text.as_bytes());
            }
            Value::Bytes(bytes) => {
                w.write_u8(tag::BYTES);
                put_bytes(&mut w, bytes);
            }
            Value::Cells(n) => {
                w.write_u8(tag::CELLS);
                varint::write(&mut w, (*n).into());
            }
            Value::Lines(n) => {
                w.write_u8(tag::LINES);
                varint::write(&mut w, (*n).into());
            }
        }
    }
    w.into_vec()
}

/// `None` if the bytes are not a well-formed argument list.
pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<(String, Value)>> {
    let mut r = Reader::new(bytes);
    let mut args = Vec::new();
    while !r.is_empty() {
        let name = String::from_utf8(get_bytes(&mut r)?.to_vec()).ok()?;
        let value = match r.read_u8().ok()? {
            tag::ID => Value::Id(Uuid::from_bytes(r.read_array().ok()?)),
            tag::TEXT => Value::Text(String::from_utf8(get_bytes(&mut r)?.to_vec()).ok()?),
            tag::BYTES => Value::Bytes(get_bytes(&mut r)?.to_vec()),
            tag::CELLS => Value::Cells(u16::try_from(varint::read(&mut r).ok()?).ok()?),
            tag::LINES => Value::Lines(u32::try_from(varint::read(&mut r).ok()?).ok()?),
            _ => return None,
        };
        args.push((name, value));
    }
    Some(args)
}

fn put_bytes(w: &mut Writer, bytes: &[u8]) {
    varint::write(w, bytes.len() as u64);
    w.write_bytes(bytes);
}

fn get_bytes<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let len = usize::try_from(varint::read(r).ok()?).ok()?;
    r.read_bytes(len).ok()
}
