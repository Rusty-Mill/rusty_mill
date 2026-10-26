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

/// A message body's recognized fields, looked up by id. The last occurrence wins.
/// Unknown fields are validated but not retained; repeated fields replace a slot.
pub(crate) struct In<'a> {
    message: u8,
    fields: [Option<Value<'a>>; crate::message::FIELD_COUNT],
}

impl<'a> In<'a> {
    pub(crate) fn parse(message: u8, body: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(body);
        let mut fields = [None; crate::message::FIELD_COUNT];
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
            if let Some(slot) = id.checked_sub(1).and_then(|i| fields.get_mut(i as usize)) {
                *slot = Some(value);
            }
        }
        Ok(Self { message, fields })
    }

    fn find(&self, id: u32) -> Result<Value<'a>, Error> {
        self.slot(id).ok_or(Error::MissingField {
            message: self.message,
            field: id,
        })
    }

    fn slot(&self, id: u32) -> Option<Value<'a>> {
        id.checked_sub(1)
            .and_then(|i| self.fields.get(i as usize))
            .copied()
            .flatten()
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

    pub(crate) fn u32(&self, id: u32) -> Result<u32, Error> {
        u32::try_from(self.varint(id)?).map_err(|_| self.invalid(id))
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
        if self.slot(id).is_none() {
            return Ok(None);
        }
        get(self, id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_count_unknown_fields_retain_no_state() {
        let mut out = Out(Writer::new());
        let first = crate::message::FIELD_COUNT as u32 + 1;
        for id in first..first + 100_000 {
            out.varint(id, 0).bytes(id, b"unknown");
        }
        out.varint(0, 0).bytes(u32::MAX, b"also unknown");
        let body = out.0.into_vec();
        let fields = In::parse(1, &body).unwrap();
        assert_eq!(fields.fields.len(), crate::message::FIELD_COUNT);
        assert!(fields.fields.iter().all(Option::is_none));
        assert_eq!(fields.opt(10, In::varint), Ok(None));
    }

    #[test]
    fn high_count_duplicates_replace_slots_including_invalid_final_values() {
        let mut out = Out(Writer::new());
        for _ in 0..100_000 {
            out.varint(10, 1).bytes(14, &[0; 16]);
        }
        out.varint(10, 42).bytes(14, b"short uuid");
        let mut body = out.0.into_vec();
        let fields = In::parse(1, &body).unwrap();
        assert_eq!(fields.fields.len(), crate::message::FIELD_COUNT);
        assert_eq!(fields.fields.iter().flatten().count(), 2);
        assert_eq!(fields.varint(10), Ok(42));
        assert_eq!(fields.opt(14, In::uuid), Err(fields.invalid(14)));

        // A wrong-kind final value must replace a valid one, not be ignored.
        body.extend_from_slice(&[21, 0]);
        let fields = In::parse(1, &body).unwrap();
        assert_eq!(fields.varint(10), Err(fields.invalid(10)));
        // Conversely, a later valid value replaces an earlier invalid one.
        body.extend_from_slice(&[20, 7]);
        assert_eq!(In::parse(1, &body).unwrap().varint(10), Ok(7));
    }
}
