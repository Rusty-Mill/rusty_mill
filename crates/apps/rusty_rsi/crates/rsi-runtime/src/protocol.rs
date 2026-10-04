//! The broker wire protocol between the runtime and the inner agent
//! (ADR-0005 §3).
//!
//! The agent is std-only Rust with no JSON parser, so the format is plain
//! length-prefixed binary that takes a few lines to implement on each side.
//! `harness/src/broker.rs` is the agent's copy; a golden-bytes test on each
//! side pins the two together.
//!
//! ```text
//! frame    = len:u32be body            (len <= MAX_FRAME_BYTES)
//! body     = tag:u8 field*
//! field    = len:u32be utf8-bytes
//!
//! request  1 llm     role, content, role, content, ...   (at least one pair)
//!          2 eval    solution source
//!          3 submit  solution source
//! response 1 text       reply
//!          2 evaluated  score ("" when buggy), feedback
//!          3 submitted
//!          4 exhausted  reason      (budget hard stop; only submit still works)
//!          5 refused    reason      (a bad request; the session continues)
//! ```
//!
//! A transcript is the session's frames in order: request, response,
//! request, response, ...

use std::io::{ErrorKind, Read, Write};

use rsi_core::{Message, Role, Score};

use crate::error::RuntimeError;

/// The largest frame body either side accepts.
pub const MAX_FRAME_BYTES: usize = 8 << 20;

/// What the agent asks the broker for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Complete these messages with the inner model.
    Llm(Vec<Message>),
    /// Run this solution on the public split and score it.
    Eval(String),
    /// Make this solution the run's answer (the latest one wins).
    Submit(String),
}

/// The broker's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// The model's reply.
    Text(String),
    /// A public evaluation: the score, or `None` for a buggy solution.
    Evaluated {
        /// The public score.
        score: Option<Score>,
        /// The run's output, for the agent to read.
        feedback: String,
    },
    /// The submission was recorded.
    Submitted,
    /// The budget is spent; only `submit` still works.
    Exhausted(String),
    /// The request was invalid; the session continues.
    Refused(String),
}

/// One request and the response it got.
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    /// What the agent asked.
    pub request: Request,
    /// What the broker answered.
    pub response: Response,
}

impl Request {
    /// Encodes the request as a frame body.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Llm(messages) => {
                let fields = messages
                    .iter()
                    .flat_map(|m| [m.role.as_str(), m.content.as_str()]);
                body(1, fields)
            }
            Self::Eval(source) => body(2, [source.as_str()]),
            Self::Submit(source) => body(3, [source.as_str()]),
        }
    }

    /// Decodes a frame body.
    ///
    /// # Errors
    /// [`RuntimeError::Broker`] for an unknown tag or malformed fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let (tag, fields) = fields(bytes)?;
        match (tag, fields.as_slice()) {
            (1, pairs) if !pairs.is_empty() && pairs.len() % 2 == 0 => pairs
                .chunks(2)
                .map(|pair| {
                    let role = Role::parse(&pair[0])
                        .ok_or_else(|| malformed(format!("unknown role {:?}", pair[0])))?;
                    Ok(Message {
                        role,
                        content: pair[1].clone(),
                    })
                })
                .collect::<Result<_, _>>()
                .map(Self::Llm),
            (2, [source]) => Ok(Self::Eval(source.clone())),
            (3, [source]) => Ok(Self::Submit(source.clone())),
            _ => Err(malformed(format!(
                "request tag {tag} with {} fields",
                fields.len()
            ))),
        }
    }
}

impl Response {
    /// Encodes the response as a frame body.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Text(text) => body(1, [text.as_str()]),
            Self::Evaluated { score, feedback } => {
                let score = score.map(|s| s.get().to_string()).unwrap_or_default();
                body(2, [score.as_str(), feedback.as_str()])
            }
            Self::Submitted => body(3, []),
            Self::Exhausted(reason) => body(4, [reason.as_str()]),
            Self::Refused(reason) => body(5, [reason.as_str()]),
        }
    }

    /// Decodes a frame body.
    ///
    /// # Errors
    /// [`RuntimeError::Broker`] for an unknown tag, malformed fields or a
    /// score outside `[0, 1]`.
    pub fn decode(bytes: &[u8]) -> Result<Self, RuntimeError> {
        let (tag, fields) = fields(bytes)?;
        match (tag, fields.as_slice()) {
            (1, [text]) => Ok(Self::Text(text.clone())),
            (2, [score, feedback]) => {
                let score = match score.as_str() {
                    "" => None,
                    text => {
                        let value = text
                            .parse::<f64>()
                            .map_err(|_| malformed(format!("score {text:?}")))?;
                        Some(Score::new(value)?)
                    }
                };
                Ok(Self::Evaluated {
                    score,
                    feedback: feedback.clone(),
                })
            }
            (3, []) => Ok(Self::Submitted),
            (4, [reason]) => Ok(Self::Exhausted(reason.clone())),
            (5, [reason]) => Ok(Self::Refused(reason.clone())),
            _ => Err(malformed(format!(
                "response tag {tag} with {} fields",
                fields.len()
            ))),
        }
    }
}

