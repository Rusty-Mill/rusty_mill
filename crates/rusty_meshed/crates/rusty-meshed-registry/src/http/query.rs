//! Server-side query-string parsing and percent-decoding.
//!
//! `rusty_http::url` has [`rusty_http::url::percent_encode`] (client
//! side, for building a query string) but no decoding counterpart --
//! this module is the missing inverse, scoped to exactly what parsing
//! an incoming request's query string needs.

/// Parses a `key=value&key2=value2` query string (the part after `?`,
/// not including it) into decoded `(key, value)` pairs, in the order
/// they appeared. A pair with no `=` is treated as a key with an empty
/// value, matching how most web frameworks (FastAPI/Starlette
/// included) parse `?flag` with no value.
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (percent_decode(key), percent_decode(value)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// Decodes `%XX` percent-escapes back to raw bytes, then lossily to
/// UTF-8. The exact inverse of [`rusty_http::url::percent_encode`]
/// (which escapes everything outside `A-Za-z0-9-_.~`), so round-trips
/// with this crate's own HTTP client cleanly. A malformed `%` escape
/// (not followed by two hex digits) is passed through literally rather
/// than rejected -- there's no error channel a query-string parser
/// needs for "the client sent a slightly malformed query string".
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Splits a request target (`RequestHead::target`, origin-form e.g.
/// `/a/b?x=1`) into its path and parsed query pairs.
pub fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target.to_string(), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_pairs_in_order() {
        assert_eq!(
            parse_query("name=orders&domain=commerce"),
            vec![
                ("name".to_string(), "orders".to_string()),
                ("domain".to_string(), "commerce".to_string()),
            ]
        );
    }

    #[test]
    fn empty_query_string_is_no_pairs() {
        assert_eq!(parse_query(""), Vec::<(String, String)>::new());
    }

    #[test]
    fn a_key_with_no_equals_gets_an_empty_value() {
        assert_eq!(
            parse_query("flag"),
            vec![("flag".to_string(), String::new())]
        );
    }

    #[test]
    fn percent_decode_inverts_percent_encode() {
        let encoded = rusty_http::url::percent_encode("team a/b");
        assert_eq!(percent_decode(&encoded), "team a/b");
    }

    #[test]
    fn percent_decode_passes_through_a_malformed_escape() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("100%zz"), "100%zz");
    }

    #[test]
    fn split_target_separates_path_and_query() {
        let (path, query) = split_target("/data-products?name=orders");
        assert_eq!(path, "/data-products");
        assert_eq!(query, vec![("name".to_string(), "orders".to_string())]);
    }

    #[test]
    fn split_target_with_no_query_string() {
        let (path, query) = split_target("/data-products");
        assert_eq!(path, "/data-products");
        assert!(query.is_empty());
    }
}
