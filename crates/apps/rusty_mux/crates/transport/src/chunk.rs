//! The chunks a datagram's plaintext is made of.

use rusty_wire::{Reader, Writer};

/// Byte ranges `[start, end)` beyond an ack's cumulative offset that have
/// arrived; at most [`MAX_RANGES`] go in one ack.
pub(crate) type Ranges = Vec<(u64, u64)>;

/// The most received ranges one [`Chunk::Ack`] reports.
pub(crate) const MAX_RANGES: usize = 4;

/// One chunk of a datagram's plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Chunk {
    /// Bytes of `stream` starting at `offset`.
    Data {
        stream: u64,
        offset: u64,
        bytes: Vec<u8>,
    },
    /// Everything of `stream` below `cumulative` has arrived, as have
    /// `ranges`; the receiver takes bytes up to `limit`.
    Ack {
        stream: u64,
        cumulative: u64,
        limit: u64,
        ranges: Ranges,
    },
    /// The sender's clock, in ms since its association began.
    Stamp(u64),
    /// The newest [`Chunk::Stamp`] heard, and the ms it was held since.
    Echo { stamp: u64, held: u64 },
    /// Asks for an answer and carries nothing else.
    Probe,
    /// `stream` is closed; `reply` confirms the peer's own close.
    Close { stream: u64, reply: bool },
    /// The whole association is closed.
    End,
}

const DATA: u8 = 1;
const ACK: u8 = 2;
const STAMP: u8 = 3;
const ECHO: u8 = 4;
const PROBE: u8 = 5;
const CLOSE: u8 = 6;
const CLOSE_REPLY: u8 = 7;
const END: u8 = 8;

/// The most bytes a [`Chunk::Data`] header takes: a tag and three varints.
pub(crate) const DATA_HEADER: usize = 1 + 3 * 10;

impl Chunk {
    /// Whether receiving this asks the peer for an answer soon.
    pub(crate) fn eliciting(&self) -> bool {
        matches!(
            self,
            Chunk::Data { .. } | Chunk::Probe | Chunk::Close { reply: false, .. }
        )
    }

    pub(crate) fn encode(&self, w: &mut Writer) {
        match self {
            Chunk::Data {
                stream,
                offset,
                bytes,
            } => {
                w.write_u8(DATA);
                varints(w, &[*stream, *offset, bytes.len() as u64]);
                w.write_bytes(bytes);
            }
            Chunk::Ack {
                stream,
                cumulative,
                limit,
                ranges,
            } => {
                w.write_u8(ACK);
                varints(w, &[*stream, *cumulative, *limit, ranges.len() as u64]);
                for (start, end) in ranges {
                    varints(w, &[*start, *end]);
                }
            }
            Chunk::Stamp(ms) => {
                w.write_u8(STAMP);
                varint(w, *ms);
            }
            Chunk::Echo { stamp, held } => {
                w.write_u8(ECHO);
                varints(w, &[*stamp, *held]);
            }
            Chunk::Probe => w.write_u8(PROBE),
            Chunk::Close { stream, reply } => {
                w.write_u8(if *reply { CLOSE_REPLY } else { CLOSE });
                varint(w, *stream);
            }
            Chunk::End => w.write_u8(END),
        }
    }

    /// Every chunk in `plain`, or `None` if any is malformed.
    pub(crate) fn decode_all(plain: &[u8]) -> Option<Vec<Chunk>> {
        let mut r = Reader::new(plain);
        let mut chunks = Vec::new();
        while !r.is_empty() {
            chunks.push(Self::decode(&mut r)?);
        }
        Some(chunks)
    }

    fn decode(r: &mut Reader<'_>) -> Option<Chunk> {
        let chunk = match r.read_u8().ok()? {
            DATA => {
                let (stream, offset) = (read(r)?, read(r)?);
                let len = usize::try_from(read(r)?).ok()?;
                let bytes = r.read_bytes(len).ok()?.to_vec();
                offset.checked_add(len as u64)?;
                Chunk::Data {
                    stream,
                    offset,
                    bytes,
                }
            }
            ACK => {
                let (stream, cumulative, limit) = (read(r)?, read(r)?, read(r)?);
                let n = usize::try_from(read(r)?).ok()?;
                if n > MAX_RANGES {
                    return None;
                }
                let ranges = (0..n)
                    .map(|_| Some((read(r)?, read(r)?)))
                    .collect::<Option<Ranges>>()?;
                Chunk::Ack {
                    stream,
                    cumulative,
                    limit,
                    ranges,
                }
            }
            STAMP => Chunk::Stamp(read(r)?),
            ECHO => Chunk::Echo {
                stamp: read(r)?,
                held: read(r)?,
            },
            PROBE => Chunk::Probe,
            CLOSE => Chunk::Close {
                stream: read(r)?,
                reply: false,
            },
            CLOSE_REPLY => Chunk::Close {
                stream: read(r)?,
                reply: true,
            },
            END => Chunk::End,
            _ => return None,
        };
        Some(chunk)
    }
}

fn varints(w: &mut Writer, values: &[u64]) {
    for value in values {
        varint(w, *value);
    }
}

/// Unsigned LEB128.
fn varint(w: &mut Writer, mut value: u64) {
    while value >= 0x80 {
        w.write_u8(value as u8 | 0x80);
        value >>= 7;
    }
    w.write_u8(value as u8);
}

fn read(r: &mut Reader<'_>) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = r.read_u8().ok()?;
        let bits = u64::from(byte & 0x7f);
        // The tenth byte may only carry the top bit of a u64.
        if shift == 63 && bits > 1 {
            return None;
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chunk_round_trips() {
        let chunks = vec![
            Chunk::Data {
                stream: 3,
                offset: u64::MAX - 5,
                bytes: b"hello".to_vec(),
            },
            Chunk::Ack {
                stream: 0,
                cumulative: 300,
                limit: 1 << 20,
                ranges: vec![(400, 500), (600, 601)],
            },
            Chunk::Stamp(123_456),
            Chunk::Echo {
                stamp: 99,
                held: 20,
            },
            Chunk::Probe,
            Chunk::Close {
                stream: 9,
                reply: false,
            },
            Chunk::Close {
                stream: 9,
                reply: true,
            },
            Chunk::End,
        ];
        let mut w = Writer::new();
        for chunk in &chunks {
            chunk.encode(&mut w);
        }
        assert_eq!(Chunk::decode_all(w.as_slice()), Some(chunks));
    }

    #[test]
    fn malformed_plaintext_is_refused_whole() {
        let mut w = Writer::new();
        Chunk::Probe.encode(&mut w);
        let bytes = w.into_vec();
        for bad in [vec![0x7f], vec![DATA, 0, 0, 5, b'x'], vec![ACK, 0, 0, 0, 5]] {
            let mut plain = bytes.clone();
            plain.extend(bad);
            assert_eq!(Chunk::decode_all(&plain), None, "{plain:?}");
        }
        // Data whose end would pass u64::MAX.
        let mut w = Writer::new();
        Chunk::Data {
            stream: 0,
            offset: u64::MAX,
            bytes: b"x".to_vec(),
        }
        .encode(&mut w);
        assert_eq!(Chunk::decode_all(w.as_slice()), None);
    }
}
