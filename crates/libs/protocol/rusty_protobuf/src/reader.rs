use crate::varint::{self, unzigzag32, unzigzag64};
use crate::{Error, WireType, DEFAULT_DEPTH_LIMIT, MAX_FIELD_NUMBER};

type Result<T> = std::result::Result<T, Error>;

/// The first `N` bytes of `buf` and the rest.
fn take<const N: usize>(buf: &[u8]) -> Option<([u8; N], &[u8])> {
    let head = buf.get(..N)?.try_into().ok()?;
    Some((head, buf.get(N..)?))
}

/// The value of one field, as the wire holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value<'a> {
    /// A varint, undecoded: see [`Field::int32`], [`Field::sint64`], [`Field::bool`] ...
    Varint(u64),
    /// Eight raw bytes read as little-endian.
    Fixed64(u64),
    /// A length-delimited run: string, bytes, sub-message or packed repeated field.
    Len(&'a [u8]),
    /// Four raw bytes read as little-endian.
    Fixed32(u32),
}

/// One `(number, value)` pair from a message.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    /// The field number from the tag.
    pub number: u32,
    /// The value, by wire type.
    pub value: Value<'a>,
    depth_left: usize,
}

/// Reads the fields of one message, in wire order, without copying. After an error it yields
/// nothing more.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    depth_left: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `buf` with [`DEFAULT_DEPTH_LIMIT`].
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader::with_depth_limit(buf, DEFAULT_DEPTH_LIMIT)
    }

    /// A reader that allows `limit` levels of nested messages below this one.
    pub fn with_depth_limit(buf: &'a [u8], limit: usize) -> Reader<'a> {
        Reader {
            buf,
            depth_left: limit,
        }
    }

    /// Reads one field off the front, leaving `self.buf` just past it.
    fn read_field(&mut self) -> Result<Field<'a>> {
        let (tag, n) = varint::decode(self.buf)?;
        let rest = self.buf.get(n..).ok_or(Error::Truncated)?;
        let number = tag >> 3;
        if number == 0 || number > u64::from(MAX_FIELD_NUMBER) {
            return Err(Error::BadTag);
        }
        let (value, rest) = match (tag & 7) as u8 {
            0 => {
                let (v, n) = varint::decode(rest)?;
                (Value::Varint(v), rest.get(n..).ok_or(Error::Truncated)?)
            }
            1 => {
                let (b, rest) = take::<8>(rest).ok_or(Error::Truncated)?;
                (Value::Fixed64(u64::from_le_bytes(b)), rest)
            }
            2 => {
                let (len, n) = varint::decode(rest)?;
                let rest = rest.get(n..).ok_or(Error::Truncated)?;
                let len = usize::try_from(len).map_err(|_| Error::Truncated)?;
                let body = rest.get(..len).ok_or(Error::Truncated)?;
                (Value::Len(body), rest.get(len..).ok_or(Error::Truncated)?)
            }
            5 => {
                let (b, rest) = take::<4>(rest).ok_or(Error::Truncated)?;
                (Value::Fixed32(u32::from_le_bytes(b)), rest)
            }
            3 | 4 => return Err(Error::Group),
            w => return Err(Error::BadWireType(w)),
        };
        self.buf = rest;
        Ok(Field {
            number: number as u32,
            value,
            depth_left: self.depth_left,
        })
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Field<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let field = self.read_field();
        if field.is_err() {
            self.buf = &[];
        }
        Some(field)
    }
}

impl<'a> Field<'a> {
    fn wrong(&self, expected: WireType) -> Error {
        Error::WrongWireType {
            expected,
            found: self.wire_type(),
        }
    }

    fn varint(&self) -> Result<u64> {
        match self.value {
            Value::Varint(v) => Ok(v),
            _ => Err(self.wrong(WireType::Varint)),
        }
    }

    fn fixed32_bits(&self) -> Result<u32> {
        match self.value {
            Value::Fixed32(v) => Ok(v),
            _ => Err(self.wrong(WireType::Fixed32)),
        }
    }

    fn fixed64_bits(&self) -> Result<u64> {
        match self.value {
            Value::Fixed64(v) => Ok(v),
            _ => Err(self.wrong(WireType::Fixed64)),
        }
    }

    /// The wire type of the value.
    pub fn wire_type(&self) -> WireType {
        match self.value {
            Value::Varint(_) => WireType::Varint,
            Value::Fixed64(_) => WireType::Fixed64,
            Value::Len(_) => WireType::Len,
            Value::Fixed32(_) => WireType::Fixed32,
        }
    }

    /// A `int32`; wider values are truncated, as the reference does.
    pub fn int32(&self) -> Result<i32> {
        self.varint().map(|v| v as i32)
    }

    /// An `int64`.
    pub fn int64(&self) -> Result<i64> {
        self.varint().map(|v| v as i64)
    }

    /// A `uint32`; wider values are truncated.
    pub fn uint32(&self) -> Result<u32> {
        self.varint().map(|v| v as u32)
    }

    /// A `uint64`.
    pub fn uint64(&self) -> Result<u64> {
        self.varint()
    }

