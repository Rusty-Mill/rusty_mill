//! The HPACK static table (RFC 7541 Appendix A). Indices are 1-based and
//! occupy the low end of the combined index space; the dynamic table
//! continues from `STATIC_TABLE.len() + 1`.

pub const STATIC_TABLE: [(&str, &str); 61] = [
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

/// Look up a static-table entry by its 1-based index. Returns `None` if
/// `index` is 0 or beyond the table.
pub fn get(index: usize) -> Option<(&'static str, &'static str)> {
    if index == 0 {
        return None;
    }
    STATIC_TABLE.get(index - 1).copied()
}

/// Find the first static-table entry with a matching name, and, if any
/// entry also matches the value, prefer that one. Used by the encoder to
/// choose between literal-with-indexed-name and fully-indexed representations.
pub fn find(name: &[u8], value: &[u8]) -> Option<(usize, bool)> {
    let mut name_only: Option<usize> = None;
    for (i, (n, v)) in STATIC_TABLE.iter().enumerate() {
        if n.as_bytes() == name {
            if v.as_bytes() == value {
                return Some((i + 1, true));
            }
            if name_only.is_none() {
                name_only = Some(i + 1);
            }
        }
    }
    name_only.map(|idx| (idx, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_one_based() {
        assert_eq!(get(1), Some((":authority", "")));
        assert_eq!(get(61), Some(("www-authenticate", "")));
        assert_eq!(get(62), None);
        assert_eq!(get(0), None);
    }

    #[test]
    fn find_exact_match() {
        assert_eq!(find(b":method", b"GET"), Some((2, true)));
        assert_eq!(find(b":method", b"PATCH"), Some((2, false)));
        assert_eq!(find(b"x-not-here", b""), None);
    }
}
