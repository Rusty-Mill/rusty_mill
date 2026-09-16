//! `rusty_stream`'s own wire protocol (ADR-0002 D1) — built on `rusty_wire`'s
//! `Reader`/`Writer` byte-cursor primitives rather than a redundant
//! hand-rolled one, and, per that same decision, not Kafka wire-protocol
//! compatible.
//!
//! Four request types — `Produce`/`Fetch` against the log, `Commit`/
//! `LastCommitted` against a consumer's offset — matching Phase 1's actual
//! storage surface (`crate::retention::Log`, `crate::consumer::
//! ConsumerOffsets`). `Commit`/`LastCommitted` are the write/read pair for
//! consumer offsets the same way `Produce`/`Fetch` are for the log itself —
//! `Commit` alone, with no way to read a commit back, wouldn't let a client
//! resume from where it left off, which is the entire point of committing.
//! Encoding is deliberately pure and synchronous, the same reason
//! `crate::record`'s framing has no I/O in it: testable without any runtime
//! at all.
//!
//! [`frame`]/[`frame_len`] exist because a real socket layer needs
//! length-prefixed framing to know how many bytes to read before a message
//! can even be decoded — this module only encodes/decodes bytes, it never
//! reads or writes a socket itself. See [`crate::server`] for the
//! `rusty_tokio` listener/connection loop built on top of these, and
//! [`crate::client`] for the client-side counterpart.

use rusty_wire::{Reader, Writer};

use crate::offset::Offset;

const OP_PRODUCE: u8 = 1;
const OP_FETCH: u8 = 2;
const OP_COMMIT: u8 = 3;
const OP_LAST_COMMITTED: u8 = 4;

const STATUS_PRODUCED: u8 = 1;
const STATUS_FETCHED: u8 = 2;
const STATUS_COMMITTED: u8 = 3;
const STATUS_LAST_COMMITTED: u8 = 4;
const STATUS_ERROR: u8 = 0xFF;

/// A request from a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Append `payload` to the log.
    Produce { payload: Vec<u8> },
    /// Read the record at `offset` back out.
    Fetch { offset: Offset },
    /// Record that `consumer_id` has processed up to and including
    /// `offset`.
    Commit { consumer_id: String, offset: Offset },
    /// Read `consumer_id`'s last-committed offset back out.
    LastCommitted { consumer_id: String },
}

/// A response to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The record was appended at this offset.
    Produced { offset: Offset },
    /// The requested record.
    Fetched { payload: Vec<u8> },
    /// The commit was recorded.
    Committed,
    /// The consumer's last-committed offset, or `None` if it's never
    /// committed anything — same as [`crate::consumer::ConsumerOffsets::
    /// last_committed`], not an error case.
    LastCommitted { offset: Option<Offset> },
    /// The request failed. Not tied to any particular request type — a
    /// caller matches this against whichever request it sent.
    Error { message: String },
}

/// Writes a length-prefixed (`u16` BE) UTF-8 string — the shape every
/// `consumer_id` on the wire uses.
fn write_str(w: &mut Writer, s: &str) {
    let bytes = s.as_bytes();
    w.write_u16_be(bytes.len() as u16);
    w.write_bytes(bytes);
}

/// Reads a length-prefixed (`u16` BE) UTF-8 string written by [`write_str`].
fn read_str(r: &mut Reader) -> Result<String, ProtocolError> {
    let len = r.read_u16_be()? as usize;
    let bytes = r.read_bytes(len)?;
    core::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|_| ProtocolError::InvalidUtf8)
}

/// Why a [`Request`]/[`Response`] failed to decode. Never panics on
/// malformed input — same "report as a typed error, not a crash" stance as
/// `crate::record::DecodeError`, since these bytes come off a network
/// socket once one exists, an even less trusted boundary than a local file.
#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// Ran out of bytes mid-message.
    Truncated,
    /// The first byte wasn't a recognized opcode (decoding a [`Request`])
    /// or status (decoding a [`Response`]).
    UnknownTag(u8),
    /// An [`Response::Error`] message's or a `consumer_id`'s bytes weren't
    /// valid UTF-8. Never applies to `Produce`/`Fetch` payloads — those are
    /// opaque bytes and don't need to be.
    InvalidUtf8,
}

impl From<rusty_wire::Error> for ProtocolError {
    fn from(_: rusty_wire::Error) -> Self {
        // rusty_wire::Error::InvalidValue is only ever produced by
        // Writer::patch_*, which this module never calls on the decode
        // path -- every rusty_wire error this module can actually observe
        // is a truncated buffer.
        ProtocolError::Truncated
    }
}

