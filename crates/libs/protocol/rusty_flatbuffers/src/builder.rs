use crate::{Error, Scalar};

type Result<T> = std::result::Result<T, Error>;

/// Where an object written by a [`Builder`] starts, as its distance from the end of the buffer.
/// Only a builder hands these out, so a stale or invented position cannot be passed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset(usize);

/// Builds a buffer back to front. Objects a table refers to (strings, vectors, sub-tables)
/// are created first; the table is written last and the root is passed to [`Builder::finish`].
#[derive(Debug, Default)]
pub struct Builder {
    /// The buffer so far, reversed: `rev[0]` is the last byte of the finished buffer.
    rev: Vec<u8>,
    min_align: usize,
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            rev: Vec::new(),
            min_align: 1,
        }
    }

    /// Pads so that after writing `additional` more bytes the position is a multiple of `size`.
    fn prep(&mut self, size: usize, additional: usize) {
        self.min_align = self.min_align.max(size);
        let pad = self.rev.len().wrapping_add(additional).wrapping_neg() & (size - 1);
        self.rev.resize(self.rev.len() + pad, 0);
    }

    fn push_scalar<T: Scalar>(&mut self, value: T) -> usize {
        self.prep(T::SIZE, 0);
        value.push_rev(&mut self.rev);
        self.rev.len()
    }

    /// A forward `u32` offset to `target`, written at the current end.
    fn push_offset(&mut self, target: Offset) -> usize {
        self.prep(4, 0);
        let rel = self.rev.len() + 4 - target.0;
        // `target` was written earlier, so it is nearer the end: `rel` cannot underflow.
        (rel as u32).push_rev(&mut self.rev);
        self.rev.len()
    }

    /// Writes a string (length, bytes, terminating zero).
    pub fn create_string(&mut self, s: &str) -> Result<Offset> {
        let len = u32::try_from(s.len()).map_err(|_| Error::TooLarge)?;
        self.prep(4, s.len() + 1);
        self.rev.push(0);
        self.rev.extend(s.bytes().rev());
        Ok(Offset(self.push_scalar(len)))
    }

    /// Writes a vector of scalars.
    pub fn create_vector<T: Scalar>(&mut self, items: &[T]) -> Result<Offset> {
        let len = u32::try_from(items.len()).map_err(|_| Error::TooLarge)?;
        self.prep(4, items.len() * T::SIZE);
        self.prep(T::SIZE, items.len() * T::SIZE);
        for item in items.iter().rev() {
            item.push_rev(&mut self.rev);
        }
        Ok(Offset(self.push_scalar(len)))
    }

    /// Writes a vector of offsets (to strings or tables) written earlier.
    pub fn create_offset_vector(&mut self, items: &[Offset]) -> Result<Offset> {
        let len = u32::try_from(items.len()).map_err(|_| Error::TooLarge)?;
        self.prep(4, items.len() * 4);
        for item in items.iter().rev() {
            self.push_offset(*item);
        }
        Ok(Offset(self.push_scalar(len)))
    }

    /// Writes a vector of structs given as their inline little-endian bytes, `size` each and
    /// aligned to `align`.
    pub fn create_struct_vector(
        &mut self,
        items: &[&[u8]],
        size: usize,
        align: usize,
    ) -> Result<Offset> {
        let len = u32::try_from(items.len()).map_err(|_| Error::TooLarge)?;
        self.prep(4, items.len() * size);
        self.prep(align, items.len() * size);
        for item in items.iter().rev() {
            debug_assert_eq!(item.len(), size);
            self.rev.extend(item.iter().rev());
        }
        Ok(Offset(self.push_scalar(len)))
    }

    /// Starts a table. Its fields go in with the `add_*` methods and `finish` closes it.
    pub fn start_table(&mut self) -> TableBuilder<'_> {
        let start = self.rev.len();
        TableBuilder {
            builder: self,
            start,
            fields: Vec::new(),
        }
    }

    /// Completes the buffer with `root` as its root table and an optional 4-byte file
    /// identifier.
    pub fn finish(mut self, root: Offset, identifier: Option<[u8; 4]>) -> Result<Vec<u8>> {
        let id_len = if identifier.is_some() { 4 } else { 0 };
        self.prep(self.min_align.max(4), 4 + id_len);
        if let Some(id) = identifier {
            self.rev.extend(id.iter().rev());
        }
        self.push_offset(root);
        u32::try_from(self.rev.len()).map_err(|_| Error::TooLarge)?;
        self.rev.reverse();
        Ok(self.rev)
    }
}

/// A table under construction (see [`Builder::start_table`]). Holding the builder mutably
/// means nothing else can be written until the table is finished, as the format requires.
pub struct TableBuilder<'b> {
    builder: &'b mut Builder,
    start: usize,
    /// `(slot, field position)` for each field written.
    fields: Vec<(usize, usize)>,
}

impl TableBuilder<'_> {
    /// Adds a scalar field unless it equals `default` (absent fields read back as the default).
    pub fn add_scalar<T: Scalar>(&mut self, slot: usize, value: T, default: T) {
        if value != default {
            let at = self.builder.push_scalar(value);
            self.fields.push((slot, at));
        }
    }

    /// Adds a field referring to a string, vector or sub-table written earlier.
    pub fn add_offset(&mut self, slot: usize, target: Offset) {
        let at = self.builder.push_offset(target);
        self.fields.push((slot, at));
    }

    /// Adds an inline struct given as its little-endian bytes, aligned to `align`.
    pub fn add_struct(&mut self, slot: usize, bytes: &[u8], align: usize) {
        self.builder.prep(align, 0);
        self.builder.rev.extend(bytes.iter().rev());
        self.fields.push((slot, self.builder.rev.len()));
    }

    /// Writes the vtable and the table header, and returns the table.
    pub fn finish(self) -> Result<Offset> {
        let TableBuilder {
            builder,
            start,
            fields,
        } = self;
        let table = builder.push_scalar(0i32);
        let slots = fields.iter().map(|(slot, _)| slot + 1).max().unwrap_or(0);
        let mut entries = vec![0u16; slots];
        for (slot, at) in fields {
            entries[slot] = u16::try_from(table - at).map_err(|_| Error::TooLarge)?;
        }
        for entry in entries.iter().rev() {
            entry.push_rev(&mut builder.rev);
        }
        let table_len = u16::try_from(table - start).map_err(|_| Error::TooLarge)?;
        let vtable_len = u16::try_from(4 + 2 * slots).map_err(|_| Error::TooLarge)?;
        table_len.push_rev(&mut builder.rev);
        vtable_len.push_rev(&mut builder.rev);
        let vtable = builder.rev.len();
        // The vtable sits before the table, so the table's signed offset to it is positive.
        let soffset = i32::try_from(vtable - table).map_err(|_| Error::TooLarge)?;
        let end = table;
        builder.rev[end - 4..end].copy_from_slice(&soffset.to_be_bytes());
        Ok(Offset(table))
    }
}
