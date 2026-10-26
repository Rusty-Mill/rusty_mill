use crate::error::{ErrorCode, H2Error, Result};
use crate::hpack::dynamic_table::DynamicTable;
use crate::hpack::primitive::{decode_integer, decode_string};
use crate::hpack::{static_table, HeaderField, DEFAULT_HEADER_TABLE_SIZE};

/// An HPACK decoder. Holds the dynamic table state built up from the
/// header blocks it has processed so far.
pub struct Decoder {
    dynamic_table: DynamicTable,
    /// The SETTINGS_HEADER_TABLE_SIZE we have advertised to the peer; a
    /// Dynamic Table Size Update instruction may not exceed this (RFC 7541
    /// §6.3).
    max_size_limit: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(DEFAULT_HEADER_TABLE_SIZE)
    }
}

impl Decoder {
    pub fn new(max_size: usize) -> Self {
        Decoder {
            dynamic_table: DynamicTable::new(max_size),
            max_size_limit: max_size,
        }
    }

    /// Number of entries currently held in the dynamic table.
    pub fn dynamic_table_len(&self) -> usize {
        self.dynamic_table.len()
    }

    /// Current total size (RFC 7541 §4.1 accounting) of the dynamic table.
    pub fn dynamic_table_size(&self) -> usize {
        self.dynamic_table.size()
    }

    /// Update the limit after we change our own SETTINGS_HEADER_TABLE_SIZE.
    pub fn set_max_size_limit(&mut self, max_size: usize) {
        self.max_size_limit = max_size;
        if self.dynamic_table.max_size() > max_size {
            self.dynamic_table.set_max_size(max_size);
        }
    }

    pub fn decode(&mut self, block: &[u8]) -> Result<Vec<HeaderField>> {
        Ok(self.decode_bounded(block, usize::MAX)?.unwrap_or_default())
    }

    /// Decodes `block`, keeping its fields only while their decoded size
    /// (RFC 9113 §6.5.2: name + value + 32 each) stays within `max`, and
    /// `None` once it does not. The whole block is still processed, so the
    /// dynamic table stays in step with the peer's encoder, but no field is
    /// copied out once over budget: a small block that repeats one large
    /// table entry cannot expand into memory.
    pub fn decode_bounded(
        &mut self,
        mut block: &[u8],
        max: usize,
    ) -> Result<Option<Vec<HeaderField>>> {
        let mut headers = Vec::new();
        let mut size = 0usize;
        let mut over = false;
        while !block.is_empty() {
            let first = block[0];
            if first & 0x80 != 0 {
                let (index, consumed) = decode_integer(block, 7)?;
                block = &block[consumed..];
                let len = self.entry_len(index as usize)?;
                if over || !within(&mut size, len, max) {
                    over = true;
                    continue;
                }
                let (name, value) = self.lookup(index as usize)?;
                headers.push(HeaderField::new(name, value));
            } else if first & 0x40 != 0 {
                let (consumed, header) = self.decode_literal(block, 6)?;
                self.dynamic_table
                    .insert(header.name.clone(), header.value.clone());
                block = &block[consumed..];
                over = over || !within(&mut size, header.name.len() + header.value.len(), max);
                if !over {
                    headers.push(header);
                }
            } else if first & 0x20 != 0 {
                let (max_size, consumed) = decode_integer(block, 5)?;
                let max_size = max_size as usize;
                if max_size > self.max_size_limit {
                    return Err(H2Error::Connection(
                        ErrorCode::CompressionError,
                        "Dynamic Table Size Update exceeds advertised SETTINGS_HEADER_TABLE_SIZE",
                    ));
                }
                self.dynamic_table.set_max_size(max_size);
                block = &block[consumed..];
            } else {
                // 0x10 = literal never indexed, 0x00 = literal without indexing;
                // both use a 4-bit prefix and are otherwise identical on the wire.
                let sensitive = first & 0x10 != 0;
                let (consumed, mut header) = self.decode_literal(block, 4)?;
                header.sensitive = sensitive;
                block = &block[consumed..];
                over = over || !within(&mut size, header.name.len() + header.value.len(), max);
                if !over {
                    headers.push(header);
                }
            }
            if over {
                // Free what was kept: none of it will be returned.
                headers = Vec::new();
            }
        }
        Ok((!over).then_some(headers))
    }

    /// The name and value length of table entry `index`, without copying it.
    fn entry_len(&self, index: usize) -> Result<usize> {
        let static_len = static_table::STATIC_TABLE.len();
        let len = match index {
            0 => None,
            i if i <= static_len => static_table::get(i).map(|(n, v)| n.len() + v.len()),
            i => self
                .dynamic_table
                .get(i - static_len)
                .map(|(n, v)| n.len() + v.len()),
        };
        len.ok_or(H2Error::Connection(
            ErrorCode::CompressionError,
            "HPACK index out of range or zero",
        ))
    }

