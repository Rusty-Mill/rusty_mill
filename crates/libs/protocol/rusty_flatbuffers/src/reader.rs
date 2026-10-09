use crate::{Error, Scalar};

type Result<T> = std::result::Result<T, Error>;

/// `len` bytes at `pos`, or [`Error::Truncated`].
fn bytes(buf: &[u8], pos: usize, len: usize) -> Result<&[u8]> {
    let truncated = Error::Truncated {
        pos,
        len,
        buf_len: buf.len(),
    };
    let end = pos.checked_add(len).ok_or(truncated)?;
    buf.get(pos..end).ok_or(truncated)
}

fn read<T: Scalar>(buf: &[u8], pos: usize) -> Result<T> {
    let raw = bytes(buf, pos, T::SIZE)?;
    T::read_le(raw).ok_or(Error::Truncated {
        pos,
        len: T::SIZE,
        buf_len: buf.len(),
    })
}

/// The absolute position a forward `u32` offset stored at `pos` refers to.
fn follow(buf: &[u8], pos: usize) -> Result<usize> {
    let rel = read::<u32>(buf, pos)?;
    pos.checked_add(usize::try_from(rel).map_err(|_| Error::BadOffset)?)
        .ok_or(Error::BadOffset)
}

/// Whether bytes 4..8 of `buf` are the file identifier `id`.
pub fn has_identifier(buf: &[u8], id: [u8; 4]) -> bool {
    buf.get(4..8) == Some(&id[..])
}

/// A table inside a buffer: fields are looked up by slot through its vtable.
#[derive(Debug, Clone, Copy)]
pub struct Table<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Table<'a> {
    /// The root table of `buf`.
    pub fn root(buf: &'a [u8]) -> Result<Table<'a>> {
        Table::at(buf, follow(buf, 0)?)
    }

    fn at(buf: &'a [u8], pos: usize) -> Result<Table<'a>> {
        bytes(buf, pos, 4)?;
        Ok(Table { buf, pos })
    }

    /// Where field `slot` lives, or `None` if the buffer omits it (the schema default).
    fn field(&self, slot: usize) -> Result<Option<usize>> {
        let soffset = i64::from(read::<i32>(self.buf, self.pos)?);
        let vtable = i64::try_from(self.pos)
            .ok()
            .and_then(|p| p.checked_sub(soffset))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or(Error::BadOffset)?;
        let vtable_len = usize::from(read::<u16>(self.buf, vtable)?);
        if vtable_len < 4 || vtable_len % 2 != 0 || bytes(self.buf, vtable, vtable_len).is_err() {
            return Err(Error::BadVtable);
        }
        let entry = 4 + 2 * slot;
        if entry + 2 > vtable_len {
            return Ok(None);
        }
        match read::<u16>(self.buf, vtable + entry)? {
            0 => Ok(None),
            off => self
                .pos
                .checked_add(usize::from(off))
                .map(Some)
                .ok_or(Error::BadOffset),
        }
    }

    /// A scalar field, or `default` if absent.
    pub fn scalar<T: Scalar>(&self, slot: usize, default: T) -> Result<T> {
        match self.field(slot)? {
            Some(pos) => read(self.buf, pos),
            None => Ok(default),
        }
    }

    /// A string field.
    pub fn string(&self, slot: usize) -> Result<Option<&'a str>> {
        let Some(pos) = self.field(slot)? else {
            return Ok(None);
        };
        string_at(self.buf, follow(self.buf, pos)?).map(Some)
    }

    /// A sub-table field (also the value slot of a union, once its type slot is known).
    pub fn table(&self, slot: usize) -> Result<Option<Table<'a>>> {
        let Some(pos) = self.field(slot)? else {
            return Ok(None);
        };
        Table::at(self.buf, follow(self.buf, pos)?).map(Some)
    }

    /// A vector field.
    pub fn vector(&self, slot: usize) -> Result<Option<Vector<'a>>> {
        let Some(pos) = self.field(slot)? else {
            return Ok(None);
        };
        let target = follow(self.buf, pos)?;
        let len = usize::try_from(read::<u32>(self.buf, target)?).map_err(|_| Error::BadOffset)?;
        Ok(Some(Vector {
            buf: self.buf,
            start: target + 4,
            len,
        }))
    }

    /// The `size` inline bytes of a struct field.
    pub fn struct_bytes(&self, slot: usize, size: usize) -> Result<Option<&'a [u8]>> {
        match self.field(slot)? {
            Some(pos) => bytes(self.buf, pos, size).map(Some),
            None => Ok(None),
        }
    }
}

fn string_at(buf: &[u8], pos: usize) -> Result<&str> {
    let len = usize::try_from(read::<u32>(buf, pos)?).map_err(|_| Error::BadOffset)?;
    let raw = bytes(buf, pos + 4, len)?;
    std::str::from_utf8(raw).map_err(|_| Error::InvalidUtf8)
}

/// A vector inside a buffer. Element bounds are checked when an element is read.
#[derive(Debug, Clone, Copy)]
pub struct Vector<'a> {
    buf: &'a [u8],
    start: usize,
    len: usize,
}

impl<'a> Vector<'a> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn check(&self, index: usize) -> Result<()> {
        if index < self.len {
            Ok(())
        } else {
            Err(Error::IndexOutOfRange {
                index,
                len: self.len,
            })
        }
    }

    /// Where element `index` of `size` bytes starts.
    fn element(&self, index: usize, size: usize) -> Result<usize> {
        self.check(index)?;
        index
            .checked_mul(size)
            .and_then(|o| self.start.checked_add(o))
            .ok_or(Error::BadOffset)
    }

    pub fn scalar<T: Scalar>(&self, index: usize) -> Result<T> {
        read(self.buf, self.element(index, T::SIZE)?)
    }

    pub fn string(&self, index: usize) -> Result<&'a str> {
        string_at(self.buf, follow(self.buf, self.element(index, 4)?)?)
    }

    pub fn table(&self, index: usize) -> Result<Table<'a>> {
        Table::at(self.buf, follow(self.buf, self.element(index, 4)?)?)
    }

    /// The `size` inline bytes of struct element `index`.
    pub fn struct_bytes(&self, index: usize, size: usize) -> Result<&'a [u8]> {
        bytes(self.buf, self.element(index, size)?, size)
    }
}
