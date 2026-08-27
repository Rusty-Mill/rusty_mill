//! Foundational LSP value types shared across requests.

use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Borrow;
use std::path::{Path, PathBuf};

/// A document URI, e.g. `file:///home/user/main.rs`.
///
/// LSP transmits URIs as opaque strings, but comparing them as raw strings is
/// hazardous: `file:///A%2Fb` and `file:///a%2fb` differ byte-wise while two
/// clients may emit either form. `Uri` is a lightweight newtype (no parsing
/// dependency) that **normalizes on construction** — the scheme is lowercased
/// and percent-encoding hex digits are uppercased — so equality, hashing, and
/// map lookups (e.g. in [`crate::Documents`]) agree across those spellings.
/// Path case is left untouched, since it is significant on most filesystems.
///
/// Construct one with [`Uri::new`] (or `From<&str>` / `From<String>`), and
/// convert to/from filesystem paths with [`Uri::from_file_path`] /
/// [`Uri::to_file_path`]. `Uri` dereferences to `str`, so string inspection
/// (`starts_with`, `split`, …) works directly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Uri(String);

impl Uri {
    /// Build a URI from a string, normalizing the scheme to lowercase and
    /// percent-encoding hex digits to uppercase.
    pub fn new(uri: impl Into<String>) -> Self {
        Uri(normalize_uri(uri.into()))
    }

    /// The URI as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the URI, returning the underlying string.
    pub fn into_string(self) -> String {
        self.0
    }

    /// The URI's scheme (e.g. `"file"`), if it has a syntactically valid one.
    pub fn scheme(&self) -> Option<&str> {
        let (scheme, _) = self.0.split_once(':')?;
        is_valid_scheme(scheme).then_some(scheme)
    }

    /// Build a `file://` URI from an absolute filesystem path,
    /// percent-encoding characters as needed. Returns `None` for relative or
    /// non-UTF-8 paths.
    pub fn from_file_path(path: impl AsRef<Path>) -> Option<Uri> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return None;
        }
        let path = path.to_str()?;
        let mut out = String::with_capacity(path.len() + 8);
        out.push_str("file://");
        #[cfg(windows)]
        {
            out.push('/');
            for ch in path.chars() {
                match ch {
                    '\\' => out.push('/'),
                    _ => encode_path_char(&mut out, ch),
                }
            }
        }
        #[cfg(not(windows))]
        for ch in path.chars() {
            encode_path_char(&mut out, ch);
        }
        Some(Uri(out))
    }

    /// The URI of the containing "directory": everything up to the last
    /// path segment. The root keeps its trailing slash
    /// (`file:///a/b` → `file:///a` → `file:///`); returns `None` when
    /// already at the root or when the URI has no path.
    pub fn parent(&self) -> Option<Uri> {
        let path_start = self.path_start()?;
        let last = self.0.rfind('/')?;
        if last < path_start || self.0.len() == path_start + 1 {
            return None; // no path segments, or already the bare root
        }
        if last == path_start {
            // Direct child of the root: parent is the root itself.
            return Some(Uri(self.0[..=path_start].to_owned()));
        }
        Some(Uri(self.0[..last].to_owned()))
    }

    /// Append a path segment, percent-encoding it as needed:
    /// `file:///src`.join(`"lib.rs"`) → `file:///src/lib.rs`. A multi-part
    /// segment (`"a/b"`) keeps its slashes as separators.
    pub fn join(&self, segment: &str) -> Uri {
        let base = self.0.strip_suffix('/').unwrap_or(&self.0);
        let mut joined = String::with_capacity(base.len() + segment.len() + 4);
        joined.push_str(base);
        joined.push('/');
        for ch in segment.chars() {
            encode_path_char(&mut joined, ch);
        }
        Uri(joined)
    }

    /// Byte index of the first `/` that begins the URI's path (the one
    /// after `scheme://authority`, or after `scheme:` for authority-less
    /// URIs). `None` when the URI has no path at all.
    fn path_start(&self) -> Option<usize> {
        let after_scheme = match self.0.find(':') {
            Some(idx) if self.scheme().is_some() => idx + 1,
            _ => 0,
        };
        let rest = &self.0[after_scheme..];
        let path_search_from = match rest.strip_prefix("//") {
            // Skip the authority: the path starts at the next '/'.
            Some(authority) => after_scheme + 2 + authority.find('/')?,
            None => after_scheme + rest.find('/')?,
        };
        Some(path_search_from)
    }

    /// Convert a `file://` URI back to a filesystem path, percent-decoding
    /// as needed. Returns `None` if the URI is not a `file` URI, names a
    /// remote host, or decodes to invalid UTF-8.
    pub fn to_file_path(&self) -> Option<PathBuf> {
        let rest = self.0.strip_prefix("file://")?;
        let (authority, path) = match rest.find('/') {
            Some(idx) => rest.split_at(idx),
            None => (rest, ""),
        };
        if !(authority.is_empty() || authority.eq_ignore_ascii_case("localhost")) {
            return None;
        }
        let decoded = percent_decode(path)?;
        #[cfg(windows)]
        {
            // `file:///c:/x` decodes to `/c:/x`; strip the leading slash
            // before a drive letter.
            let bytes = decoded.as_bytes();
            let decoded = if bytes.len() >= 3
                && bytes[0] == b'/'
                && bytes[1].is_ascii_alphabetic()
                && bytes[2] == b':'
            {
                decoded[1..].replace('/', "\\")
            } else {
                decoded.replace('/', "\\")
            };
            return Some(PathBuf::from(decoded));
        }
        #[cfg(not(windows))]
        Some(PathBuf::from(decoded))
    }
}

