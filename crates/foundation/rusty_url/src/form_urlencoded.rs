//! Parsing and serialization for the
//! [`application/x-www-form-urlencoded` syntax](https://url.spec.whatwg.org/#application/x-www-form-urlencoded),
//! as used by HTML forms and URL query strings.

use std::borrow::{Borrow, Cow};

use crate::percent_encode::{percent_decode, percent_encode_byte};

/// A function that encodes a `str` to bytes before percent-encoding, used to
/// override the default (UTF-8) encoding of [`Serializer`] output.
pub type EncodingOverride<'a> = Option<&'a dyn Fn(&str) -> Cow<'_, [u8]>>;

/// Parse a `application/x-www-form-urlencoded` byte string into an iterator
/// of `(name, value)` pairs. Use `parse(s.as_bytes())` for a `&str`.
///
/// Names and values are percent-decoded (`+` decodes to a space).
pub fn parse(input: &[u8]) -> Parse<'_> {
    Parse { input }
}

/// The return type of [`parse`].
#[derive(Clone, Copy)]
pub struct Parse<'a> {
    input: &'a [u8],
}

impl<'a> Iterator for Parse<'a> {
    type Item = (Cow<'a, str>, Cow<'a, str>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.input.is_empty() {
                return None;
            }
            let mut split = self.input.splitn(2, |&b| b == b'&');
            let sequence = split.next().unwrap();
            self.input = split.next().unwrap_or(&[]);
            if sequence.is_empty() {
                continue;
            }
            let mut split = sequence.splitn(2, |&b| b == b'=');
            let name = split.next().unwrap();
            let value = split.next().unwrap_or(&[]);
            return Some((decode(name), decode(value)));
        }
    }
}

impl<'a> Parse<'a> {
    /// Return an equivalent iterator yielding `(String, String)` pairs
    /// instead of borrowed `Cow<str>` pairs.
    pub fn into_owned(self) -> ParseIntoOwned<'a> {
        ParseIntoOwned { inner: self }
    }
}

/// The return type of [`Parse::into_owned`].
pub struct ParseIntoOwned<'a> {
    inner: Parse<'a>,
}

impl Iterator for ParseIntoOwned<'_> {
    type Item = (String, String);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
    }
}

fn decode(input: &[u8]) -> Cow<'_, str> {
    let replaced = replace_plus(input);
    let decoded = match percent_decode(&replaced) {
        Cow::Owned(bytes) => Cow::Owned(bytes),
        Cow::Borrowed(_) => replaced,
    };
    decode_utf8_lossy(decoded)
}

fn replace_plus(input: &[u8]) -> Cow<'_, [u8]> {
    if !input.contains(&b'+') {
        return Cow::Borrowed(input);
    }
    let mut replaced = input.to_owned();
    for byte in &mut replaced {
        if *byte == b'+' {
            *byte = b' ';
        }
    }
    Cow::Owned(replaced)
}

