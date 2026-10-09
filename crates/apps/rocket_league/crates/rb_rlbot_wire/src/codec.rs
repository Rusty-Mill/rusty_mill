//! Shared plumbing between the message types: inline structs, enum macro, and the helpers
//! that read and write strings, structs and vectors of tables.

use rusty_flatbuffers::{Builder, Offset, Table, TableBuilder};

use crate::Error;

pub(crate) type R<T> = Result<T, Error>;

/// A FlatBuffers struct: fixed size, stored inline, no vtable.
pub trait Struct: Sized {
    const SIZE: usize;
    const ALIGN: usize;
    /// Appends exactly `SIZE` little-endian bytes.
    fn write(&self, out: &mut Vec<u8>);
    /// Reads from the first `SIZE` bytes.
    fn read(bytes: &[u8]) -> Option<Self>;
}

/// A FlatBuffers table with a fixed slot layout.
pub trait TableCodec: Sized {
    /// Writes the table (children first) and returns it.
    fn write(&self, b: &mut Builder) -> R<Offset>;
    fn read(t: &Table<'_>) -> R<Self>;
}

pub(crate) fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Defines a `u8` enum with its protocol values and a checked conversion.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum $name { $($variant = $value),+ }

        impl $name {
            pub(crate) fn from_u8(value: u8) -> $crate::codec::R<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err($crate::Error::UnknownEnum { name: stringify!($name), value }),
                }
            }
        }
    };
}
pub(crate) use wire_enum;

// ---- reading ----

pub(crate) fn string(t: &Table<'_>, slot: usize) -> R<String> {
    Ok(t.string(slot)?.unwrap_or_default().to_owned())
}

pub(crate) fn opt_struct<S: Struct>(t: &Table<'_>, slot: usize) -> R<Option<S>> {
    match t.struct_bytes(slot, S::SIZE)? {
        Some(bytes) => Ok(Some(S::read(bytes).ok_or(Error::Missing("struct bytes"))?)),
        None => Ok(None),
    }
}

/// A struct field the protocol always sends; absent reads as `S::default()`.
pub(crate) fn struct_or_default<S: Struct + Default>(t: &Table<'_>, slot: usize) -> R<S> {
    Ok(opt_struct(t, slot)?.unwrap_or_default())
}

pub(crate) fn struct_vec<S: Struct>(t: &Table<'_>, slot: usize) -> R<Vec<S>> {
    let Some(v) = t.vector(slot)? else {
        return Ok(Vec::new());
    };
    (0..v.len())
        .map(|i| S::read(v.struct_bytes(i, S::SIZE)?).ok_or(Error::Missing("struct bytes")))
        .collect()
}

pub(crate) fn opt_table_vec<T: TableCodec>(t: &Table<'_>, slot: usize) -> R<Option<Vec<T>>> {
    let Some(v) = t.vector(slot)? else {
        return Ok(None);
    };
    (0..v.len())
        .map(|i| T::read(&v.table(i)?))
        .collect::<R<Vec<_>>>()
        .map(Some)
}

pub(crate) fn table_vec<T: TableCodec>(t: &Table<'_>, slot: usize) -> R<Vec<T>> {
    Ok(opt_table_vec(t, slot)?.unwrap_or_default())
}

pub(crate) fn opt_table<T: TableCodec>(t: &Table<'_>, slot: usize) -> R<Option<T>> {
    t.table(slot)?.map(|sub| T::read(&sub)).transpose()
}

pub(crate) fn req_table<T: TableCodec>(t: &Table<'_>, slot: usize, name: &'static str) -> R<T> {
    opt_table(t, slot)?.ok_or(Error::Missing(name))
}

pub(crate) fn enum_field<E>(t: &Table<'_>, slot: usize, from: fn(u8) -> R<E>) -> R<E> {
    from(t.scalar(slot, 0u8)?)
}

// ---- writing ----

pub(crate) fn struct_bytes<S: Struct>(s: &S) -> Vec<u8> {
    let mut out = Vec::with_capacity(S::SIZE);
    s.write(&mut out);
    out
}

pub(crate) fn add_struct<S: Struct>(t: &mut TableBuilder<'_>, slot: usize, s: &S) -> R<()> {
    Ok(t.add_struct(slot, &struct_bytes(s), S::ALIGN)?)
}

pub(crate) fn add_opt_struct<S: Struct>(
    t: &mut TableBuilder<'_>,
    slot: usize,
    s: &Option<S>,
) -> R<()> {
    s.as_ref().map_or(Ok(()), |s| add_struct(t, slot, s))
}

/// Writes a string, empty or not: the reference reader treats string fields as required.
pub(crate) fn string_offset(b: &mut Builder, s: &str) -> R<Option<Offset>> {
    Ok(Some(b.create_string(s)?))
}

pub(crate) fn add_offset(t: &mut TableBuilder<'_>, slot: usize, o: Option<Offset>) -> R<()> {
    o.map_or(Ok(()), |o| Ok(t.add_offset(slot, o)?))
}

pub(crate) fn table_offset<T: TableCodec>(b: &mut Builder, v: &Option<T>) -> R<Option<Offset>> {
    v.as_ref().map(|v| v.write(b)).transpose()
}

/// A vector of structs, always written (the reference reader treats vector fields as required).
pub(crate) fn struct_vec_offset<S: Struct>(b: &mut Builder, items: &[S]) -> R<Option<Offset>> {
    let bytes: Vec<Vec<u8>> = items.iter().map(struct_bytes).collect();
    let views: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    Ok(Some(b.create_struct_vector(&views, S::SIZE, S::ALIGN)?))
}

/// A vector of tables; `None` stays absent, `Some(empty)` is written as an empty vector.
pub(crate) fn table_vec_offset<T: TableCodec>(
    b: &mut Builder,
    items: Option<&[T]>,
) -> R<Option<Offset>> {
    let Some(items) = items else { return Ok(None) };
    let offsets = items.iter().map(|t| t.write(b)).collect::<R<Vec<_>>>()?;
    Ok(Some(b.create_offset_vector(&offsets)?))
}

/// A required vector of tables: always written, empty or not.
pub(crate) fn required_table_vec_offset<T: TableCodec>(
    b: &mut Builder,
    items: &[T],
) -> R<Option<Offset>> {
    table_vec_offset(b, Some(items))
}