/// Encodes a request: `[opcode][body]`.
pub fn encode_request(req: &Request) -> Vec<u8> {
    let mut w = Writer::new();
    match req {
        Request::Produce { payload } => {
            w.write_u8(OP_PRODUCE);
            w.write_u32_be(payload.len() as u32);
            w.write_bytes(payload);
        }
        Request::Fetch { offset } => {
            w.write_u8(OP_FETCH);
            w.write_u64_be(offset.0);
        }
        Request::Commit {
            consumer_id,
            offset,
        } => {
            w.write_u8(OP_COMMIT);
            write_str(&mut w, consumer_id);
            w.write_u64_be(offset.0);
        }
        Request::LastCommitted { consumer_id } => {
            w.write_u8(OP_LAST_COMMITTED);
            write_str(&mut w, consumer_id);
        }
    }
    w.into_vec()
}

/// Decodes a request from exactly the bytes [`encode_request`] produced —
/// no framing/length prefix expected here (see [`frame`] for that layer).
pub fn decode_request(bytes: &[u8]) -> Result<Request, ProtocolError> {
    let mut r = Reader::new(bytes);
    match r.read_u8()? {
        OP_PRODUCE => {
            let len = r.read_u32_be()? as usize;
            let payload = r.read_bytes(len)?.to_vec();
            Ok(Request::Produce { payload })
        }
        OP_FETCH => {
            let offset = Offset(r.read_u64_be()?);
            Ok(Request::Fetch { offset })
        }
        OP_COMMIT => {
            let consumer_id = read_str(&mut r)?;
            let offset = Offset(r.read_u64_be()?);
            Ok(Request::Commit {
                consumer_id,
                offset,
            })
        }
        OP_LAST_COMMITTED => {
            let consumer_id = read_str(&mut r)?;
            Ok(Request::LastCommitted { consumer_id })
        }
        other => Err(ProtocolError::UnknownTag(other)),
    }
}

/// Encodes a response: `[status][body]`.
pub fn encode_response(resp: &Response) -> Vec<u8> {
    let mut w = Writer::new();
    match resp {
        Response::Produced { offset } => {
            w.write_u8(STATUS_PRODUCED);
            w.write_u64_be(offset.0);
        }
        Response::Fetched { payload } => {
            w.write_u8(STATUS_FETCHED);
            w.write_u32_be(payload.len() as u32);
            w.write_bytes(payload);
        }
        Response::Committed => {
            w.write_u8(STATUS_COMMITTED);
        }
        Response::LastCommitted { offset } => {
            w.write_u8(STATUS_LAST_COMMITTED);
            match offset {
                Some(offset) => {
                    w.write_u8(1);
                    w.write_u64_be(offset.0);
                }
                None => w.write_u8(0),
            }
        }
        Response::Error { message } => {
            w.write_u8(STATUS_ERROR);
            let bytes = message.as_bytes();
            w.write_u16_be(bytes.len() as u16);
            w.write_bytes(bytes);
        }
    }
    w.into_vec()
}

/// Decodes a response from exactly the bytes [`encode_response`] produced.
pub fn decode_response(bytes: &[u8]) -> Result<Response, ProtocolError> {
    let mut r = Reader::new(bytes);
    match r.read_u8()? {
        STATUS_PRODUCED => {
            let offset = Offset(r.read_u64_be()?);
            Ok(Response::Produced { offset })
        }
        STATUS_FETCHED => {
            let len = r.read_u32_be()? as usize;
            let payload = r.read_bytes(len)?.to_vec();
            Ok(Response::Fetched { payload })
        }
        STATUS_COMMITTED => Ok(Response::Committed),
        STATUS_LAST_COMMITTED => {
            let offset = match r.read_u8()? {
                0 => None,
                _ => Some(Offset(r.read_u64_be()?)),
            };
            Ok(Response::LastCommitted { offset })
        }
        STATUS_ERROR => {
            let len = r.read_u16_be()? as usize;
            let bytes = r.read_bytes(len)?;
            let message = core::str::from_utf8(bytes)
                .map_err(|_| ProtocolError::InvalidUtf8)?
                .to_string();
            Ok(Response::Error { message })
        }
        other => Err(ProtocolError::UnknownTag(other)),
    }
}

/// Prefixes an already-encoded [`Request`]/[`Response`] with a 4-byte
/// big-endian length — what actually goes on the wire once a socket exists.
/// Framing is independent of message content, which is why this takes
/// already-encoded bytes rather than a `Request`/`Response` directly.
pub fn frame(message: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(4 + message.len());
    w.write_u32_be(message.len() as u32);
    w.write_bytes(message);
    w.into_vec()
}

