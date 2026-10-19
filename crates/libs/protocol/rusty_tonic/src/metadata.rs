/// Why a metadata key or value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataError {
    /// A key that is empty or has a character outside `[0-9a-z_.-]` (keys are lowercase).
    InvalidKey,
    /// A key the protocol owns: `grpc-*`, `content-type`, `te`, `user-agent`, pseudo-headers, ...
    ReservedKey,
    /// An ASCII value with a byte outside 0x20..=0x7E.
    InvalidValue,
    /// A `-bin` key given an ASCII value, or an ASCII key given a binary one.
    WrongKind,
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MetadataError::InvalidKey => "metadata key must be lowercase [0-9a-z_.-]",
            MetadataError::ReservedKey => "metadata key is reserved by the protocol",
            MetadataError::InvalidValue => "metadata value must be printable ASCII",
            MetadataError::WrongKind => "-bin keys take binary values and others take ASCII",
        })
    }
}

impl std::error::Error for MetadataError {}

/// Names the protocol owns, besides anything starting with `grpc-`.
const RESERVED: &[&str] = &[
    "te",
    "content-type",
    "user-agent",
    "host",
    "content-length",
    "content-encoding",
    "accept-encoding",
    "connection",
    "transfer-encoding",
    "keep-alive",
    "proxy-connection",
    "upgrade",
    "trailer",
];

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'_' | b'-' | b'.'))
}

fn is_reserved(key: &str) -> bool {
    key.starts_with("grpc-") || RESERVED.contains(&key)
}

fn is_binary_key(key: &str) -> bool {
    key.ends_with("-bin")
}

fn valid_ascii(value: &[u8]) -> bool {
    value.iter().all(|b| (0x20..=0x7e).contains(b))
}

fn check_key(key: &str) -> Result<(), MetadataError> {
    if !valid_key(key) {
        return Err(MetadataError::InvalidKey);
    }
    if is_reserved(key) {
        return Err(MetadataError::ReservedKey);
    }
    Ok(())
}

/// Decodes base64 with or without padding.
fn decode_base64(value: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(value).ok()?.trim_end_matches('=');
    let mut padded = String::with_capacity(text.len() + 2);
    padded.push_str(text);
    match text.len() % 4 {
        0 => {}
        2 => padded.push_str("=="),
        3 => padded.push('='),
        _ => return None,
    }
    rusty_base64::decode_standard(&padded).ok()
}

/// One metadata value, borrowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueRef<'a> {
    /// Printable ASCII, for a key not ending in `-bin`.
    Ascii(&'a str),
    /// Arbitrary bytes, for a key ending in `-bin`.
    Binary(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Ascii(String),
    Binary(Vec<u8>),
}

/// Call metadata: an ordered multimap with validated keys and values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MetadataMap {
    entries: Vec<(String, Value)>,
}

impl MetadataMap {
    /// An empty map.
    pub fn new() -> MetadataMap {
        MetadataMap::default()
    }

    /// Sets `key` to one ASCII value, replacing any it had.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), MetadataError> {
        self.check_ascii(key, value)?;
        self.remove(key);
        self.entries
            .push((key.to_string(), Value::Ascii(value.to_string())));
        Ok(())
    }

    /// Adds an ASCII value for `key`, keeping the ones it has.
    pub fn append(&mut self, key: &str, value: &str) -> Result<(), MetadataError> {
        self.check_ascii(key, value)?;
        self.entries
            .push((key.to_string(), Value::Ascii(value.to_string())));
        Ok(())
    }

    /// Sets a `-bin` key to one binary value, replacing any it had.
    pub fn insert_bin(&mut self, key: &str, value: &[u8]) -> Result<(), MetadataError> {
        self.check_binary(key)?;
        self.remove(key);
        self.entries
            .push((key.to_string(), Value::Binary(value.to_vec())));
        Ok(())
    }

    /// Adds a binary value for a `-bin` key, keeping the ones it has.
    pub fn append_bin(&mut self, key: &str, value: &[u8]) -> Result<(), MetadataError> {
        self.check_binary(key)?;
        self.entries
            .push((key.to_string(), Value::Binary(value.to_vec())));
        Ok(())
    }

    /// The first ASCII value for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find_map(|(k, v)| match v {
            Value::Ascii(s) if k == key => Some(s.as_str()),
            _ => None,
        })
    }

    /// Every ASCII value for `key`, in the order added.
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries.iter().filter_map(move |(k, v)| match v {
            Value::Ascii(s) if k == key => Some(s.as_str()),
            _ => None,
        })
    }

    /// The first binary value for a `-bin` key.
    pub fn get_bin(&self, key: &str) -> Option<&[u8]> {
        self.entries.iter().find_map(|(k, v)| match v {
            Value::Binary(b) if k == key => Some(b.as_slice()),
            _ => None,
        })
    }

    /// Removes every value for `key`.
    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|(k, _)| k != key);
    }

    /// Whether `key` has any value.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// The number of values (a key with two values counts twice).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no values.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every `(key, value)` in the order added.
    pub fn iter(&self) -> impl Iterator<Item = (&str, ValueRef<'_>)> {
        self.entries.iter().map(|(k, v)| {
            (
                k.as_str(),
                match v {
                    Value::Ascii(s) => ValueRef::Ascii(s),
                    Value::Binary(b) => ValueRef::Binary(b),
                },
            )
        })
    }

    /// The header list for this metadata: ASCII values as they are, binary values as base64
    /// without padding.
    pub fn to_headers(&self) -> Vec<(String, Vec<u8>)> {
        self.entries
            .iter()
            .map(|(k, v)| {
                let value = match v {
                    Value::Ascii(s) => s.clone().into_bytes(),
                    Value::Binary(b) => rusty_base64::encode_standard(b)
                        .trim_end_matches('=')
                        .as_bytes()
                        .to_vec(),
                };
                (k.clone(), value)
            })
            .collect()
    }

    /// Reads metadata from received headers. An entry that cannot be metadata (an invalid key
    /// or value, bad base64, a protocol-reserved name) is skipped, and the count is returned
    /// so the caller can notice. A `-bin` value may carry padding or not.
    pub fn from_headers<'a>(
        headers: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> (MetadataMap, usize) {
        let mut map = MetadataMap::new();
        let mut dropped = 0;
        for (name, value) in headers {
            if name.starts_with(':') {
                continue; // a pseudo-header, not metadata
            }
            if !valid_key(name) {
                dropped += 1;
                continue;
            }
            if is_reserved(name) {
                continue; // the protocol's, handled by whoever reads the head or trailers
            }
            let parsed = if is_binary_key(name) {
                decode_base64(value).map(Value::Binary)
            } else if valid_ascii(value) {
                std::str::from_utf8(value)
                    .ok()
                    .map(|s| Value::Ascii(s.to_string()))
            } else {
                None
            };
            match parsed {
                Some(v) => map.entries.push((name.to_string(), v)),
                None => dropped += 1,
            }
        }
        (map, dropped)
    }

    fn check_ascii(&self, key: &str, value: &str) -> Result<(), MetadataError> {
        check_key(key)?;
        if is_binary_key(key) {
            return Err(MetadataError::WrongKind);
        }
        if !valid_ascii(value.as_bytes()) {
            return Err(MetadataError::InvalidValue);
        }
        Ok(())
    }

    fn check_binary(&self, key: &str) -> Result<(), MetadataError> {
        check_key(key)?;
        if !is_binary_key(key) {
            return Err(MetadataError::WrongKind);
        }
        Ok(())
    }
}