/// Whether `scheme` is a syntactically valid URI scheme
/// (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`).
fn is_valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Lowercase the scheme and uppercase percent-encoding hex digits.
fn normalize_uri(uri: String) -> String {
    let scheme_len = match uri.split_once(':') {
        Some((scheme, _)) if is_valid_scheme(scheme) => scheme.len(),
        _ => 0,
    };
    let needs_scheme_fix = uri[..scheme_len].bytes().any(|b| b.is_ascii_uppercase());
    let needs_hex_fix = uri.bytes().enumerate().any(|(i, b)| {
        b == b'%'
            && uri.as_bytes()[i + 1..]
                .iter()
                .take(2)
                .any(u8::is_ascii_lowercase)
    });
    if !needs_scheme_fix && !needs_hex_fix {
        return uri;
    }
    let mut bytes = uri.into_bytes();
    for b in &mut bytes[..scheme_len] {
        b.make_ascii_lowercase();
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            for b in bytes.iter_mut().skip(i + 1).take(2) {
                if b.is_ascii_hexdigit() {
                    b.make_ascii_uppercase();
                }
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    // The transformations only touch ASCII bytes, so UTF-8 validity holds.
    String::from_utf8(bytes).expect("ASCII-only edits preserve UTF-8")
}

/// Append `ch` to `out`, percent-encoding it unless it is an RFC 3986
/// unreserved character or a path separator (and `:` on Windows, for drive
/// letters).
fn encode_path_char(out: &mut String, ch: char) {
    let keep = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~' | '/');
    #[cfg(windows)]
    let keep = keep || ch == ':';
    if keep {
        out.push(ch);
    } else {
        let mut buf = [0u8; 4];
        for byte in ch.encode_utf8(&mut buf).bytes() {
            out.push('%');
            out.push(
                char::from_digit((byte >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((byte & 0xF) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
}

/// Percent-decode `s`, returning `None` on malformed escapes or invalid UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = char::from(*bytes.get(i + 1)?).to_digit(16)?;
            let lo = char::from(*bytes.get(i + 2)?).to_digit(16)?;
            out.push((hi as u8) << 4 | lo as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl<'de> Deserialize<'de> for Uri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Uri::new)
    }
}

impl From<String> for Uri {
    fn from(s: String) -> Self {
        Uri::new(s)
    }
}

impl From<&str> for Uri {
    fn from(s: &str) -> Self {
        Uri::new(s)
    }
}

impl std::fmt::Display for Uri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for Uri {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Uri {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// `Hash`/`Eq` on `Uri` delegate to the inner string, so borrowing as `str`
// is consistent — this is what lets `HashMap<Uri, _>` be queried by `&str`.
impl Borrow<str> for Uri {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Uri {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Uri {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Uri {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Uri> for str {
    fn eq(&self, other: &Uri) -> bool {
        self == other.0
    }
}

impl PartialEq<Uri> for &str {
    fn eq(&self, other: &Uri) -> bool {
        *self == other.0
    }
}

/// A zero-based position inside a text document.
///
/// `character` is, per the base LSP spec, an offset in **UTF-16 code units**
/// from the start of the line — not bytes and not Unicode scalar values.
/// Servers must convert accordingly when indexing UTF-8 buffers.
///
/// Positions order by line, then character, so `<`/`max`/sorting behave as
/// document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-16 code-unit offset within the line.
    pub character: u32,
}

impl Position {
    /// Construct a position from a line and character offset.
    pub fn new(line: u32, character: u32) -> Self {
        Position { line, character }
    }
}

/// A contiguous range `[start, end)` within a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    /// The range's start position (inclusive).
    pub start: Position,
    /// The range's end position (exclusive).
    pub end: Position,
}

impl Range {
    /// Construct a range from explicit start/end positions.
    pub fn new(start: Position, end: Position) -> Self {
        Range { start, end }
    }
}

/// A location: a [`Range`] within a specific document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    /// The document URI.
    pub uri: Uri,
    /// The range within that document.
    pub range: Range,
}

/// Identifies a text document by URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextDocumentIdentifier {
    /// The document URI.
    pub uri: Uri,
}

/// Identifies a text document together with the version it is expected to be at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    /// The document URI.
    pub uri: Uri,
    /// The version number after the associated change was applied.
    pub version: i32,
}

/// An item representing a document the client just opened.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentItem {
    /// The document URI.
    pub uri: Uri,
    /// The document's language identifier (e.g. `"rust"`).
    pub language_id: String,
    /// The version number of this document.
    pub version: i32,
    /// The full content of the opened document.
    pub text: String,
}

/// A document position parameter pair, flattened into feature request params.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentPositionParams {
    /// The document the request applies to.
    pub text_document: TextDocumentIdentifier,
    /// The position inside that document.
    pub position: Position,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_normalizes_scheme_and_percent_hex() {
        assert_eq!(Uri::new("FILE:///a%2fb").as_str(), "file:///a%2Fb");
        // Path case is preserved.
        assert_eq!(Uri::new("file:///A/b").as_str(), "file:///A/b");
        // Differently-spelled equivalents compare equal after normalization.
        assert_eq!(Uri::new("File:///x%3d"), Uri::new("file:///x%3D"));
    }

    #[test]
    fn uri_round_trips_through_serde_normalized() {
        let uri: Uri = serde_json::from_str("\"FILE:///a\"").unwrap();
        assert_eq!(uri, "file:///a");
        assert_eq!(serde_json::to_string(&uri).unwrap(), "\"file:///a\"");
    }

    #[test]
    fn uri_file_path_round_trip() {
        if cfg!(windows) {
            let uri = Uri::from_file_path("C:\\home\\user\\hello world.rs").expect("absolute path");
            assert_eq!(uri.as_str(), "file:///C:/home/user/hello%20world.rs");
            assert_eq!(
                uri.to_file_path().expect("file uri"),
                std::path::PathBuf::from("C:\\home\\user\\hello world.rs")
            );
        } else {
            let uri = Uri::from_file_path("/home/user/hello world.rs").expect("absolute path");
            assert_eq!(uri.as_str(), "file:///home/user/hello%20world.rs");
            assert_eq!(
                uri.to_file_path().expect("file uri"),
                std::path::PathBuf::from("/home/user/hello world.rs")
            );
        }
        assert!(Uri::from_file_path("relative/path").is_none());
        assert!(Uri::new("https://example.com/x").to_file_path().is_none());
        // A remote host is not a local file path.
        assert!(Uri::new("file://host/x").to_file_path().is_none());
        // `localhost` is accepted as local.
        if !cfg!(windows) {
            assert_eq!(
                Uri::new("file://localhost/x").to_file_path(),
                Some(std::path::PathBuf::from("/x"))
            );
        }
    }

    #[test]
    fn uri_scheme_parses_and_validates() {
        assert_eq!(Uri::new("file:///a").scheme(), Some("file"));
        assert_eq!(Uri::new("untitled:Untitled-1").scheme(), Some("untitled"));
        assert_eq!(Uri::new("not a uri").scheme(), None);
    }

    #[test]
    fn uri_borrows_as_str_for_map_lookups() {
        let mut map = std::collections::HashMap::new();
        map.insert(Uri::new("file:///a"), 1);
        assert_eq!(map.get("file:///a"), Some(&1));
    }
}

impl Range {
    /// Whether `position` lies inside this half-open `[start, end)` range.
    /// A position exactly at `end` is *not* contained (matching how the
    /// spec treats range ends as exclusive).
    pub fn contains(&self, position: Position) -> bool {
        self.start <= position && position < self.end
    }

    /// Whether this range and `other` share at least one position, under
    /// half-open semantics — two ranges that merely touch (`a.end ==
    /// b.start`) do not overlap.
    pub fn overlaps(&self, other: &Range) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// The shared sub-range of this range and `other`, or `None` when they
    /// do not [`overlaps`](Self::overlaps).
    pub fn intersection(&self, other: &Range) -> Option<Range> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then_some(Range { start, end })
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn positions_order_by_line_then_character() {
        assert!(Position::new(0, 9) < Position::new(1, 0));
        assert!(Position::new(2, 3) < Position::new(2, 4));
        assert!(Position::new(2, 3) <= Position::new(2, 3));
    }

    #[test]
    fn range_contains_is_half_open() {
        let range = Range::new(Position::new(1, 2), Position::new(1, 5));
        assert!(range.contains(Position::new(1, 2)));
        assert!(range.contains(Position::new(1, 4)));
        assert!(!range.contains(Position::new(1, 5))); // exclusive end
        assert!(!range.contains(Position::new(0, 3)));
    }

    #[test]
    fn range_overlap_and_intersection() {
        let a = Range::new(Position::new(0, 0), Position::new(0, 5));
        let b = Range::new(Position::new(0, 3), Position::new(0, 8));
        let touching = Range::new(Position::new(0, 5), Position::new(0, 9));

        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&touching)); // touching is not overlapping
        assert_eq!(
            a.intersection(&b),
            Some(Range::new(Position::new(0, 3), Position::new(0, 5)))
        );
        assert_eq!(a.intersection(&touching), None);
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn parent_walks_up_to_the_root() {
        let uri = Uri::new("file:///a/b/c.rs");
        let parent = uri.parent().expect("has parent");
        assert_eq!(parent, "file:///a/b");
        assert_eq!(parent.parent().expect("grandparent"), "file:///a");
        let root = parent.parent().unwrap().parent().expect("root");
        assert_eq!(root, "file:///");
        assert!(root.parent().is_none());
        assert!(Uri::new("untitled:Untitled-1").parent().is_none());
    }

    #[test]
    fn join_appends_and_percent_encodes() {
        let dir = Uri::new("file:///src");
        assert_eq!(dir.join("lib.rs"), "file:///src/lib.rs");
        assert_eq!(dir.join("my file.rs"), "file:///src/my%20file.rs");
        // Root and trailing-slash bases don't double the separator.
        assert_eq!(Uri::new("file:///").join("a"), "file:///a");
        assert_eq!(Uri::new("file:///a/").join("b"), "file:///a/b");
        // Multi-part segments keep their separators.
        assert_eq!(dir.join("tests/it.rs"), "file:///src/tests/it.rs");
        // join is the inverse of parent for simple segments.
        let file = Uri::new("file:///w/mod.rs");
        assert_eq!(file.parent().unwrap().join("mod.rs"), file);
    }
}