/// The one piece of framing a socket layer needs from this module: given
/// the first 4 bytes read off the wire, how many more bytes to buffer
/// before a full message is available to decode. Buffering partial reads
/// until that many bytes have actually arrived is the real socket layer's
/// job, not implemented here — see this module's top-level docs.
pub fn frame_len(header: [u8; 4]) -> u32 {
    u32::from_be_bytes(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produce_request_round_trips() {
        let req = Request::Produce {
            payload: b"hello".to_vec(),
        };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn fetch_request_round_trips() {
        let req = Request::Fetch { offset: Offset(42) };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn commit_request_round_trips() {
        let req = Request::Commit {
            consumer_id: "reader-a".to_string(),
            offset: Offset(9),
        };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn last_committed_request_round_trips() {
        let req = Request::LastCommitted {
            consumer_id: "reader-a".to_string(),
        };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn committed_response_round_trips() {
        let resp = Response::Committed;
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn last_committed_response_round_trips_with_a_value() {
        let resp = Response::LastCommitted {
            offset: Some(Offset(3)),
        };
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn last_committed_response_round_trips_with_no_value() {
        let resp = Response::LastCommitted { offset: None };
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn empty_consumer_id_round_trips() {
        let req = Request::LastCommitted {
            consumer_id: String::new(),
        };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn truncated_commit_request_is_reported_not_panicked() {
        let req = Request::Commit {
            consumer_id: "reader-a".to_string(),
            offset: Offset(1),
        };
        let mut encoded = encode_request(&req);
        encoded.truncate(encoded.len() - 3);
        assert_eq!(decode_request(&encoded), Err(ProtocolError::Truncated));
    }

    #[test]
    fn invalid_utf8_consumer_id_is_reported_not_panicked() {
        let mut encoded = encode_request(&Request::LastCommitted {
            consumer_id: "x".to_string(),
        });
        let last = encoded.len() - 1;
        encoded[last] = 0xFF;
        assert_eq!(decode_request(&encoded), Err(ProtocolError::InvalidUtf8));
    }

    #[test]
    fn produced_response_round_trips() {
        let resp = Response::Produced { offset: Offset(7) };
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn fetched_response_round_trips() {
        let resp = Response::Fetched {
            payload: b"the record".to_vec(),
        };
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn error_response_round_trips() {
        let resp = Response::Error {
            message: "offset out of range".to_string(),
        };
        let encoded = encode_response(&resp);
        assert_eq!(decode_response(&encoded).unwrap(), resp);
    }

    #[test]
    fn empty_produce_payload_round_trips() {
        let req = Request::Produce { payload: vec![] };
        let encoded = encode_request(&req);
        assert_eq!(decode_request(&encoded).unwrap(), req);
    }

    #[test]
    fn truncated_request_is_reported_not_panicked() {
        let req = Request::Fetch { offset: Offset(1) };
        let mut encoded = encode_request(&req);
        encoded.truncate(3); // opcode + partial offset
        assert_eq!(decode_request(&encoded), Err(ProtocolError::Truncated));
        assert_eq!(decode_request(&[]), Err(ProtocolError::Truncated));
    }

    #[test]
    fn produce_request_claiming_more_payload_than_present_is_truncated() {
        let req = Request::Produce {
            payload: b"hello".to_vec(),
        };
        let mut encoded = encode_request(&req);
        encoded.truncate(encoded.len() - 2); // header says 5 bytes, only 3 present
        assert_eq!(decode_request(&encoded), Err(ProtocolError::Truncated));
    }

    #[test]
    fn unknown_request_opcode_is_reported_not_panicked() {
        assert_eq!(
            decode_request(&[0xAB]),
            Err(ProtocolError::UnknownTag(0xAB))
        );
    }

    #[test]
    fn unknown_response_status_is_reported_not_panicked() {
        assert_eq!(
            decode_response(&[0x00]),
            Err(ProtocolError::UnknownTag(0x00))
        );
    }

    #[test]
    fn invalid_utf8_in_error_message_is_reported_not_panicked() {
        let mut encoded = encode_response(&Response::Error {
            message: "x".to_string(),
        });
        // Overwrite the one-byte message body with an invalid UTF-8 byte.
        let last = encoded.len() - 1;
        encoded[last] = 0xFF;
        assert_eq!(decode_response(&encoded), Err(ProtocolError::InvalidUtf8));
    }

    #[test]
    fn frame_round_trips_length() {
        let message = encode_request(&Request::Fetch { offset: Offset(9) });
        let framed = frame(&message);
        let header: [u8; 4] = framed[0..4].try_into().unwrap();
        let len = frame_len(header) as usize;
        assert_eq!(len, message.len());
        assert_eq!(&framed[4..4 + len], message.as_slice());
    }
}
