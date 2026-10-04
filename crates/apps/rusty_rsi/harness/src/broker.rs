//! The agent's side of the broker protocol.
//!
//! The runtime's copy (`rsi-runtime/src/protocol.rs`) documents the format;
//! a golden-bytes test on each side keeps the two in step.
//!
//! ```text
//! frame = len:u32be body;  body = tag:u8 field*;  field = len:u32be utf8
//! ```

use std::io::{self, Read, Write};

/// The largest frame accepted from the broker.
const MAX_FRAME_BYTES: usize = 8 << 20;

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// `system`, `user` or `assistant`.
    pub role: &'static str,
    /// The text.
    pub content: String,
}

/// The broker's answer to a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// The model's reply.
    Text(String),
    /// A public evaluation: the score, or `None` for a buggy solution.
    Evaluated {
        /// The public score in `[0, 1]`.
        score: Option<f64>,
        /// The solution's output.
        feedback: String,
    },
    /// The submission was recorded.
    Submitted,
    /// The budget is spent; only `submit` still works.
    Exhausted(String),
    /// The request was invalid.
    Refused(String),
}

/// What the agent can ask of the broker.
pub trait Broker {
    /// Completes `messages` with the inner model.
    fn llm(&mut self, messages: &[Message]) -> io::Result<Reply>;
    /// Runs `source` on the public split and scores it.
    fn eval(&mut self, source: &str) -> io::Result<Reply>;
    /// Makes `source` the run's answer; the latest submission wins.
    fn submit(&mut self, source: &str) -> io::Result<Reply>;
}

/// The broker over a byte stream (the socket on standard input).
#[derive(Debug)]
pub struct Wire<S>(S);

// Off Unix there is no broker socket, so nothing constructs a `Wire`.
#[cfg_attr(not(unix), allow(dead_code))]
impl<S> Wire<S> {
    /// Speaks the protocol over `stream`.
    pub const fn new(stream: S) -> Self {
        Self(stream)
    }
}

impl<S: Read + Write> Wire<S> {
    fn call(&mut self, request: &[u8]) -> io::Result<Reply> {
        let len = u32::try_from(request.len())
            .ok()
            .filter(|&len| len as usize <= MAX_FRAME_BYTES)
            .ok_or_else(|| invalid("request too large"))?;
        self.0.write_all(&len.to_be_bytes())?;
        self.0.write_all(request)?;
        self.0.flush()?;
        let mut len = [0u8; 4];
        self.0.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(invalid("response too large"));
        }
        let mut frame = vec![0u8; len];
        self.0.read_exact(&mut frame)?;
        decode_reply(&frame)
    }
}

impl<S: Read + Write> Broker for Wire<S> {
    fn llm(&mut self, messages: &[Message]) -> io::Result<Reply> {
        let fields = messages.iter().flat_map(|m| [m.role, m.content.as_str()]);
        self.call(&body(1, fields))
    }

    fn eval(&mut self, source: &str) -> io::Result<Reply> {
        self.call(&body(2, [source]))
    }

    fn submit(&mut self, source: &str) -> io::Result<Reply> {
        self.call(&body(3, [source]))
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn body<'a>(tag: u8, fields: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    let mut out = vec![tag];
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out
}

fn decode_reply(frame: &[u8]) -> io::Result<Reply> {
    let (&tag, mut rest) = frame.split_first().ok_or_else(|| invalid("empty frame"))?;
    let mut fields = Vec::new();
    while !rest.is_empty() {
        if rest.len() < 4 {
            return Err(invalid("truncated field"));
        }
        let (len, tail) = rest.split_at(4);
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        if tail.len() < len {
            return Err(invalid("truncated field"));
        }
        let (field, tail) = tail.split_at(len);
        let text = String::from_utf8(field.to_vec()).map_err(|_| invalid("non-UTF-8 field"))?;
        fields.push(text);
        rest = tail;
    }
    let mut fields = fields.into_iter();
    let reply = match (tag, fields.next(), fields.next()) {
        (1, Some(text), None) => Reply::Text(text),
        (2, Some(score), Some(feedback)) => Reply::Evaluated {
            score: match score.as_str() {
                "" => None,
                text => Some(text.parse().map_err(|_| invalid("bad score"))?),
            },
            feedback,
        },
        (3, None, None) => Reply::Submitted,
        (4, Some(reason), None) => Reply::Exhausted(reason),
        (5, Some(reason), None) => Reply::Refused(reason),
        _ => return Err(invalid("unknown reply")),
    };
    if fields.next().is_some() {
        return Err(invalid("extra fields"));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same bytes `rsi-runtime`'s `protocol::tests::golden_bytes` pins.
    #[test]
    fn golden_bytes() {
        let hi = [Message {
            role: "user",
            content: "hi".to_owned(),
        }];
        let fields = hi.iter().flat_map(|m| [m.role, m.content.as_str()]);
        assert_eq!(body(1, fields), b"\x01\0\0\0\x04user\0\0\0\x02hi");
        assert_eq!(body(2, ["e"]), b"\x02\0\0\0\x01e");
        assert_eq!(body(3, ["s"]), b"\x03\0\0\0\x01s");
        assert_eq!(
            decode_reply(b"\x02\0\0\0\x030.5\0\0\0\x02ok").ok(),
            Some(Reply::Evaluated {
                score: Some(0.5),
                feedback: "ok".to_owned()
            })
        );
        assert_eq!(decode_reply(b"\x03").ok(), Some(Reply::Submitted));
    }

    #[test]
    fn decodes_every_reply_and_rejects_junk() {
        assert_eq!(
            decode_reply(b"\x02\0\0\0\0\0\0\0\x01x").ok(),
            Some(Reply::Evaluated {
                score: None,
                feedback: "x".to_owned()
            })
        );
        assert_eq!(
            decode_reply(b"\x04\0\0\0\x01r").ok(),
            Some(Reply::Exhausted("r".to_owned()))
        );
        for junk in [
            &b""[..],
            b"\x09",
            b"\x01",
            b"\x01\0\0\0\x05ab",
            b"\x03\0\0\0\0",
        ] {
            assert!(decode_reply(junk).is_err(), "{junk:?}");
        }
    }

    #[test]
    fn a_call_writes_one_frame_and_reads_one_back() {
        struct Duplex {
            sent: Vec<u8>,
            reply: io::Cursor<Vec<u8>>,
        }
        impl Read for Duplex {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.reply.read(buf)
            }
        }
        impl Write for Duplex {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.sent.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut wire = Wire::new(Duplex {
            sent: Vec::new(),
            reply: io::Cursor::new(b"\0\0\0\x01\x03".to_vec()),
        });
        assert_eq!(wire.submit("s").ok(), Some(Reply::Submitted));
        assert_eq!(wire.0.sent, b"\0\0\0\x06\x03\0\0\0\x01s");
        assert!(wire.submit("s").is_err(), "the broker hung up");
    }
}
