//! A strict, minimal DER reader: just enough for RSAPublicKey and ECDSA
//! signatures. Rejects non-minimal lengths and integers, indefinite lengths,
//! and trailing data, as `ring` does.

/// A cursor over DER bytes.
pub(crate) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }

    /// Reads one TLV with tag `tag`, returning its contents.
    pub(crate) fn tlv(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.take(1)?[0] != tag {
            return None;
        }
        let first = self.take(1)?[0];
        let len = if first < 0x80 {
            first as usize
        } else {
            let count = (first & 0x7f) as usize;
            // Long form only for lengths above 127, in the fewest bytes.
            if count == 0 || count > 2 {
                return None;
            }
            let bytes = self.take(count)?;
            if bytes[0] == 0 {
                return None;
            }
            let len = bytes.iter().fold(0usize, |acc, &b| (acc << 8) | b as usize);
            if len < 0x80 {
                return None;
            }
            len
        };
        self.take(len)
    }

    /// Reads a positive INTEGER and returns its magnitude without the sign byte.
    pub(crate) fn positive_integer(&mut self) -> Option<&'a [u8]> {
        let body = self.tlv(0x02)?;
        match body {
            [] => None,
            [first, ..] if first & 0x80 != 0 => None,
            [0, rest @ ..] => match rest.first() {
                Some(b) if b & 0x80 != 0 => Some(rest),
                _ => None,
            },
            _ => Some(body),
        }
        .filter(|magnitude| magnitude.iter().any(|&b| b != 0))
    }

    /// Whether every byte has been consumed.
    pub(crate) fn at_end(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn reads_sequence_of_integers() {
        let der = [0x30, 0x07, 0x02, 0x02, 0x00, 0x80, 0x02, 0x01, 0x05];
        let mut r = Reader::new(&der);
        let body = r.tlv(0x30).unwrap();
        assert!(r.at_end());
        let mut inner = Reader::new(body);
        assert_eq!(inner.positive_integer().unwrap(), &[0x80]);
        assert_eq!(inner.positive_integer().unwrap(), &[0x05]);
        assert!(inner.at_end());
    }

    #[test]
    fn rejects_non_minimal_and_negative_integers() {
        assert!(Reader::new(&[2, 2, 0, 1]).positive_integer().is_none());
        assert!(Reader::new(&[2, 1, 0x80]).positive_integer().is_none());
        assert!(Reader::new(&[2, 0]).positive_integer().is_none());
        assert!(Reader::new(&[2, 1, 0]).positive_integer().is_none());
    }

    #[test]
    fn rejects_non_minimal_two_byte_length_even_with_a_full_body() {
        // 0x82 0x00 0x90 encodes 144, which fits the one-byte form 0x81 0x90.
        let mut der = vec![0x30, 0x82, 0x00, 0x90];
        der.extend(vec![0u8; 0x90]);
        assert!(Reader::new(&der).tlv(0x30).is_none());
        // The minimal spelling of the same length is accepted.
        let mut ok = vec![0x30, 0x81, 0x90];
        ok.extend(vec![0u8; 0x90]);
        assert_eq!(Reader::new(&ok).tlv(0x30).unwrap().len(), 0x90);
    }

    #[test]
    fn rejects_bad_lengths() {
        assert!(Reader::new(&[0x30, 0x81, 0x05, 0, 0, 0, 0, 0])
            .tlv(0x30)
            .is_none());
        assert!(Reader::new(&[0x30, 0x80]).tlv(0x30).is_none());
        assert!(Reader::new(&[0x30, 0x05, 1]).tlv(0x30).is_none());
        assert!(Reader::new(&[0x31, 0]).tlv(0x30).is_none());
        assert!(Reader::new(&[0x30, 0x82, 0x00, 0x90]).tlv(0x30).is_none());
    }
}
