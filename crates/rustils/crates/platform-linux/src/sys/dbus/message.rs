//! A full D-Bus message: the 16-byte fixed header, the header fields
//! array (itself an ordinary `a(yv)` value — reuses `wire::marshal`/
//! `unmarshal` rather than hand-rolling a second encoder for it), and
//! the body. Little-endian only (`mod.rs`'s doc comment).

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use super::value::Value;
use super::wire::{marshal, unmarshal};

pub const TYPE_METHOD_CALL: u8 = 1;
pub const TYPE_METHOD_RETURN: u8 = 2;
pub const TYPE_ERROR: u8 = 3;
/// Reserved for future signal-subscription support (`AddMatch`) — no
/// consumer of this slice sends or filters on `SIGNAL` messages yet.
#[allow(dead_code)]
pub const TYPE_SIGNAL: u8 = 4;

const FIELD_PATH: u8 = 1;
const FIELD_INTERFACE: u8 = 2;
const FIELD_MEMBER: u8 = 3;
const FIELD_ERROR_NAME: u8 = 4;
const FIELD_REPLY_SERIAL: u8 = 5;
const FIELD_DESTINATION: u8 = 6;
const FIELD_SENDER: u8 = 7;
const FIELD_SIGNATURE: u8 = 8;

/// A D-Bus message. Construct with [`Message::method_call`] for the one
/// direction this client's transport sends; every other field is
/// populated from a parsed reply by `decode`.
#[derive(Debug, Clone, Default)]
pub struct Message {
    pub message_type: u8,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    /// The body's signature — empty (not `None`) when there is no body,
    /// matching the wire protocol's own "no SIGNATURE header field at
    /// all" contract for a bodyless message.
    pub signature: String,
    pub body: Vec<Value>,
}

impl Message {
    /// A `METHOD_CALL` addressed to `destination`/`path`/`interface`/
    /// `member`, with `body` marshaled per `signature`.
    pub fn method_call(
        serial: u32,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: Vec<Value>,
    ) -> Self {
        Message {
            message_type: TYPE_METHOD_CALL,
            flags: 0,
            serial,
            path: Some(path.to_string()),
            interface: Some(interface.to_string()),
            member: Some(member.to_string()),
            error_name: None,
            reply_serial: None,
            destination: Some(destination.to_string()),
            sender: None,
            signature: signature.to_string(),
            body,
        }
    }
}

fn field(code: u8, value: Value) -> Value {
    Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(value))])
}

/// Serialize `msg` to its full on-wire byte representation.
pub fn encode(msg: &Message) -> Result<Vec<u8>> {
    let mut body_bytes = Vec::new();
    if !msg.signature.is_empty() {
        marshal(&msg.signature, &msg.body, &mut body_bytes)?;
    }

    let mut fields = Vec::new();
    if let Some(p) = &msg.path {
        fields.push(field(FIELD_PATH, Value::ObjectPath(p.clone())));
    }
    if let Some(i) = &msg.interface {
        fields.push(field(FIELD_INTERFACE, Value::String(i.clone())));
    }
    if let Some(m) = &msg.member {
        fields.push(field(FIELD_MEMBER, Value::String(m.clone())));
    }
    if let Some(e) = &msg.error_name {
        fields.push(field(FIELD_ERROR_NAME, Value::String(e.clone())));
    }
    if let Some(rs) = msg.reply_serial {
        fields.push(field(FIELD_REPLY_SERIAL, Value::UInt32(rs)));
    }
    if let Some(d) = &msg.destination {
        fields.push(field(FIELD_DESTINATION, Value::String(d.clone())));
    }
    if let Some(s) = &msg.sender {
        fields.push(field(FIELD_SENDER, Value::String(s.clone())));
    }
    if !msg.signature.is_empty() {
        fields.push(field(
            FIELD_SIGNATURE,
            Value::Signature(msg.signature.clone()),
        ));
    }

    let mut out = Vec::with_capacity(16 + body_bytes.len() + 64);
    out.push(b'l'); // little-endian
    out.push(msg.message_type);
    out.push(msg.flags);
    out.push(1); // protocol version
    out.extend_from_slice(&(body_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&msg.serial.to_le_bytes());
    marshal("a(yv)", &[Value::Array("(yv)".into(), fields)], &mut out)?;
    while out.len() % 8 != 0 {
        out.push(0);
    }
    out.extend_from_slice(&body_bytes);
    Ok(out)
}

fn bad(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, detail)
}

fn need_more_data() -> PlatformError {
    PlatformError::new(
        ErrorKind::WouldBlock,
        OsCode::None,
        "D-Bus message incomplete, more bytes needed",
    )
}

