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

    pub fn decode(&mut self, mut block: &[u8]) -> Result<Vec<HeaderField>> {
        let mut headers = Vec::new();
        while !block.is_empty() {
            let first = block[0];
            if first & 0x80 != 0 {
                let (index, consumed) = decode_integer(block, 7)?;
                let (name, value) = self.lookup(index as usize)?;
                headers.push(HeaderField::new(name, value));
                block = &block[consumed..];
            } else if first & 0x40 != 0 {
                let (consumed, header) = self.decode_literal(block, 6)?;
                self.dynamic_table
                    .insert(header.name.clone(), header.value.clone());
                headers.push(header);
                block = &block[consumed..];
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
                headers.push(header);
                block = &block[consumed..];
            }
        }
        Ok(headers)
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

#[cfg(test)]
mod tests {
    use super::*;

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
