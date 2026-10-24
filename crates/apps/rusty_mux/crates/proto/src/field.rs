//! Tagged fields: a varint key `id << 1 | kind`, then the value.

use rusty_uuid::Uuid;
use rusty_wire::{Reader, Writer};

use crate::{Error, varint};

const VARINT: u64 = 0;
const BYTES: u64 = 1;

/// Appends fields to a message body.
pub(crate) struct Out(pub(crate) Writer);

impl Out {
    pub(crate) fn varint(&mut self, id: u32, value: u64) -> &mut Self {
        varint::write(&mut self.0, u64::from(id) << 1 | VARINT);
        varint::write(&mut self.0, value);
        self
    }

    pub(crate) fn bytes(&mut self, id: u32, value: &[u8]) -> &mut Self {
        varint::write(&mut self.0, u64::from(id) << 1 | BYTES);
        varint::write(&mut self.0, value.len() as u64);
        self.0.write_bytes(value);
        self
    }

    pub(crate) fn uuid(&mut self, id: u32, value: Uuid) -> &mut Self {
        self.bytes(id, value.as_bytes())
    }
}

#[derive(Clone, Copy)]
enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

/// Field ids below this are kept for lookup; every message's ids must fit.
/// Higher ids are parsed and validated, then dropped, so decode state stays
/// fixed however many fields a frame repeats.
pub(crate) const SLOTS: usize = 16;

/// A message body's fields, looked up by id. The last occurrence of an id wins.
pub(crate) struct In<'a> {
    message: u8,
    slots: [Option<Value<'a>>; SLOTS],
}

impl<'a> In<'a> {
    pub(crate) fn parse(message: u8, body: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(body);
        let mut slots = [None; SLOTS];
        while !r.is_empty() {
            let key = varint::read(&mut r)?;
            let id = u32::try_from(key >> 1).map_err(|_| Error::FieldIdTooLarge(key))?;
            let value = match key & 1 {
                VARINT => Value::Varint(varint::read(&mut r)?),
                _ => {
                    let len =
                        usize::try_from(varint::read(&mut r)?).map_err(|_| Error::Truncated)?;
                    Value::Bytes(r.read_bytes(len)?)
                }
            };
            if let Some(slot) = usize::try_from(id).ok().and_then(|i| slots.get_mut(i)) {
                *slot = Some(value);
            }
        }
        Ok(Self { message, slots })
    }

    fn get(&self, id: u32) -> Option<Value<'a>> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.slots.get(i).copied().flatten())
    }

    fn find(&self, id: u32) -> Result<Value<'a>, Error> {
        self.get(id).ok_or(Error::MissingField {
            message: self.message,
            field: id,
        })
    }

    pub(crate) fn invalid(&self, id: u32) -> Error {
        Error::InvalidField {
            message: self.message,
            field: id,
        }
    }

    pub(crate) fn varint(&self, id: u32) -> Result<u64, Error> {
        match self.find(id)? {
            Value::Varint(v) => Ok(v),
            Value::Bytes(_) => Err(self.invalid(id)),
        }
    }

    pub(crate) fn u16(&self, id: u32) -> Result<u16, Error> {
        u16::try_from(self.varint(id)?).map_err(|_| self.invalid(id))
    }

    pub(crate) fn bytes(&self, id: u32) -> Result<&'a [u8], Error> {
        match self.find(id)? {
            Value::Bytes(b) => Ok(b),
            Value::Varint(_) => Err(self.invalid(id)),
        }
    }

    pub(crate) fn string(&self, id: u32) -> Result<String, Error> {
        let raw = self.bytes(id)?;
        String::from_utf8(raw.to_vec()).map_err(|_| self.invalid(id))
    }

    pub(crate) fn uuid(&self, id: u32) -> Result<Uuid, Error> {
        let raw: [u8; 16] = self.bytes(id)?.try_into().map_err(|_| self.invalid(id))?;
        Ok(Uuid::from_bytes(raw))
    }

    /// An optional field: absent is `None`, present but malformed is an error.
    pub(crate) fn opt<T>(
        &self,
        id: u32,
        get: impl FnOnce(&Self, u32) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        if self.get(id).is_none() {
            return Ok(None);
        }
        get(self, id).map(Some)
    }
}