    /// A zigzag `sint32`.
    pub fn sint32(&self) -> Result<i32> {
        self.varint().map(|v| unzigzag32(v as u32))
    }

    /// A zigzag `sint64`.
    pub fn sint64(&self) -> Result<i64> {
        self.varint().map(unzigzag64)
    }

    /// A `bool`: any non-zero varint is true.
    pub fn bool(&self) -> Result<bool> {
        self.varint().map(|v| v != 0)
    }

    /// A `fixed32`.
    pub fn fixed32(&self) -> Result<u32> {
        self.fixed32_bits()
    }

    /// An `sfixed32`.
    pub fn sfixed32(&self) -> Result<i32> {
        self.fixed32_bits().map(|v| v as i32)
    }

    /// A `float`.
    pub fn float(&self) -> Result<f32> {
        self.fixed32_bits().map(f32::from_bits)
    }

    /// A `fixed64`.
    pub fn fixed64(&self) -> Result<u64> {
        self.fixed64_bits()
    }

    /// An `sfixed64`.
    pub fn sfixed64(&self) -> Result<i64> {
        self.fixed64_bits().map(|v| v as i64)
    }

    /// A `double`.
    pub fn double(&self) -> Result<f64> {
        self.fixed64_bits().map(f64::from_bits)
    }

    /// A `bytes` field.
    pub fn bytes(&self) -> Result<&'a [u8]> {
        match self.value {
            Value::Len(b) => Ok(b),
            _ => Err(self.wrong(WireType::Len)),
        }
    }

    /// A `string` field, checked for UTF-8.
    pub fn str(&self) -> Result<&'a str> {
        std::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidUtf8)
    }

    /// A sub-message: a reader one level deeper, or [`Error::DepthExceeded`].
    pub fn message(&self) -> Result<Reader<'a>> {
        let buf = self.bytes()?;
        let depth_left = self.depth_left.checked_sub(1).ok_or(Error::DepthExceeded)?;
        Ok(Reader { buf, depth_left })
    }

    /// The elements of a repeated varint field: a packed run, or a single unpacked element
    /// (call this for every occurrence of the field and chain the results).
    pub fn packed_varints(&self) -> Result<Varints<'a>> {
        match self.value {
            Value::Varint(v) => Ok(Varints {
                single: Some(v),
                rest: &[],
            }),
            Value::Len(rest) => Ok(Varints { single: None, rest }),
            _ => Err(self.wrong(WireType::Len)),
        }
    }

    /// The elements of a repeated `fixed32`/`sfixed32`/`float` field, packed or single.
    pub fn packed_fixed32(&self) -> Result<Fixed32s<'a>> {
        match self.value {
            Value::Fixed32(v) => Ok(Fixed32s {
                single: Some(v),
                rest: &[],
            }),
            Value::Len(rest) if rest.len() % 4 == 0 => Ok(Fixed32s { single: None, rest }),
            Value::Len(_) => Err(Error::BadPackedLength),
            _ => Err(self.wrong(WireType::Len)),
        }
    }

    /// The elements of a repeated `fixed64`/`sfixed64`/`double` field, packed or single.
    pub fn packed_fixed64(&self) -> Result<Fixed64s<'a>> {
        match self.value {
            Value::Fixed64(v) => Ok(Fixed64s {
                single: Some(v),
                rest: &[],
            }),
            Value::Len(rest) if rest.len() % 8 == 0 => Ok(Fixed64s { single: None, rest }),
            Value::Len(_) => Err(Error::BadPackedLength),
            _ => Err(self.wrong(WireType::Len)),
        }
    }
}

/// The varints of one occurrence of a repeated field, undecoded (see [`Field::packed_varints`]).
#[derive(Debug, Clone)]
pub struct Varints<'a> {
    single: Option<u64>,
    rest: &'a [u8],
}

impl Iterator for Varints<'_> {
    type Item = Result<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(v) = self.single.take() {
            return Some(Ok(v));
        }
        if self.rest.is_empty() {
            return None;
        }
        match varint::decode(self.rest) {
            Ok((v, n)) => {
                self.rest = self.rest.get(n..).unwrap_or_default();
                Some(Ok(v))
            }
            Err(e) => {
                self.rest = &[];
                Some(Err(e))
            }
        }
    }
}

/// The 4-byte values of one occurrence of a repeated field (see [`Field::packed_fixed32`]).
#[derive(Debug, Clone)]
pub struct Fixed32s<'a> {
    single: Option<u32>,
    rest: &'a [u8],
}

impl Iterator for Fixed32s<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if let Some(v) = self.single.take() {
            return Some(v);
        }
        let (b, rest) = take::<4>(self.rest)?;
        self.rest = rest;
        Some(u32::from_le_bytes(b))
    }
}

/// The 8-byte values of one occurrence of a repeated field (see [`Field::packed_fixed64`]).
#[derive(Debug, Clone)]
pub struct Fixed64s<'a> {
    single: Option<u64>,
    rest: &'a [u8],
}

impl Iterator for Fixed64s<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        if let Some(v) = self.single.take() {
            return Some(v);
        }
        let (b, rest) = take::<8>(self.rest)?;
        self.rest = rest;
        Some(u64::from_le_bytes(b))
    }
}