fn malformed(what: String) -> RuntimeError {
    RuntimeError::Broker(format!("malformed frame: {what}"))
}

fn body<'a>(tag: u8, fields: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    let mut out = vec![tag];
    for field in fields {
        // Every field is far below 4 GiB: frames are capped at 8 MiB.
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out
}

fn fields(bytes: &[u8]) -> Result<(u8, Vec<String>), RuntimeError> {
    let (&tag, mut rest) = bytes
        .split_first()
        .ok_or_else(|| malformed("empty body".into()))?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (len, tail) = rest
            .split_first_chunk::<4>()
            .ok_or_else(|| malformed("truncated field length".into()))?;
        let len = u32::from_be_bytes(*len) as usize;
        if tail.len() < len {
            return Err(malformed("truncated field".into()));
        }
        let (field, tail) = tail.split_at(len);
        let text = std::str::from_utf8(field).map_err(|_| malformed("non-UTF-8 field".into()))?;
        out.push(text.to_owned());
        rest = tail;
    }
    Ok((tag, out))
}

/// Reads one frame body; `None` at a clean end of stream.
///
/// # Errors
/// [`RuntimeError::Broker`] for a frame over [`MAX_FRAME_BYTES`] or one cut
/// off mid-way; [`RuntimeError::Io`] when reading fails.
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<u8>>, RuntimeError> {
    let mut len = [0u8; 4];
    let mut filled = 0;
    while filled < len.len() {
        match reader.read(&mut len[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(malformed("stream ended inside a frame length".into())),
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(RuntimeError::io("reading a broker frame", e)),
        }
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(malformed(format!(
            "{len} bytes exceeds the {MAX_FRAME_BYTES}-byte limit"
        )));
    }
    let mut frame = vec![0u8; len];
    reader.read_exact(&mut frame).map_err(|e| match e.kind() {
        ErrorKind::UnexpectedEof => malformed("stream ended inside a frame".into()),
        _ => RuntimeError::io("reading a broker frame", e),
    })?;
    Ok(Some(frame))
}

/// Writes one frame.
///
/// # Errors
/// [`RuntimeError::Io`] when writing fails (for example, the agent died).
pub fn write_frame(writer: &mut impl Write, frame: &[u8]) -> Result<(), RuntimeError> {
    let len = u32::try_from(frame.len())
        .ok()
        .filter(|&len| len as usize <= MAX_FRAME_BYTES)
        .ok_or_else(|| malformed(format!("{} bytes is too large to send", frame.len())))?;
    writer
        .write_all(&len.to_be_bytes())
        .and_then(|()| writer.write_all(frame))
        .and_then(|()| writer.flush())
        .map_err(|e| RuntimeError::io("writing a broker frame", e))
}

/// Encodes a transcript: each exchange's request frame, then its response.
///
/// # Errors
/// [`RuntimeError::Broker`] if a frame exceeds [`MAX_FRAME_BYTES`].
pub fn encode_transcript(exchanges: &[Exchange]) -> Result<Vec<u8>, RuntimeError> {
    let mut out = Vec::new();
    for exchange in exchanges {
        write_frame(&mut out, &exchange.request.encode())?;
        write_frame(&mut out, &exchange.response.encode())?;
    }
    Ok(out)
}

/// Decodes a transcript produced by [`encode_transcript`].
///
/// # Errors
/// [`RuntimeError::Broker`] for a malformed or truncated transcript.
pub fn decode_transcript(mut bytes: &[u8]) -> Result<Vec<Exchange>, RuntimeError> {
    let mut out = Vec::new();
    while let Some(request) = read_frame(&mut bytes)? {
        let response = read_frame(&mut bytes)?
            .ok_or_else(|| malformed("transcript ends after a request".into()))?;
        out.push(Exchange {
            request: Request::decode(&request)?,
            response: Response::decode(&response)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchanges() -> Vec<Exchange> {
        let message = |role, content: &str| Message {
            role,
            content: content.to_owned(),
        };
        vec![
            Exchange {
                request: Request::Llm(vec![
                    message(Role::System, "be brief"),
                    message(Role::User, "solve ✓"),
                ]),
                response: Response::Text("```python\nprint(1)\n```".into()),
            },
            Exchange {
                request: Request::Eval("print(1)\n".into()),
                response: Response::Evaluated {
                    score: Some(Score::new(0.1 + 0.2).expect("valid")),
                    feedback: "[exited 0]\n".into(),
                },
            },
            Exchange {
                request: Request::Eval("raise SystemExit(1)\n".into()),
                response: Response::Evaluated {
                    score: None,
                    feedback: String::new(),
                },
            },
            Exchange {
                request: Request::Submit("print(1)\n".into()),
                response: Response::Submitted,
            },
            Exchange {
                request: Request::Eval("x".into()),
                response: Response::Exhausted("token budget exhausted".into()),
            },
            Exchange {
                request: Request::Submit(String::new()),
                response: Response::Refused("empty".into()),
            },
        ]
    }

    #[test]
    fn transcripts_round_trip_exactly() {
        let original = exchanges();
        let bytes = encode_transcript(&original).expect("encodes");
        assert_eq!(decode_transcript(&bytes).expect("decodes"), original);
    }

    /// The agent's copy of the protocol (`harness/src/broker.rs`) pins the
    /// same bytes; change both together.
    #[test]
    fn golden_bytes() {
        let request = Request::Llm(vec![Message {
            role: Role::User,
            content: "hi".into(),
        }]);
        assert_eq!(request.encode(), b"\x01\0\0\0\x04user\0\0\0\x02hi".to_vec());
        assert_eq!(Request::Eval("e".into()).encode(), b"\x02\0\0\0\x01e");
        assert_eq!(Request::Submit("s".into()).encode(), b"\x03\0\0\0\x01s");
        let evaluated = Response::Evaluated {
            score: Some(Score::new(0.5).expect("valid")),
            feedback: "ok".into(),
        };
        assert_eq!(evaluated.encode(), b"\x02\0\0\0\x030.5\0\0\0\x02ok");
        assert_eq!(Response::Submitted.encode(), b"\x03");
        let mut framed = Vec::new();
        write_frame(&mut framed, b"\x03").expect("writes");
        assert_eq!(framed, b"\0\0\0\x01\x03");
    }

    #[test]
    fn malformed_frames_are_rejected() {
        let bad: [&[u8]; 9] = [
            b"",
            b"\x09",
            b"\x01",
            b"\x01\0\0\0\x04user",
            b"\x01\0\0\0\x04tool\0\0\0\x02hi",
            b"\x02\0\0\0\x05ab",
            b"\x02\0\0",
            b"\x02\0\0\0\x01\xff",
            b"\x03\0\0\0\x01a\0\0\0\x01b",
        ];
        for bytes in bad {
            assert!(Request::decode(bytes).is_err(), "{bytes:?}");
        }
        let out_of_range = body(2, ["1.5", "fb"]);
        assert!(Response::decode(&out_of_range).is_err());
        assert!(Response::decode(&body(2, ["NaN", "fb"])).is_err());
        assert!(Response::decode(&body(3, ["extra"])).is_err());
    }

    #[test]
    fn frames_are_bounded_and_complete() {
        let mut empty: &[u8] = b"";
        assert!(read_frame(&mut empty).expect("clean end").is_none());
        let mut huge: &[u8] = &(MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        assert!(matches!(
            read_frame(&mut huge),
            Err(RuntimeError::Broker(_))
        ));
        let mut cut: &[u8] = b"\0\0\0\x05abc";
        assert!(matches!(read_frame(&mut cut), Err(RuntimeError::Broker(_))));
        let mut cut_len: &[u8] = b"\0\0";
        assert!(matches!(
            read_frame(&mut cut_len),
            Err(RuntimeError::Broker(_))
        ));
        let mut ok: &[u8] = b"\0\0\0\x02hi\0\0\0\x00";
        assert_eq!(read_frame(&mut ok).expect("frame"), Some(b"hi".to_vec()));
        assert_eq!(read_frame(&mut ok).expect("frame"), Some(Vec::new()));
        assert!(decode_transcript(b"\0\0\0\x01\x03").is_err(), "no response");
    }
}