fn decode_utf8_lossy(input: Cow<'_, [u8]>) -> Cow<'_, str> {
    match input {
        Cow::Borrowed(bytes) => String::from_utf8_lossy(bytes),
        Cow::Owned(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

fn byte_serialized_unchanged(byte: u8) -> bool {
    matches!(byte, b'*' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'_' | b'a'..=b'z')
}

/// [`application/x-www-form-urlencoded` byte serializer](
/// https://url.spec.whatwg.org/#concept-urlencoded-byte-serializer): percent-encode
/// `input`, using `+` for spaces.
pub fn byte_serialize(input: &[u8]) -> ByteSerialize<'_> {
    ByteSerialize { bytes: input }
}

/// The return type of [`byte_serialize`].
#[derive(Debug)]
pub struct ByteSerialize<'a> {
    bytes: &'a [u8],
}

impl<'a> Iterator for ByteSerialize<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let (&first, tail) = self.bytes.split_first()?;
        if !byte_serialized_unchanged(first) {
            self.bytes = tail;
            return Some(if first == b' ' {
                "+"
            } else {
                percent_encode_byte(first)
            });
        }
        let position = tail.iter().position(|&b| !byte_serialized_unchanged(b));
        let (unchanged, remaining) = match position {
            Some(i) => self.bytes.split_at(1 + i),
            None => (self.bytes, &[][..]),
        };
        self.bytes = remaining;
        // SAFETY: `byte_serialized_unchanged` only accepts single-byte UTF-8.
        Some(unsafe { std::str::from_utf8_unchecked(unchanged) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.bytes.is_empty() {
            (0, Some(0))
        } else {
            (1, Some(self.bytes.len()))
        }
    }
}

/// A trait for the output of a [`Serializer`]: an in-progress `String` buffer
/// that can be finished into some result type.
pub trait Target {
    /// The type returned by [`Target::finish`].
    type Finished;
    /// Mutable access to the buffer being serialized into.
    fn as_mut_string(&mut self) -> &mut String;
    /// Consume `self`, returning the finished result.
    fn finish(self) -> Self::Finished;
}

impl Target for String {
    type Finished = Self;
    fn as_mut_string(&mut self) -> &mut String {
        self
    }
    fn finish(self) -> Self {
        self
    }
}

impl Target for &mut String {
    type Finished = Self;
    fn as_mut_string(&mut self) -> &mut String {
        self
    }
    fn finish(self) -> Self {
        self
    }
}

/// The [`application/x-www-form-urlencoded` serializer](
/// https://url.spec.whatwg.org/#concept-urlencoded-serializer): builds a
/// `name=value&name=value` string incrementally into a [`Target`].
pub struct Serializer<'a, T: Target> {
    target: Option<T>,
    start_position: usize,
    encoding: EncodingOverride<'a>,
}

impl<'a, T: Target> Serializer<'a, T> {
    /// Create a serializer appending to `target`. If `target` is non-empty,
    /// its content is assumed to already be `application/x-www-form-urlencoded`.
    pub fn new(target: T) -> Self {
        Self::for_suffix(target, 0)
    }

    /// Create a serializer that only clears/appends after byte offset
    /// `start_position` in `target`, treating anything before it as an
    /// untouched prefix.
    ///
    /// # Panics
    ///
    /// Panics if `start_position` is greater than `target`'s current length.
    pub fn for_suffix(mut target: T, start_position: usize) -> Self {
        assert!(
            target.as_mut_string().len() >= start_position,
            "invalid length {start_position} for target of length {}",
            target.as_mut_string().len()
        );
        Serializer {
            target: Some(target),
            start_position,
            encoding: None,
        }
    }

    /// Truncate the target back to `start_position`, removing all pairs
    /// appended so far.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Serializer::finish`].
    pub fn clear(&mut self) -> &mut Self {
        buffer(&mut self.target).truncate(self.start_position);
        self
    }

    /// Set the byte encoding applied to names/values before percent-encoding
    /// (default: UTF-8).
    pub fn encoding_override(&mut self, new: EncodingOverride<'a>) -> &mut Self {
        self.encoding = new;
        self
    }

    /// Append a `name=value` pair.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Serializer::finish`].
    pub fn append_pair(&mut self, name: &str, value: &str) -> &mut Self {
        append_separator_if_needed(buffer(&mut self.target), self.start_position);
        append_encoded(name, buffer(&mut self.target), self.encoding);
        buffer(&mut self.target).push('=');
        append_encoded(value, buffer(&mut self.target), self.encoding);
        self
    }

    /// Append a bare `name` with no `=value`.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Serializer::finish`].
    pub fn append_key_only(&mut self, name: &str) -> &mut Self {
        append_separator_if_needed(buffer(&mut self.target), self.start_position);
        append_encoded(name, buffer(&mut self.target), self.encoding);
        self
    }

    /// Call [`Serializer::append_pair`] once per item.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Serializer::finish`].
    pub fn extend_pairs<I, K, V>(&mut self, iter: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for pair in iter {
            let (k, v) = pair.borrow();
            self.append_pair(k.as_ref(), v.as_ref());
        }
        self
    }

    /// Call [`Serializer::append_key_only`] once per item.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Serializer::finish`].
    pub fn extend_keys_only<I, K>(&mut self, iter: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: Borrow<K>,
        K: AsRef<str>,
    {
        for key in iter {
            self.append_key_only(key.borrow().as_ref());
        }
        self
    }

    /// Consume the serializer, returning its target's [`Target::finish`].
    ///
    /// # Panics
    ///
    /// Panics if called more than once.
    pub fn finish(&mut self) -> T::Finished {
        self.target
            .take()
            .expect("form_urlencoded::Serializer double finish")
            .finish()
    }
}

fn buffer<T: Target>(target: &mut Option<T>) -> &mut String {
    target
        .as_mut()
        .expect("form_urlencoded::Serializer used after finish")
        .as_mut_string()
}

fn append_separator_if_needed(buffer: &mut String, start_position: usize) {
    if buffer.len() > start_position {
        buffer.push('&');
    }
}

fn append_encoded(input: &str, buffer: &mut String, encoding: EncodingOverride<'_>) {
    let bytes = match encoding {
        Some(encode) => encode(input),
        None => Cow::Borrowed(input.as_bytes()),
    };
    buffer.extend(byte_serialize(&bytes));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decodes_plus_and_percent() {
        let pairs: Vec<_> = parse(b"a=1&b=2+2&c=%23hash").collect();
        assert_eq!(
            pairs,
            vec![
                (Cow::Borrowed("a"), Cow::Borrowed("1")),
                (Cow::Borrowed("b"), Cow::Borrowed("2 2")),
                (Cow::Borrowed("c"), Cow::Borrowed("#hash")),
            ]
        );
    }

    #[test]
    fn parse_key_only_pair_has_empty_value() {
        let pairs: Vec<_> = parse(b"flag&k=v").collect();
        assert_eq!(
            pairs,
            vec![
                (Cow::Borrowed("flag"), Cow::Borrowed("")),
                (Cow::Borrowed("k"), Cow::Borrowed("v")),
            ]
        );
    }

    #[test]
    fn parse_skips_empty_sequences() {
        let pairs: Vec<_> = parse(b"a=1&&b=2").collect();
        assert_eq!(
            pairs,
            vec![
                (Cow::Borrowed("a"), Cow::Borrowed("1")),
                (Cow::Borrowed("b"), Cow::Borrowed("2")),
            ]
        );
    }

    #[test]
    fn into_owned_yields_strings() {
        let pairs: Vec<(String, String)> = parse(b"a=1").into_owned().collect();
        assert_eq!(pairs, vec![("a".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn byte_serialize_uses_plus_for_space() {
        assert_eq!(
            byte_serialize(b"foo bar?").collect::<String>(),
            "foo+bar%3F"
        );
    }

    #[test]
    fn serializer_round_trips_with_parser() {
        let encoded = Serializer::new(String::new())
            .append_pair("foo", "bar & baz")
            .append_pair("saison", "\u{c9}t\u{e9}+hiver")
            .finish();
        assert_eq!(encoded, "foo=bar+%26+baz&saison=%C3%89t%C3%A9%2Bhiver");
        let decoded: Vec<(String, String)> = parse(encoded.as_bytes()).into_owned().collect();
        assert_eq!(
            decoded,
            vec![
                ("foo".to_owned(), "bar & baz".to_owned()),
                ("saison".to_owned(), "\u{c9}t\u{e9}+hiver".to_owned()),
            ]
        );
    }

    #[test]
    fn serializer_append_key_only() {
        let encoded = Serializer::new(String::new())
            .append_key_only("flag")
            .append_pair("k", "v")
            .finish();
        assert_eq!(encoded, "flag&k=v");
    }

    #[test]
    fn serializer_extend_pairs_and_keys_only() {
        let encoded = Serializer::new(String::new())
            .extend_pairs([("a", "1"), ("b", "2")])
            .extend_keys_only::<_, &str>(["c", "d"])
            .finish();
        assert_eq!(encoded, "a=1&b=2&c&d");
    }

    #[test]
    fn serializer_clear() {
        let mut serializer = Serializer::new(String::new());
        serializer.append_pair("a", "1");
        serializer.clear();
        serializer.append_pair("b", "2");
        assert_eq!(serializer.finish(), "b=2");
    }

    #[test]
    fn serializer_for_suffix_preserves_prefix() {
        let mut target = "existing".to_owned();
        let start = target.len();
        let encoded = Serializer::for_suffix(&mut target, start)
            .append_pair("a", "1")
            .finish();
        assert_eq!(*encoded, "existinga=1");
    }

    #[test]
    #[should_panic]
    fn serializer_for_suffix_panics_on_out_of_range_start() {
        let _ = Serializer::for_suffix(String::new(), 5);
    }

    #[test]
    fn serializer_encoding_override() {
        let encoded = Serializer::new(String::new())
            .encoding_override(Some(&|s: &str| {
                Cow::Owned(s.as_bytes().to_ascii_uppercase())
            }))
            .append_pair("a", "bc")
            .finish();
        assert_eq!(encoded, "A=BC");
    }

    #[test]
    #[should_panic]
    fn serializer_double_finish_panics() {
        let mut serializer = Serializer::new(String::new());
        serializer.append_pair("a", "1");
        serializer.finish();
        serializer.finish();
    }
}
