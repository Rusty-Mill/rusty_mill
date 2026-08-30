//! The subset of the D-Bus type system this client models, and the
//! signature grammar that drives marshaling (`wire.rs`).
//!
//! Signatures are the source of truth for marshaling, not `Value`
//! itself: distinguishing `INT32` from `UINT32`, or where a `STRUCT`'s
//! fields end, requires the signature string, not just the bytes. Every
//! container `Value` variant therefore carries enough of its own type
//! information ([`Value::Array`]'s element signature) that
//! [`signature_of`] can derive a complete signature for *any* value
//! without ambiguity — an empty array with no elements to inspect still
//! knows its own element type, unlike a design that tried to infer
//! signatures from contents alone.

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

/// A D-Bus value. `h` (`UNIX_FDS`) has no variant — no consumer of this
/// client passes file descriptors over D-Bus.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Boolean(bool),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Double(f64),
    String(String),
    ObjectPath(String),
    Signature(String),
    /// The element type's complete D-Bus signature, then the elements —
    /// carried explicitly (not inferred from `Vec::first`) so an empty
    /// array still has a well-defined signature.
    Array(String, Vec<Value>),
    Struct(Vec<Value>),
    Variant(Box<Value>),
    DictEntry(Box<Value>, Box<Value>),
}

pub(super) fn bad_signature(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, detail)
}

/// This type's own alignment requirement in bytes, keyed by its leading
/// signature character (D-Bus spec §"Alignment").
pub(super) fn align_of(type_char: u8) -> usize {
    match type_char {
        b'y' | b'g' | b'v' => 1,
        b'n' | b'q' => 2,
        b'b' | b'i' | b'u' | b's' | b'o' | b'a' => 4,
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        _ => 1,
    }
}

/// The byte length of the single complete type at the front of `sig` —
/// `a` plus one complete type, `(...)`/`{...}` up to their matching
/// close bracket, or one byte for a basic-type code. Nested brackets of
/// the *same* kind (`(`/`)` inside a struct, `{`/`}` inside a dict
/// entry) are depth-counted independently, which is sufficient because
/// D-Bus signatures never reuse a bracket character across container
/// kinds.
pub(super) fn type_len(sig: &[u8]) -> Result<usize> {
    let Some(&first) = sig.first() else {
        return Err(bad_signature("empty signature where a type was expected"));
    };
    match first {
        b'a' => Ok(1 + type_len(&sig[1..])?),
        b'(' => bracketed_len(sig, b'(', b')'),
        b'{' => bracketed_len(sig, b'{', b'}'),
        b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g'
        | b'v' => Ok(1),
        _ => Err(bad_signature("unrecognized or unsupported type code")),
    }
}

fn bracketed_len(sig: &[u8], open: u8, close: u8) -> Result<usize> {
    let mut depth = 0i32;
    for (i, &b) in sig.iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Ok(i + 1);
            }
        }
    }
    Err(bad_signature("unterminated container type"))
}

/// Split the single complete type at the front of `sig` from the rest.
pub(super) fn split_one(sig: &str) -> Result<(&str, &str)> {
    let n = type_len(sig.as_bytes())?;
    Ok(sig.split_at(n))
}

/// The complete D-Bus signature for `v` — always derivable without
/// ambiguity, since [`Value::Array`] carries its own element signature
/// rather than requiring one to be inferred from (possibly zero)
/// elements.
pub fn signature_of(v: &Value) -> String {
    match v {
        Value::Byte(_) => "y".to_string(),
        Value::Boolean(_) => "b".to_string(),
        Value::Int16(_) => "n".to_string(),
        Value::UInt16(_) => "q".to_string(),
        Value::Int32(_) => "i".to_string(),
        Value::UInt32(_) => "u".to_string(),
        Value::Int64(_) => "x".to_string(),
        Value::UInt64(_) => "t".to_string(),
        Value::Double(_) => "d".to_string(),
        Value::String(_) => "s".to_string(),
        Value::ObjectPath(_) => "o".to_string(),
        Value::Signature(_) => "g".to_string(),
        Value::Array(elem_sig, _) => format!("a{elem_sig}"),
        Value::Struct(fields) => {
            let inner: String = fields.iter().map(signature_of).collect();
            format!("({inner})")
        }
        Value::Variant(_) => "v".to_string(),
        Value::DictEntry(k, val) => format!("{{{}{}}}", signature_of(k), signature_of(val)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_len_splits_basic_and_container_types() {
        assert_eq!(type_len(b"s").unwrap(), 1);
        assert_eq!(type_len(b"as").unwrap(), 2);
        assert_eq!(type_len(b"a{sv}").unwrap(), 5);
        assert_eq!(type_len(b"(iu)").unwrap(), 4);
        assert_eq!(type_len(b"(iu)s").unwrap(), 4); // stops at the matching ')'
        assert_eq!(type_len(b"a(a{sv}as)i").unwrap(), 10);
    }

    #[test]
    fn type_len_rejects_malformed_signatures() {
        assert!(type_len(b"").is_err());
        assert!(type_len(b"(iu").is_err());
        assert!(type_len(b"Q").is_err());
    }

    #[test]
    fn split_one_walks_a_full_signature() {
        let mut remaining = "sa{sv}i";
        let mut parts = Vec::new();
        while !remaining.is_empty() {
            let (this, rest) = split_one(remaining).unwrap();
            parts.push(this.to_string());
            remaining = rest;
        }
        assert_eq!(parts, vec!["s", "a{sv}", "i"]);
    }

    #[test]
    fn signature_of_derives_from_values_including_empty_arrays() {
        assert_eq!(signature_of(&Value::UInt32(1)), "u");
        assert_eq!(
            signature_of(&Value::Array("s".into(), vec![])),
            "as",
            "an empty array must still know its own element type"
        );
        assert_eq!(
            signature_of(&Value::Struct(vec![Value::Byte(1), Value::UInt32(2)])),
            "(yu)"
        );
        assert_eq!(
            signature_of(&Value::Variant(Box::new(Value::String("x".into())))),
            "v"
        );
        assert_eq!(
            signature_of(&Value::DictEntry(
                Box::new(Value::String("k".into())),
                Box::new(Value::Variant(Box::new(Value::Int32(1))))
            )),
            "{sv}"
        );
    }
}
