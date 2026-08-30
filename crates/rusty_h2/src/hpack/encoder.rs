use crate::hpack::dynamic_table::DynamicTable;
use crate::hpack::primitive::{encode_integer, encode_string};
use crate::hpack::{static_table, HeaderField, DEFAULT_HEADER_TABLE_SIZE};

/// An HPACK encoder. Holds the dynamic table state mirroring what the
/// remote peer's decoder will build up as it processes our header blocks.
pub struct Encoder {
    dynamic_table: DynamicTable,
    pending_size_update: Option<usize>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new(DEFAULT_HEADER_TABLE_SIZE)
    }
}

impl Encoder {
    pub fn new(max_size: usize) -> Self {
        Encoder {
            dynamic_table: DynamicTable::new(max_size),
            pending_size_update: None,
        }
    }

    /// Record a change to the peer-advertised SETTINGS_HEADER_TABLE_SIZE.
    /// The corresponding Dynamic Table Size Update instruction is emitted
    /// at the start of the next call to [`encode`](Self::encode).
    pub fn set_max_dynamic_table_size(&mut self, max_size: usize) {
        self.pending_size_update = Some(max_size);
    }

    /// Encode a full header list into a single header block. Splitting the
    /// block across HEADERS/CONTINUATION frames is the caller's job.
    pub fn encode(&mut self, headers: &[HeaderField], out: &mut Vec<u8>) {
        if let Some(max_size) = self.pending_size_update.take() {
            encode_integer(out, 5, 0x20, max_size as u64);
            self.dynamic_table.set_max_size(max_size);
        }

        for header in headers {
            self.encode_one(header, out);
        }
    }

    fn encode_one(&mut self, header: &HeaderField, out: &mut Vec<u8>) {
        let static_hit = static_table::find(&header.name, &header.value);
        let dynamic_hit = self.dynamic_table.find(&header.name, &header.value);

        // Prefer an exact (name, value) match from either table; a static
        // exact match is checked first only because it's cheaper to compute.
        let exact = match (static_hit, dynamic_hit) {
            (Some((idx, true)), _) => Some(idx),
            (_, Some((idx, true))) => Some(static_table::STATIC_TABLE.len() + idx),
            _ => None,
        };
        if let Some(index) = exact {
            encode_integer(out, 7, 0x80, index as u64);
            return;
        }

        let name_index = match (static_hit, dynamic_hit) {
            (Some((idx, _)), _) => Some(idx),
            (_, Some((idx, _))) => Some(static_table::STATIC_TABLE.len() + idx),
            _ => None,
        };

        if header.sensitive {
            self.encode_literal(out, 4, 0x10, name_index, header, false);
        } else {
            self.encode_literal(out, 6, 0x40, name_index, header, true);
        }
    }

    fn encode_literal(
        &mut self,
        out: &mut Vec<u8>,
        prefix_bits: u8,
        prefix_pattern: u8,
        name_index: Option<usize>,
        header: &HeaderField,
        add_to_table: bool,
    ) {
        match name_index {
            Some(idx) => encode_integer(out, prefix_bits, prefix_pattern, idx as u64),
            None => {
                encode_integer(out, prefix_bits, prefix_pattern, 0);
                encode_string(out, &header.name);
            }
        }
        encode_string(out, &header.value);
        if add_to_table {
            self.dynamic_table
                .insert(header.name.clone(), header.value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hpack::decoder::Decoder;

    #[test]
    fn roundtrip_with_decoder() {
        let mut enc = Encoder::default();
        let mut dec = Decoder::default();
        let headers = vec![
            HeaderField::new(":method", "GET"),
            HeaderField::new(":path", "/"),
            HeaderField::new("custom-key", "custom-value"),
        ];
        let mut block = Vec::new();
        enc.encode(&headers, &mut block);
        let decoded = dec.decode(&block).unwrap();
        assert_eq!(decoded, headers);
    }

    #[test]
    fn repeated_header_uses_dynamic_table_indexed_representation() {
        let mut enc = Encoder::default();
        let headers = vec![HeaderField::new("custom-key", "custom-value")];
        let mut first = Vec::new();
        enc.encode(&headers, &mut first);
        let mut second = Vec::new();
        enc.encode(&headers, &mut second);
        // Second time round it's a single indexed byte (0x80 | index).
        assert_eq!(second.len(), 1);
        assert_eq!(second[0] & 0x80, 0x80);
    }

    #[test]
    fn sensitive_header_never_indexed_and_not_stored() {
        let mut enc = Encoder::default();
        let mut dec = Decoder::default();
        let headers = vec![HeaderField::sensitive("authorization", "secret")];
        let mut block = Vec::new();
        enc.encode(&headers, &mut block);
        assert_eq!(block[0] & 0xf0, 0x10);
        let decoded = dec.decode(&block).unwrap();
        assert_eq!(decoded, headers);
        assert!(decoded[0].sensitive);
    }

    #[test]
    fn dynamic_table_size_update_is_emitted_once() {
        let mut enc = Encoder::default();
        enc.set_max_dynamic_table_size(0);
        let mut out = Vec::new();
        enc.encode(&[HeaderField::new("a", "b")], &mut out);
        assert_eq!(out[0] & 0xe0, 0x20);
        // Subsequent calls shouldn't repeat the size update: the first byte
        // should go straight to a literal representation (0x40 prefix),
        // not the size-update pattern (0x20).
        let mut out2 = Vec::new();
        enc.encode(&[HeaderField::new("a", "b")], &mut out2);
        assert_ne!(out2[0] & 0xe0, 0x20);
        assert_eq!(out2[0] & 0xc0, 0x40);
    }
}
