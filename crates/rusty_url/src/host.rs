//! The [WHATWG URL Standard host parser](https://url.spec.whatwg.org/#host-parsing).

use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::percent_encode::{percent_decode, utf8_percent_encode, CONTROLS};
use crate::ParseError;

/// A [`Host`]'s data without the borrowed/owned string, stored inline in a
/// [`crate::Url`] (whose `Domain` case is a byte range into the URL's own
/// serialization instead of a separate allocation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostInternal {
    None,
    Domain,
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

impl From<Host<String>> for HostInternal {
    fn from(host: Host<String>) -> Self {
        match host {
            Host::Domain(s) if s.is_empty() => HostInternal::None,
            Host::Domain(_) => HostInternal::Domain,
            Host::Ipv4(addr) => HostInternal::Ipv4(addr),
            Host::Ipv6(addr) => HostInternal::Ipv6(addr),
        }
    }
}

/// The host name of a URL: a domain, an IPv4 address, or an IPv6 address.
///
/// `S` is the string type used for [`Host::Domain`] — usually `String` (the
/// default) for an owned host, or `&str` when borrowing from a parsed
/// [`Url`](crate::Url).
#[derive(Debug, Clone, Eq, Ord, PartialOrd, Hash)]
pub enum Host<S = String> {
    /// A DNS domain name: '.'-separated labels. Non-ASCII labels have
    /// already been IDNA-processed to punycode (for domains of special-scheme
    /// URLs) or percent-encoded (for opaque hosts of non-special URLs).
    Domain(S),
    /// An IPv4 address.
    Ipv4(Ipv4Addr),
    /// An IPv6 address, serialized between `[` and `]` per
    /// [RFC 5952](https://tools.ietf.org/html/rfc5952).
    Ipv6(Ipv6Addr),
}

impl Host<&str> {
    /// Return a copy of `self` that owns an allocated `String` rather than
    /// borrowing from a `Url`.
    pub fn to_owned(&self) -> Host<String> {
        match *self {
            Host::Domain(domain) => Host::Domain(domain.to_owned()),
            Host::Ipv4(addr) => Host::Ipv4(addr),
            Host::Ipv6(addr) => Host::Ipv6(addr),
        }
    }
}

impl Host<String> {
    /// Parse a host: either an IPv6 address in `[...]` brackets, or a domain
    /// (subject to IDNA normalization) or IPv4 address.
    ///
    /// <https://url.spec.whatwg.org/#host-parsing>
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        parse_host(input)
    }

    /// Parse an *opaque* host: used for non-special-scheme URLs, where the
    /// host is not resolved as a domain — only a restricted character set is
    /// validated and the rest is percent-encoded verbatim.
    ///
    /// <https://url.spec.whatwg.org/#concept-opaque-host-parser>
    pub fn parse_opaque(input: &str) -> Result<Self, ParseError> {
        parse_opaque_host(input)
    }
}

fn parse_host(input: &str) -> Result<Host<String>, ParseError> {
    if let Some(inner) = input.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or(ParseError::InvalidIpv6Address)?;
        return parse_ipv6addr(inner).map(Host::Ipv6);
    }

    let decoded: Cow<'_, [u8]> = percent_decode(input.as_bytes());
    let decoded = match decoded {
        Cow::Borrowed(_) => Cow::Borrowed(input),
        Cow::Owned(bytes) => {
            Cow::Owned(String::from_utf8(bytes).map_err(|_| ParseError::InvalidDomainCharacter)?)
        }
    };

    let domain =
        idna::domain_to_ascii_cow(decoded.as_bytes(), idna::AsciiDenyList::URL)?.into_owned();

    if domain.is_empty() {
        return Err(ParseError::EmptyHost);
    }

    if ends_in_a_number(&domain) {
        parse_ipv4addr(&domain).map(Host::Ipv4)
    } else {
        Ok(Host::Domain(domain))
    }
}

