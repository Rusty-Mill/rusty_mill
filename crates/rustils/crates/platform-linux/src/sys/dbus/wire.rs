//! D-Bus type-system marshaling (little-endian only — see `mod.rs`'s
//! doc comment). Signature-driven, not value-driven: [`marshal`]/
//! [`unmarshal`] walk a signature string one complete type at a time
//! (`value::split_one`) and marshal/unmarshal exactly one [`Value`] per
//! type, recursing into containers.
//!
//! Every alignment/padding rule here is asserted byte-for-byte in this
//! module's own tests, not just round-tripped — a symmetric encode/
//! decode bug (e.g. consistently forgetting one padding rule on both
//! sides) would still pass a round-trip test and be wrong on the wire.

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use super::value::{align_of, bad_signature, split_one, type_len, Value};

fn pad_to(buf: &mut Vec<u8>, align: usize) {
    while buf.len() % align != 0 {
        buf.push(0);
    }
}

fn truncated() -> PlatformError {
    PlatformError::new(
        ErrorKind::InvalidInput,
        OsCode::None,
        "D-Bus message truncated (need more bytes)",
    )
}

fn take<'a>(buf: &'a [u8], offset: &mut usize, n: usize) -> Result<&'a [u8]> {
    if *offset + n > buf.len() {
        return Err(truncated());
    }
    let s = &buf[*offset..*offset + n];
    *offset += n;
    Ok(s)
}

fn align_read(buf: &[u8], offset: &mut usize, align: usize) -> Result<()> {
    let target = offset.div_ceil(align) * align;
    if target > buf.len() {
        return Err(truncated());
    }
    *offset = target;
    Ok(())
}

/// Marshal `values` (one per complete type in `sig`) onto the end of
/// `buf`, applying every type's own alignment padding first.
pub fn marshal(sig: &str, values: &[Value], buf: &mut Vec<u8>) -> Result<()> {
    let mut remaining = sig;
    let mut vi = 0usize;
    while !remaining.is_empty() {
        let (this_ty, rest) = split_one(remaining)?;
        let v = values
            .get(vi)
            .ok_or_else(|| bad_signature("fewer values than the signature requires"))?;
        marshal_one(this_ty, v, buf)?;
        vi += 1;
        remaining = rest;
    }
    if vi != values.len() {
        return Err(bad_signature("more values than the signature requires"));
    }
    Ok(())
}

fn marshal_one(ty: &str, v: &Value, buf: &mut Vec<u8>) -> Result<()> {
    let c = ty.as_bytes()[0];
    macro_rules! num {
        ($align:expr, $variant:path, $to_bytes:ident) => {{
            pad_to(buf, $align);
            let $variant(x) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            buf.extend_from_slice(&x.$to_bytes());
        }};
    }
    match c {
        b'y' => {
            let Value::Byte(x) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            buf.push(*x);
        }
        b'b' => {
            pad_to(buf, 4);
            let Value::Boolean(x) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            buf.extend_from_slice(&u32::from(*x).to_le_bytes());
        }
        b'n' => num!(2, Value::Int16, to_le_bytes),
        b'q' => num!(2, Value::UInt16, to_le_bytes),
        b'i' => num!(4, Value::Int32, to_le_bytes),
        b'u' => num!(4, Value::UInt32, to_le_bytes),
        b'x' => num!(8, Value::Int64, to_le_bytes),
        b't' => num!(8, Value::UInt64, to_le_bytes),
        b'd' => num!(8, Value::Double, to_le_bytes),
        b's' | b'o' => {
            pad_to(buf, 4);
            let s = match v {
                Value::String(s) | Value::ObjectPath(s) => s,
                _ => return Err(bad_signature("value does not match signature")),
            };
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
        }
        b'g' => {
            let Value::Signature(s) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            buf.push(s.len() as u8);
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
        }
        b'a' => {
            pad_to(buf, 4);
            let Value::Array(elem_sig, items) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            if elem_sig != &ty[1..] {
                return Err(bad_signature(
                    "array element signature does not match its own value's",
                ));
            }
            let len_pos = buf.len();
            buf.extend_from_slice(&[0u8; 4]);
            // Padding before the first element is present on the wire
            // but excluded from the length prefix (D-Bus spec: the
            // length covers only the element data).
            pad_to(buf, align_of(elem_sig.as_bytes()[0]));
            let content_start = buf.len();
            for item in items {
                marshal_one(elem_sig, item, buf)?;
            }
            let content_len = (buf.len() - content_start) as u32;
            buf[len_pos..len_pos + 4].copy_from_slice(&content_len.to_le_bytes());
        }
        b'(' => {
            pad_to(buf, 8);
            let Value::Struct(fields) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            marshal(&ty[1..ty.len() - 1], fields, buf)?;
        }
        b'v' => {
            let Value::Variant(inner) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            let inner_sig = super::value::signature_of(inner);
            marshal_one("g", &Value::Signature(inner_sig.clone()), buf)?;
            marshal_one(&inner_sig, inner, buf)?;
        }
        b'{' => {
            pad_to(buf, 8);
            let Value::DictEntry(k, val) = v else {
                return Err(bad_signature("value does not match signature"));
            };
            marshal_one(&ty[1..2], k, buf)?;
            marshal_one(&ty[2..ty.len() - 1], val, buf)?;
        }
        _ => return Err(bad_signature("unsupported type code")),
    }
    Ok(())
}

