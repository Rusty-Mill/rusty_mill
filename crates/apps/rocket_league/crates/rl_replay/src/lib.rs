//! A first-party Rocket League `.replay` parser, with no external
//! dependencies (ADR-0002 Tier S).
//!
//! This is slice P1 of `rleval/docs/replay-parser-scope.md`: the container,
//! both section CRCs and the header. [`parse`] checks the framing and CRCs,
//! decodes the header and hands back the content section's bytes for the
//! network decoder that later slices add. `boxcars` is a test oracle only:
//! every committed sample's header must equal what it decodes.
//!
//! The format is described in `rleval/docs/replay-file-format.md`. Errors fail
//! closed and name what was being read and the byte offset.
//!
//! ```no_run
//! let data = std::fs::read("match.replay").unwrap();
//! let replay = rl_replay::parse(&data).unwrap();
//! println!("{:?}", replay.header.get("MapName"));
//! ```

mod crc;
mod error;
mod header;
mod reader;

pub use crc::crc;
pub use error::Error;
pub use header::{Header, Properties, Property, MAX_PROPERTY_DEPTH, MIN_VERSION};

use reader::Reader;

/// A parsed replay: the decoded header and the still-encoded content section.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay<'a> {
    /// Header section length in bytes, after its CRC word.
    pub header_size: u32,
    /// The header section's stored (and verified) CRC.
    pub header_crc: u32,
    /// The decoded header.
    pub header: Header,
    /// Content section length in bytes, after its CRC word.
    pub content_size: u32,
    /// The content section's stored (and verified) CRC.
    pub content_crc: u32,
    /// The content section's bytes: tables, keyframes and the network stream.
    pub content: &'a [u8],
}

/// Parse a replay file's container and header.
///
/// Both CRCs are always checked, so a corrupt file is reported as such rather
/// than as a confusing decode error further in. Array and struct nesting share
/// a limit of [`MAX_PROPERTY_DEPTH`] levels; deeper headers return
/// [`Error::NestingLimit`].
pub fn parse(data: &[u8]) -> Result<Replay<'_>, Error> {
    let mut r = Reader::new(data, 0);
    let (header_size, header_crc, header_offset, header_data) = section(&mut r, "header")?;
    let header = header::parse(&mut Reader::new(header_data, header_offset))?;
    let (content_size, content_crc, _, content) = section(&mut r, "content")?;
    Ok(Replay {
        header_size,
        header_crc,
        header,
        content_size,
        content_crc,
        content,
    })
}

/// One `size, crc, data` section, with its CRC verified. Returns the size, the
/// CRC, the data's absolute offset and the data.
fn section<'a>(
    r: &mut Reader<'a>,
    name: &'static str,
) -> Result<(u32, u32, usize, &'a [u8]), Error> {
    let size = r.u32("section size")?;
    let stored = r.u32("section crc")?;
    let offset = r.offset();
    let data = r.take(size as usize, name)?;
    let actual = crc(data);
    if actual != stored {
        return Err(Error::Crc {
            section: name,
            stored,
            actual,
        });
    }
    Ok((size, stored, offset, data))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn minimal_header() -> Vec<u8> {
        let mut h = Vec::new();
        for v in [868i32, 27, 10] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        for s in ["TAGame.Replay_Soccar_TA", "None"] {
            h.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
            h.extend_from_slice(s.as_bytes());
            h.push(0);
        }
        h
    }

    fn file(header: &[u8], content: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        for section in [header, content] {
            f.extend_from_slice(&(section.len() as u32).to_le_bytes());
            f.extend_from_slice(&crc(section).to_le_bytes());
            f.extend_from_slice(section);
        }
        f
    }

    #[test]
    fn parses_a_minimal_file() {
        let data = file(&minimal_header(), b"body");
        let replay = parse(&data).unwrap();
        assert_eq!(replay.header.engine_version, 868);
        assert!(replay.header.properties.is_empty());
        assert_eq!(replay.content, b"body");
    }

    #[test]
    fn a_flipped_content_byte_is_a_crc_error() {
        let mut data = file(&minimal_header(), b"body");
        let last = data.len() - 1;
        data[last] ^= 1;
        assert!(matches!(
            parse(&data),
            Err(Error::Crc {
                section: "content",
                ..
            })
        ));
    }

    #[test]
    fn a_short_file_is_truncated_not_a_panic() {
        let data = file(&minimal_header(), b"body");
        for cut in [0, 3, 8, 20, data.len() - 1] {
            assert!(
                matches!(parse(&data[..cut]), Err(Error::Truncated { .. })),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn header_errors_carry_absolute_offsets() {
        let mut header = minimal_header();
        header.truncate(header.len() - 9); // drop the "None" key
        let err = parse(&file(&header, b"")).unwrap_err();
        let Error::Truncated { offset, .. } = err else {
            panic!("{err}");
        };
        assert_eq!(offset, 8 + header.len());
    }
}
