//! HPACK header compression (RFC 7541).

pub mod decoder;
pub mod dynamic_table;
pub mod encoder;
pub mod huffman;
pub mod primitive;
pub mod static_table;

pub use decoder::Decoder;
pub use encoder::Encoder;

/// The default SETTINGS_HEADER_TABLE_SIZE / dynamic table capacity (RFC
/// 9113 §6.5.2).
pub const DEFAULT_HEADER_TABLE_SIZE: usize = 4096;

/// A single decoded (or to-be-encoded) header field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderField {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    /// RFC 7541 §7.1.3: never re-index or re-compress this field (e.g. an
    /// `authorization` or `set-cookie` value) — forces the "literal never
    /// indexed" representation on encode.
    pub sensitive: bool,
}

impl HeaderField {
    pub fn new(name: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        HeaderField {
            name: name.into(),
            value: value.into(),
            sensitive: false,
        }
    }

    pub fn sensitive(name: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        HeaderField {
            name: name.into(),
            value: value.into(),
            sensitive: true,
        }
    }
}