/// Parse one complete message from the front of `buf`, returning it
/// along with how many bytes it occupied. `Err` with `ErrorKind::WouldBlock`
/// means `buf` doesn't yet hold a complete message — callers read more
/// and retry, never a real failure.
pub fn decode(buf: &[u8]) -> Result<(Message, usize)> {
    if buf.len() < 16 {
        return Err(need_more_data());
    }
    if buf[0] != b'l' {
        return Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "big-endian D-Bus messages are not supported",
        ));
    }
    let message_type = buf[1];
    let flags = buf[2];
    let body_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let serial = u32::from_le_bytes(buf[8..12].try_into().unwrap());

    let mut offset = 12;
    let header_fields = match unmarshal("a(yv)", buf, &mut offset) {
        Ok(v) => v,
        Err(e) if e.kind == ErrorKind::InvalidInput => return Err(need_more_data()),
        Err(e) => return Err(e),
    };
    while offset % 8 != 0 {
        offset += 1;
    }
    let body_start = offset;
    if buf.len() < body_start + body_len {
        return Err(need_more_data());
    }

    let Value::Array(_, field_structs) = &header_fields[0] else {
        return Err(bad("header fields value was not the expected array"));
    };

    let mut msg = Message {
        message_type,
        flags,
        serial,
        ..Default::default()
    };
    for f in field_structs {
        let Value::Struct(parts) = f else {
            continue;
        };
        let (Some(Value::Byte(code)), Some(Value::Variant(val))) = (parts.first(), parts.get(1))
        else {
            continue;
        };
        match (*code, val.as_ref()) {
            (FIELD_PATH, Value::ObjectPath(p)) => msg.path = Some(p.clone()),
            (FIELD_INTERFACE, Value::String(s)) => msg.interface = Some(s.clone()),
            (FIELD_MEMBER, Value::String(s)) => msg.member = Some(s.clone()),
            (FIELD_ERROR_NAME, Value::String(s)) => msg.error_name = Some(s.clone()),
            (FIELD_REPLY_SERIAL, Value::UInt32(u)) => msg.reply_serial = Some(*u),
            (FIELD_DESTINATION, Value::String(s)) => msg.destination = Some(s.clone()),
            (FIELD_SENDER, Value::String(s)) => msg.sender = Some(s.clone()),
            (FIELD_SIGNATURE, Value::Signature(s)) => msg.signature = s.clone(),
            _ => {}
        }
    }

    msg.body = if msg.signature.is_empty() {
        Vec::new()
    } else {
        let mut body_offset = body_start;
        unmarshal(&msg.signature, buf, &mut body_offset)?
    };

    Ok((msg, body_start + body_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trips_a_method_call() {
        let msg = Message::method_call(
            7,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.Peer",
            "Ping",
            "",
            vec![],
        );
        let bytes = encode(&msg).unwrap();
        assert_eq!(bytes.len() % 8, 0, "a bodyless message must end 8-aligned");
        let (decoded, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded.message_type, TYPE_METHOD_CALL);
        assert_eq!(decoded.serial, 7);
        assert_eq!(decoded.path.as_deref(), Some("/org/freedesktop/DBus"));
        assert_eq!(
            decoded.interface.as_deref(),
            Some("org.freedesktop.DBus.Peer")
        );
        assert_eq!(decoded.member.as_deref(), Some("Ping"));
        assert_eq!(decoded.destination.as_deref(), Some("org.freedesktop.DBus"));
        assert!(decoded.body.is_empty());
    }

    #[test]
    fn encode_decode_round_trips_a_message_with_a_body() {
        let msg = Message::method_call(
            1,
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "OpenSession",
            "sv",
            vec![
                Value::String("plain".into()),
                Value::Variant(Box::new(Value::String("".into()))),
            ],
        );
        let bytes = encode(&msg).unwrap();
        let (decoded, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded.signature, "sv");
        assert_eq!(
            decoded.body,
            vec![
                Value::String("plain".into()),
                Value::Variant(Box::new(Value::String("".into()))),
            ]
        );
    }

    #[test]
    fn decode_reports_would_block_on_a_partial_message() {
        let msg = Message::method_call(
            1,
            "org.freedesktop.DBus",
            "/",
            "org.freedesktop.DBus.Peer",
            "Ping",
            "",
            vec![],
        );
        let bytes = encode(&msg).unwrap();
        // Feed only the fixed 16-byte header — nowhere near a full message.
        let e = decode(&bytes[..16]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::WouldBlock);

        // Feed everything except the last byte.
        let e = decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::WouldBlock);
    }

    #[test]
    fn decode_refuses_big_endian() {
        let mut bytes = encode(&Message::method_call(1, "d", "/", "i", "m", "", vec![])).unwrap();
        bytes[0] = b'B';
        let e = decode(&bytes).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
    }
}
