use rusty_protobuf::{Error, Field, Reader, Writer, DEFAULT_DEPTH_LIMIT};

/// `Result` with the wire layer's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// A hand-written message schema.
pub trait Message: Default + Sized {
    /// Writes the fields in field-number order. Proto3 implicit-presence fields equal to their
    /// default are omitted; `Option` fields are written when `Some`, even if zero.
    fn encode(&self, w: &mut Writer);

    /// Reads one occurrence of a field into `self`, applying the merge rules. A field number
    /// the schema does not know is skipped by returning `Ok(())`.
    fn merge_field(&mut self, f: Field<'_>) -> Result<()>;

    /// Merges every field of `r` into `self`.
    fn merge(&mut self, r: Reader<'_>) -> Result<()> {
        for f in r {
            self.merge_field(f?)?;
        }
        Ok(())
    }

    /// Decodes `buf` with the default nesting limit.
    fn decode(buf: &[u8]) -> Result<Self> {
        Self::decode_with_depth(buf, DEFAULT_DEPTH_LIMIT)
    }

    /// Decodes `buf`, refusing messages nested more than `limit` levels deep.
    fn decode_with_depth(buf: &[u8], limit: usize) -> Result<Self> {
        let mut m = Self::default();
        m.merge(Reader::with_depth_limit(buf, limit))?;
        Ok(m)
    }

    /// The encoded message, or [`Error::TooLarge`] past the format's 2 GiB limit.
    fn encode_to_vec(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        self.encode(&mut w);
        w.finish()
    }
}

/// The type URL a message is packed under in a [`wkt::Any`](crate::wkt::Any).
pub trait Name {
    /// For example `type.googleapis.com/google.rpc.ErrorInfo`.
    const TYPE_URL: &'static str;
}