/// Unmarshal one [`Value`] per complete type in `sig`, starting at
/// `*offset` (which is byte offset zero of the *message*, not of `buf` —
/// callers slicing a message apart still pass the message-relative
/// offset so alignment padding is computed against the right origin).
pub fn unmarshal(sig: &str, buf: &[u8], offset: &mut usize) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut remaining = sig;
    while !remaining.is_empty() {
        let (this_ty, rest) = split_one(remaining)?;
        out.push(unmarshal_one(this_ty, buf, offset)?);
        remaining = rest;
    }
    Ok(out)
}

fn unmarshal_one(ty: &str, buf: &[u8], offset: &mut usize) -> Result<Value> {
    let c = ty.as_bytes()[0];
    macro_rules! num {
        ($align:expr, $n:expr, $ty:ty, $variant:path) => {{
            align_read(buf, offset, $align)?;
            let bytes: [u8; $n] = take(buf, offset, $n)?.try_into().unwrap();
            $variant(<$ty>::from_le_bytes(bytes))
        }};
    }
    Ok(match c {
        b'y' => Value::Byte(take(buf, offset, 1)?[0]),
        b'b' => {
            align_read(buf, offset, 4)?;
            let bytes: [u8; 4] = take(buf, offset, 4)?.try_into().unwrap();
            Value::Boolean(u32::from_le_bytes(bytes) != 0)
        }
        b'n' => num!(2, 2, i16, Value::Int16),
        b'q' => num!(2, 2, u16, Value::UInt16),
        b'i' => num!(4, 4, i32, Value::Int32),
        b'u' => num!(4, 4, u32, Value::UInt32),
        b'x' => num!(8, 8, i64, Value::Int64),
        b't' => num!(8, 8, u64, Value::UInt64),
        b'd' => num!(8, 8, f64, Value::Double),
        b's' | b'o' => {
            align_read(buf, offset, 4)?;
            let len_bytes: [u8; 4] = take(buf, offset, 4)?.try_into().unwrap();
            let len = u32::from_le_bytes(len_bytes) as usize;
            let s = take(buf, offset, len)?.to_vec();
            take(buf, offset, 1)?; // trailing NUL
            let s = String::from_utf8(s)
                .map_err(|_| bad_signature("string body is not valid UTF-8"))?;
            if c == b's' {
                Value::String(s)
            } else {
                Value::ObjectPath(s)
            }
        }
        b'g' => {
            let len = take(buf, offset, 1)?[0] as usize;
            let s = take(buf, offset, len)?.to_vec();
            take(buf, offset, 1)?; // trailing NUL
            Value::Signature(
                String::from_utf8(s)
                    .map_err(|_| bad_signature("signature body is not valid UTF-8"))?,
            )
        }
        b'a' => {
            align_read(buf, offset, 4)?;
            let len_bytes: [u8; 4] = take(buf, offset, 4)?.try_into().unwrap();
            let byte_len = u32::from_le_bytes(len_bytes) as usize;
            let elem_sig = &ty[1..];
            align_read(buf, offset, align_of(elem_sig.as_bytes()[0]))?;
            let content_end = *offset + byte_len;
            if content_end > buf.len() {
                return Err(truncated());
            }
            let mut items = Vec::new();
            while *offset < content_end {
                items.push(unmarshal_one(elem_sig, buf, offset)?);
            }
            if *offset != content_end {
                return Err(bad_signature(
                    "array element did not end exactly at the declared length",
                ));
            }
            Value::Array(elem_sig.to_string(), items)
        }
        b'(' => {
            align_read(buf, offset, 8)?;
            let fields = unmarshal(&ty[1..ty.len() - 1], buf, offset)?;
            Value::Struct(fields)
        }
        b'v' => {
            let Value::Signature(inner_sig) = unmarshal_one("g", buf, offset)? else {
                unreachable!("unmarshal_one(\"g\", ..) always returns Value::Signature")
            };
            if type_len(inner_sig.as_bytes())? != inner_sig.len() {
                return Err(bad_signature(
                    "variant signature is not a single complete type",
                ));
            }
            Value::Variant(Box::new(unmarshal_one(&inner_sig, buf, offset)?))
        }
        b'{' => {
            align_read(buf, offset, 8)?;
            let inner = &ty[1..ty.len() - 1];
            let (key_ty, val_ty) = split_one(inner)?;
            let key = unmarshal_one(key_ty, buf, offset)?;
            let val = unmarshal_one(val_ty, buf, offset)?;
            Value::DictEntry(Box::new(key), Box::new(val))
        }
        _ => return Err(bad_signature("unsupported type code")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(sig: &str, values: Vec<Value>) {
        let mut buf = Vec::new();
        marshal(sig, &values, &mut buf).expect("marshal");
        let mut offset = 0;
        let decoded = unmarshal(sig, &buf, &mut offset).expect("unmarshal");
        assert_eq!(decoded, values);
        assert_eq!(offset, buf.len(), "unmarshal must consume every byte");
    }

    #[test]
    fn byte_has_no_padding() {
        let mut buf = Vec::new();
        marshal("y", &[Value::Byte(0x42)], &mut buf).unwrap();
        assert_eq!(buf, vec![0x42]);
    }

    #[test]
    fn uint32_pads_to_four() {
        let mut buf = vec![0xAA]; // one byte already in the buffer
        marshal("u", &[Value::UInt32(1)], &mut buf).unwrap();
        // 1 pad byte to reach offset 4, then the 4-byte little-endian value.
        assert_eq!(buf, vec![0xAA, 0, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn string_is_length_prefixed_and_nul_terminated() {
        let mut buf = Vec::new();
        marshal("s", &[Value::String("hi".into())], &mut buf).unwrap();
        // 4-byte LE length (2), "hi", trailing NUL.
        assert_eq!(buf, vec![2, 0, 0, 0, b'h', b'i', 0]);
    }

    #[test]
    fn signature_type_has_a_one_byte_length_prefix() {
        let mut buf = Vec::new();
        marshal("g", &[Value::Signature("ai".into())], &mut buf).unwrap();
        assert_eq!(buf, vec![2, b'a', b'i', 0]);
    }

    #[test]
    fn struct_aligns_to_eight_regardless_of_fields() {
        let mut buf = vec![0u8; 3]; // offset 3
        marshal(
            "(yu)",
            &[Value::Struct(vec![Value::Byte(1), Value::UInt32(2)])],
            &mut buf,
        )
        .unwrap();
        // Padded to 8, then byte 1, then 3 pad bytes (uint32 needs 4-align
        // from struct start: offset 8 + 1 = 9, pad to 12), then uint32 LE.
        assert_eq!(
            buf,
            vec![
                0, 0, 0, /* pad to 8 */ 0, 0, 0, 0, 0, /* byte */ 1, /* pad */ 0, 0,
                0, /* u32 */ 2, 0, 0, 0
            ]
        );
    }

    #[test]
    fn array_length_excludes_its_own_alignment_padding() {
        // Array of UINT64 (8-byte aligned elements): the 4 padding bytes
        // between the length prefix and the first element must NOT be
        // counted in the length value itself.
        let mut buf = Vec::new();
        marshal(
            "at",
            &[Value::Array("t".into(), vec![Value::UInt64(7)])],
            &mut buf,
        )
        .unwrap();
        let declared_len = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(
            declared_len, 8,
            "length must be just the one UINT64, not +4 padding"
        );
        assert_eq!(buf.len(), 4 + 4 /* padding */ + 8 /* the u64 */);
    }

    #[test]
    fn variant_carries_its_own_signature() {
        let mut buf = Vec::new();
        marshal(
            "v",
            &[Value::Variant(Box::new(Value::UInt32(42)))],
            &mut buf,
        )
        .unwrap();
        // signature "u" (1-byte len + "u" + NUL) then pad-to-4 then the u32.
        assert_eq!(buf, vec![1, b'u', 0, 0, 42, 0, 0, 0]);
    }

    #[test]
    fn roundtrips_every_basic_type() {
        roundtrip("y", vec![Value::Byte(255)]);
        roundtrip("b", vec![Value::Boolean(true)]);
        roundtrip("n", vec![Value::Int16(-1)]);
        roundtrip("q", vec![Value::UInt16(65535)]);
        roundtrip("i", vec![Value::Int32(-100)]);
        roundtrip("u", vec![Value::UInt32(100)]);
        roundtrip("x", vec![Value::Int64(-1)]);
        roundtrip("t", vec![Value::UInt64(u64::MAX)]);
        roundtrip("d", vec![Value::Double(1.5)]);
        roundtrip("s", vec![Value::String("hello".into())]);
        roundtrip("o", vec![Value::ObjectPath("/a/b".into())]);
        roundtrip("g", vec![Value::Signature("a{sv}".into())]);
    }

    #[test]
    fn roundtrips_nested_containers() {
        roundtrip(
            "a{sv}",
            vec![Value::Array(
                "{sv}".into(),
                vec![
                    Value::DictEntry(
                        Box::new(Value::String("k1".into())),
                        Box::new(Value::Variant(Box::new(Value::String("v1".into())))),
                    ),
                    Value::DictEntry(
                        Box::new(Value::String("k2".into())),
                        Box::new(Value::Variant(Box::new(Value::UInt32(9)))),
                    ),
                ],
            )],
        );
        roundtrip(
            "(sai)",
            vec![Value::Struct(vec![
                Value::String("x".into()),
                Value::Array("i".into(), vec![Value::Int32(1), Value::Int32(2)]),
            ])],
        );
    }

    #[test]
    fn roundtrips_an_empty_array() {
        roundtrip("as", vec![Value::Array("s".into(), vec![])]);
    }

    #[test]
    fn unmarshal_reports_truncated_not_a_panic() {
        let buf = vec![2, 0, 0, 0, b'h']; // string claims len 2 but only 1 byte follows
        let mut offset = 0;
        let e = unmarshal("s", &buf, &mut offset).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn marshal_rejects_a_value_that_does_not_match_the_signature() {
        let mut buf = Vec::new();
        let e = marshal("u", &[Value::String("nope".into())], &mut buf).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }
}
