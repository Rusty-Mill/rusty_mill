//! Opaque list cursors. A cursor is base64 of `1.<tag>.<offset>`: versioned
//! so a cursor held across a deploy is refused instead of reinterpreted, and
//! tagged so one issued for tools cannot seek into another list. The tool
//! set is fixed once a server is built, so an offset is stable.

use rusty_base64::{decode_url_safe, encode_url_safe_no_pad};

/// Which list a cursor belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Tools,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Tools => "tools",
        }
    }
}

pub(crate) fn encode(kind: Kind, offset: usize) -> String {
    encode_url_safe_no_pad(format!("1.{}.{offset}", kind.tag()).as_bytes())
}

/// The offset a cursor names, or `None` when it is not one of ours for
/// `kind` (garbage, wrong list, wrong version, or past `len`).
pub(crate) fn decode(kind: Kind, cursor: &str, len: usize) -> Option<usize> {
    let bytes = decode_url_safe(cursor).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let rest = text
        .strip_prefix("1.")?
        .strip_prefix(kind.tag())?
        .strip_prefix('.')?;
    let offset: usize = rest.parse().ok()?;
    (offset <= len).then_some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_round_trips() {
        for offset in [0, 1, 99, 12345] {
            let c = encode(Kind::Tools, offset);
            assert_eq!(decode(Kind::Tools, &c, 20000), Some(offset));
        }
    }

    #[test]
    fn foreign_stale_and_garbage_cursors_are_refused() {
        let good = encode(Kind::Tools, 3);
        assert_eq!(decode(Kind::Tools, &good, 2), None, "past the end");
        assert_eq!(
            decode(Kind::Tools, &good, 3),
            Some(3),
            "the end itself is fine"
        );
        for bad in [
            "",
            "!!!",
            &encode_url_safe_no_pad(b"2.tools.1"),
            &encode_url_safe_no_pad(b"1.prompts.1"),
            &encode_url_safe_no_pad(b"1.tools."),
            &encode_url_safe_no_pad(b"1.tools.-1"),
            &encode_url_safe_no_pad(b"1.tools.1x"),
            &encode_url_safe_no_pad(&[0xff, 0xfe]),
        ] {
            assert_eq!(decode(Kind::Tools, bad, 100), None, "{bad:?}");
        }
    }
}
