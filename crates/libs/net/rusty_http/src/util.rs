//! Shared byte-scanning helpers for the sans-IO parsers in [`crate::head`]
//! and [`crate::body`].

/// Finds the next `\n`-terminated line in `buf` (a trailing `\r` is
/// stripped, matching real servers that tolerate a bare `\n`). Returns
/// `None` if no line terminator has arrived yet -- the caller needs to
/// read more bytes before retrying.
pub(crate) fn next_line(buf: &[u8]) -> Option<(&[u8], usize)> {
    let pos = buf.iter().position(|&b| b == b'\n')?;
    let mut line = &buf[..pos];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    Some((line, pos + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_crlf_line() {
        let (line, consumed) = next_line(b"GET / HTTP/1.1\r\nrest").unwrap();
        assert_eq!(line, b"GET / HTTP/1.1");
        assert_eq!(consumed, 16);
    }

    #[test]
    fn splits_bare_lf_line() {
        let (line, consumed) = next_line(b"GET / HTTP/1.1\nrest").unwrap();
        assert_eq!(line, b"GET / HTTP/1.1");
        assert_eq!(consumed, 15);
    }

    #[test]
    fn none_when_no_terminator_yet() {
        assert!(next_line(b"still growing").is_none());
    }
}