    fn decode_literal(&self, block: &[u8], prefix_bits: u8) -> Result<(usize, HeaderField)> {
        let (index, mut consumed) = decode_integer(block, prefix_bits)?;
        let name = if index == 0 {
            let (name, len) = decode_string(&block[consumed..])?;
            consumed += len;
            name
        } else {
            self.lookup(index as usize)?.0.to_vec()
        };
        let (value, len) = decode_string(&block[consumed..])?;
        consumed += len;
        Ok((consumed, HeaderField::new(name, value)))
    }

    fn lookup(&self, index: usize) -> Result<(Vec<u8>, Vec<u8>)> {
        if index == 0 {
            return Err(H2Error::Connection(
                ErrorCode::CompressionError,
                "HPACK index 0 is not a valid header field reference",
            ));
        }
        let static_len = static_table::STATIC_TABLE.len();
        if index <= static_len {
            let (name, value) = static_table::get(index).expect("index checked against static_len");
            return Ok((name.as_bytes().to_vec(), value.as_bytes().to_vec()));
        }
        self.dynamic_table
            .get(index - static_len)
            .map(|(n, v)| (n.to_vec(), v.to_vec()))
            .ok_or(H2Error::Connection(
                ErrorCode::CompressionError,
                "HPACK index out of range",
            ))
    }
}

/// Adds a field of `len` name and value bytes to `size`; whether the list
/// is still within `max`.
fn within(size: &mut usize, len: usize, max: usize) -> bool {
    *size = size.saturating_add(len + 32);
    *size <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3000-byte field added to the dynamic table, then `refs` one-byte
    /// references to it (index 62, the newest dynamic entry).
    fn expanding_block(refs: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = crate::hpack::encoder::Encoder::new(DEFAULT_HEADER_TABLE_SIZE);
        enc.encode(&[HeaderField::new("x-big", vec![b'v'; 3000])], &mut out);
        out.extend(std::iter::repeat_n(0x80 | 62, refs));
        out
    }

    #[test]
    fn a_block_over_budget_is_refused_without_its_fields() {
        let mut dec = Decoder::default();
        // 1000 references expand to about 3 MiB; the budget is 8 KiB.
        assert_eq!(
            dec.decode_bounded(&expanding_block(1000), 8192).unwrap(),
            None
        );
        // The table still holds the entry, so the next block, which refers
        // to it, decodes as the peer's encoder expects.
        let next = dec.decode_bounded(&[0x80 | 62], 8192).unwrap().unwrap();
        assert_eq!(next[0].name, b"x-big");
        assert_eq!(next[0].value.len(), 3000);
    }

    #[test]
    fn a_block_within_budget_keeps_every_field() {
        let mut dec = Decoder::default();
        let fields = dec
            .decode_bounded(&expanding_block(1), 2 * (3000 + 5 + 32))
            .unwrap()
            .unwrap();
        assert_eq!(fields.len(), 2);
        // Exactly two fields' size is within; one byte less is not.
        let mut dec = Decoder::default();
        assert_eq!(
            dec.decode_bounded(&expanding_block(1), 2 * (3000 + 5 + 32) - 1)
                .unwrap(),
            None
        );
    }

    #[test]
    fn rejects_index_zero() {
        let mut dec = Decoder::default();
        assert!(dec.decode(&[0x80]).is_err());
    }

    #[test]
    fn rejects_out_of_range_index() {
        let mut dec = Decoder::default();
        // 0xff selects the "needs continuation" prefix value (127), plus a
        // continuation byte pushing the index well past the static table.
        assert!(dec.decode(&[0xff, 0x7f]).is_err());
    }

    #[test]
    fn rejects_oversized_dynamic_table_size_update() {
        let mut dec = Decoder::new(100);
        let mut out = Vec::new();
        crate::hpack::primitive::encode_integer(&mut out, 5, 0x20, 200);
        assert!(dec.decode(&out).is_err());
    }

    #[test]
    fn literal_without_indexing_does_not_grow_table() {
        let mut dec = Decoder::default();
        // Force "without indexing" by using the raw wire format directly:
        // 0000_0000 (new name, no indexing), then two string literals.
        let mut block = vec![0x00];
        crate::hpack::primitive::encode_string(&mut block, b"x-custom");
        crate::hpack::primitive::encode_string(&mut block, b"val");
        let decoded = dec.decode(&block).unwrap();
        assert_eq!(decoded, vec![HeaderField::new("x-custom", "val")]);
        assert_eq!(dec.dynamic_table.len(), 0);
    }
}