fn parse_opaque_host(input: &str) -> Result<Host<String>, ParseError> {
    if let Some(inner) = input.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or(ParseError::InvalidIpv6Address)?;
        return parse_ipv6addr(inner).map(Host::Ipv6);
    }

    let is_invalid_host_char = |c: char| {
        matches!(
            c,
            '\0' | '\t'
                | '\n'
                | '\r'
                | ' '
                | '#'
                | '/'
                | ':'
                | '<'
                | '>'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '^'
                | '|'
        )
    };
    if input.contains(is_invalid_host_char) {
        return Err(ParseError::InvalidDomainCharacter);
    }

    Ok(Host::Domain(
        utf8_percent_encode(input, CONTROLS).into_owned(),
    ))
}

impl<S: AsRef<str>> fmt::Display for Host<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(domain) => f.write_str(domain.as_ref()),
            Host::Ipv4(addr) => write!(f, "{addr}"),
            Host::Ipv6(addr) => {
                f.write_str("[")?;
                write_ipv6(addr, f)?;
                f.write_str("]")
            }
        }
    }
}

impl<S, T> PartialEq<Host<T>> for Host<S>
where
    S: PartialEq<T>,
{
    fn eq(&self, other: &Host<T>) -> bool {
        match (self, other) {
            (Host::Domain(a), Host::Domain(b)) => a == b,
            (Host::Ipv4(a), Host::Ipv4(b)) => a == b,
            (Host::Ipv6(a), Host::Ipv6(b)) => a == b,
            _ => false,
        }
    }
}

fn write_ipv6(addr: &Ipv6Addr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let segments = addr.segments();
    let (compress_start, compress_end) = longest_zero_run(&segments);
    let mut i = 0isize;
    while i < 8 {
        if i == compress_start {
            f.write_str(":")?;
            if i == 0 {
                f.write_str(":")?;
            }
            if compress_end < 8 {
                i = compress_end;
            } else {
                break;
            }
        }
        write!(f, "{:x}", segments[i as usize])?;
        if i < 7 {
            f.write_str(":")?;
        }
        i += 1;
    }
    Ok(())
}

/// Longest run of zero segments (length >= 2), per the IPv6 serializer's
/// compression rule. Returns `(-1, -2)` (never matches) if there's none.
fn longest_zero_run(pieces: &[u16; 8]) -> (isize, isize) {
    let mut longest = -1isize;
    let mut longest_len = -1isize;
    let mut start = -1isize;
    for i in 0..=8isize {
        let is_zero = i < 8 && pieces[i as usize] == 0;
        if is_zero {
            if start < 0 {
                start = i;
            }
        } else if start >= 0 {
            let len = i - start;
            if len > longest_len {
                longest = start;
                longest_len = len;
            }
            start = -1;
        }
    }
    if longest_len < 2 {
        (-1, -2)
    } else {
        (longest, longest + longest_len)
    }
}

/// <https://url.spec.whatwg.org/#ends-in-a-number-checker>
fn ends_in_a_number(input: &str) -> bool {
    let mut parts = input.rsplit('.');
    let last = match parts.next() {
        Some("") => match parts.next() {
            Some(part) => part,
            None => return false,
        },
        Some(last) => last,
        None => return false,
    };
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_ok()
}

/// `Ok(None)` means the input is a syntactically valid number that overflows `u32`.
fn parse_ipv4_number(mut input: &str) -> Result<Option<u32>, ()> {
    if input.is_empty() {
        return Err(());
    }
    let radix = if input.starts_with("0x") || input.starts_with("0X") {
        input = &input[2..];
        16
    } else if input.len() >= 2 && input.starts_with('0') {
        input = &input[1..];
        8
    } else {
        10
    };
    if input.is_empty() {
        return Ok(Some(0));
    }
    let valid = match radix {
        8 => input.bytes().all(|b| (b'0'..=b'7').contains(&b)),
        10 => input.bytes().all(|b| b.is_ascii_digit()),
        16 => input.bytes().all(|b| b.is_ascii_hexdigit()),
        _ => unreachable!(),
    };
    if !valid {
        return Err(());
    }
    match u32::from_str_radix(input, radix) {
        Ok(n) => Ok(Some(n)),
        Err(_) => Ok(None),
    }
}

/// <https://url.spec.whatwg.org/#concept-ipv4-parser>
fn parse_ipv4addr(input: &str) -> Result<Ipv4Addr, ParseError> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.is_empty() || parts.len() > 4 {
        return Err(ParseError::InvalidIpv4Address);
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in parts {
        match parse_ipv4_number(part) {
            Ok(Some(n)) => numbers.push(n),
            _ => return Err(ParseError::InvalidIpv4Address),
        }
    }
    let mut ipv4 = numbers.pop().expect("non-empty list of numbers");
    if !numbers.is_empty() && ipv4 > u32::MAX >> (8 * numbers.len() as u32) {
        return Err(ParseError::InvalidIpv4Address);
    }
    if numbers.iter().any(|n| *n > 255) {
        return Err(ParseError::InvalidIpv4Address);
    }
    for (i, n) in numbers.iter().enumerate() {
        ipv4 += n << (8 * (3 - i as u32));
    }
    Ok(Ipv4Addr::from(ipv4))
}

/// <https://url.spec.whatwg.org/#concept-ipv6-parser>
fn parse_ipv6addr(input: &str) -> Result<Ipv6Addr, ParseError> {
    let input = input.as_bytes();
    let len = input.len();
    let mut is_ipv4 = false;
    let mut pieces = [0u16; 8];
    let mut piece_pointer = 0usize;
    let mut compress_pointer: Option<usize> = None;
    let mut i = 0usize;

    if len < 2 {
        return Err(ParseError::InvalidIpv6Address);
    }

    if input[0] == b':' {
        if input[1] != b':' {
            return Err(ParseError::InvalidIpv6Address);
        }
        i = 2;
        piece_pointer = 1;
        compress_pointer = Some(1);
    }

    while i < len {
        if piece_pointer == 8 {
            return Err(ParseError::InvalidIpv6Address);
        }
        if input[i] == b':' {
            if compress_pointer.is_some() {
                return Err(ParseError::InvalidIpv6Address);
            }
            i += 1;
            piece_pointer += 1;
            compress_pointer = Some(piece_pointer);
            continue;
        }
        let start = i;
        let end = cmp::min(len, start + 4);
        let mut value = 0u16;
        while i < end {
            match (input[i] as char).to_digit(16) {
                Some(digit) => {
                    value = value * 0x10 + digit as u16;
                    i += 1;
                }
                None => break,
            }
        }
        if i < len {
            match input[i] {
                b'.' => {
                    if i == start {
                        return Err(ParseError::InvalidIpv6Address);
                    }
                    i = start;
                    if piece_pointer > 6 {
                        return Err(ParseError::InvalidIpv6Address);
                    }
                    is_ipv4 = true;
                }
                b':' => {
                    i += 1;
                    if i == len {
                        return Err(ParseError::InvalidIpv6Address);
                    }
                }
                _ => return Err(ParseError::InvalidIpv6Address),
            }
        }
        if is_ipv4 {
            break;
        }
        pieces[piece_pointer] = value;
        piece_pointer += 1;
    }

    if is_ipv4 {
        if piece_pointer > 6 {
            return Err(ParseError::InvalidIpv6Address);
        }
        let mut numbers_seen = 0;
        while i < len {
            if numbers_seen > 0 {
                if numbers_seen < 4 && input[i] == b'.' {
                    i += 1;
                } else {
                    return Err(ParseError::InvalidIpv6Address);
                }
            }
            let mut piece: Option<u16> = None;
            while i < len {
                let digit = match input[i] {
                    c @ b'0'..=b'9' => c - b'0',
                    _ => break,
                };
                match piece {
                    None => piece = Some(digit as u16),
                    Some(0) => return Err(ParseError::InvalidIpv6Address),
                    Some(ref mut v) => {
                        *v = *v * 10 + digit as u16;
                        if *v > 255 {
                            return Err(ParseError::InvalidIpv6Address);
                        }
                    }
                }
                i += 1;
            }
            let Some(v) = piece else {
                return Err(ParseError::InvalidIpv6Address);
            };
            pieces[piece_pointer] = pieces[piece_pointer] * 0x100 + v;
            numbers_seen += 1;
            if numbers_seen == 2 || numbers_seen == 4 {
                piece_pointer += 1;
            }
        }
        if numbers_seen != 4 {
            return Err(ParseError::InvalidIpv6Address);
        }
    }

    if i < len {
        return Err(ParseError::InvalidIpv6Address);
    }

    match compress_pointer {
        Some(compress_pointer) => {
            let mut swaps = piece_pointer - compress_pointer;
            piece_pointer = 7;
            while swaps > 0 {
                pieces.swap(piece_pointer, compress_pointer + swaps - 1);
                swaps -= 1;
                piece_pointer -= 1;
            }
        }
        None if piece_pointer != 8 => return Err(ParseError::InvalidIpv6Address),
        None => {}
    }

    Ok(Ipv6Addr::new(
        pieces[0], pieces[1], pieces[2], pieces[3], pieces[4], pieces[5], pieces[6], pieces[7],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ascii_domain() {
        assert_eq!(
            Host::parse("Example.COM").unwrap(),
            Host::Domain("example.com".to_owned())
        );
    }

    #[test]
    fn parses_idna_domain_to_punycode() {
        assert_eq!(
            Host::parse("bücher.example").unwrap(),
            Host::Domain("xn--bcher-kva.example".to_owned())
        );
    }

    #[test]
    fn rejects_empty_host() {
        assert_eq!(Host::parse(""), Err(ParseError::EmptyHost));
    }

    #[test]
    fn parses_ipv4_variants() {
        assert_eq!(
            Host::parse("127.0.0.1").unwrap(),
            Host::<String>::Ipv4(Ipv4Addr::new(127, 0, 0, 1))
        );
        assert_eq!(
            Host::parse("0x7f.1").unwrap(),
            Host::<String>::Ipv4(Ipv4Addr::new(127, 0, 0, 1))
        );
        assert_eq!(
            Host::parse("0177.0.0.1").unwrap(),
            Host::<String>::Ipv4(Ipv4Addr::new(127, 0, 0, 1))
        );
    }

    #[test]
    fn rejects_invalid_ipv4_octet() {
        assert_eq!(
            Host::parse("256.0.0.1"),
            Err(ParseError::InvalidIpv4Address)
        );
    }

    #[test]
    fn parses_ipv6() {
        assert_eq!(
            Host::parse("[::1]").unwrap(),
            Host::<String>::Ipv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))
        );
        assert_eq!(
            Host::parse("[2001:db8::1]").unwrap(),
            Host::<String>::Ipv6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))
        );
    }

    #[test]
    fn parses_ipv6_with_embedded_ipv4() {
        assert_eq!(
            Host::parse("[::ffff:192.0.2.1]").unwrap(),
            Host::<String>::Ipv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xc000, 0x0201))
        );
    }

    #[test]
    fn rejects_unclosed_ipv6_bracket() {
        assert_eq!(Host::parse("[::1"), Err(ParseError::InvalidIpv6Address));
    }

    #[test]
    fn ipv6_display_compresses_longest_zero_run() {
        let host: Host<String> = Host::Ipv6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert_eq!(host.to_string(), "[2001:db8::1]");
    }

    #[test]
    fn ipv4_display() {
        let host: Host<String> = Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(host.to_string(), "127.0.0.1");
    }

    #[test]
    fn opaque_host_percent_encodes_controls_only() {
        // 0x7F (DEL) is in the CONTROLS percent-encode set but not in the
        // opaque-host forbidden-character list, unlike e.g. space.
        assert_eq!(
            Host::parse_opaque("a\u{7f}b").unwrap(),
            Host::Domain("a%7Fb".to_owned())
        );
        assert_eq!(
            Host::parse_opaque("EXAMPLE").unwrap(),
            Host::Domain("EXAMPLE".to_owned())
        );
    }

    #[test]
    fn opaque_host_rejects_forbidden_chars() {
        assert_eq!(
            Host::parse_opaque("a/b"),
            Err(ParseError::InvalidDomainCharacter)
        );
    }

    #[test]
    fn host_to_owned_and_partial_eq_cross_type() {
        let owned: Host<String> = Host::Domain("example.com".to_owned());
        let borrowed: Host<&str> = Host::Domain("example.com");
        assert_eq!(borrowed.to_owned(), owned);
    }
}
