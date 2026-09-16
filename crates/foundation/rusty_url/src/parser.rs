//! The [WHATWG basic URL parser](https://url.spec.whatwg.org/#url-parsing):
//! `Url::parse` (no base), base-relative parsing (`Url::join`), and the
//! full `ParseOptions` builder (base URL, encoding override, and
//! syntax-violation reporting).

use std::fmt::{self, Write as _};
use std::str::Chars;

use crate::form_urlencoded::EncodingOverride;
use crate::host::{Host, HostInternal};
use crate::percent_encode::{self, utf8_percent_encode, AsciiSet, CONTROLS};
use crate::url::Url;
use crate::ParseError;

pub(crate) type ParseResult<T> = Result<T, ParseError>;

/// Non-fatal syntax quirks noticed while parsing a URL — the URL still
/// parses successfully, but the input wasn't "clean" (a stray backslash
/// used as a path separator, embedded credentials, etc.). Report these via
/// [`crate::ParseOptions::syntax_violation_callback`]; they have no effect
/// on the parsed result.
///
/// This may be extended in the future, so exhaustive matching is forbidden.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyntaxViolation {
    Backslash,
    C0SpaceIgnored,
    EmbeddedCredentials,
    ExpectedDoubleSlash,
    ExpectedFileDoubleSlash,
    FileWithHostAndWindowsDrive,
    NonUrlCodePoint,
    NullInFragment,
    PercentDecode,
    TabOrNewlineIgnored,
    UnencodedAtSign,
}

impl SyntaxViolation {
    pub fn description(&self) -> &'static str {
        match self {
            Self::Backslash => "backslash",
            Self::C0SpaceIgnored => {
                "leading or trailing control or space character are ignored in URLs"
            }
            Self::EmbeddedCredentials => {
                "embedding authentication information (username or password) \
                 in an URL is not recommended"
            }
            Self::ExpectedDoubleSlash => "expected //",
            Self::ExpectedFileDoubleSlash => "expected // after file:",
            Self::FileWithHostAndWindowsDrive => "file: with host and Windows drive letter",
            Self::NonUrlCodePoint => "non-URL code point",
            Self::NullInFragment => "NULL characters are ignored in URL fragment identifiers",
            Self::PercentDecode => "expected 2 hex digits after %",
            Self::TabOrNewlineIgnored => "tabs or newlines are ignored in URLs",
            Self::UnencodedAtSign => "unencoded @ sign in username or password",
        }
    }
}

impl fmt::Display for SyntaxViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// <https://url.spec.whatwg.org/#fragment-percent-encode-set>
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');
/// <https://url.spec.whatwg.org/#path-percent-encode-set>
const PATH: &AsciiSet = &FRAGMENT.add(b'#').add(b'?').add(b'{').add(b'}');
/// Like [`PATH`], but also escapes `/` and `%` — used when a single path
/// *segment* string (not a full path) is being percent-encoded, so a
/// literal `/` in the segment can't be mistaken for a separator.
pub(crate) const PATH_SEGMENT: &AsciiSet = &PATH.add(b'/').add(b'%');
/// [`PATH_SEGMENT`], plus `\`, for special schemes where backslash is also
/// a path separator.
pub(crate) const SPECIAL_PATH_SEGMENT: &AsciiSet = &PATH_SEGMENT.add(b'\\');
/// <https://url.spec.whatwg.org/#userinfo-percent-encode-set>
pub(crate) const USERINFO: &AsciiSet = &PATH
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'|');
/// <https://url.spec.whatwg.org/#query-state>
const QUERY: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'<').add(b'>');
const SPECIAL_QUERY: &AsciiSet = &QUERY.add(b'\'');

/// A scheme's parsing category, per <https://url.spec.whatwg.org/#special-scheme>.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum SchemeType {
    File,
    SpecialNotFile,
    NotSpecial,
}

impl SchemeType {
    pub(crate) fn is_special(self) -> bool {
        !matches!(self, Self::NotSpecial)
    }

    pub(crate) fn is_file(self) -> bool {
        matches!(self, Self::File)
    }
}

impl From<&str> for SchemeType {
    fn from(scheme: &str) -> Self {
        match scheme {
            "http" | "https" | "ws" | "wss" | "ftp" => Self::SpecialNotFile,
            "file" => Self::File,
            _ => Self::NotSpecial,
        }
    }
}

/// <https://url.spec.whatwg.org/#default-port>
pub(crate) fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

pub(crate) fn to_u32(n: usize) -> ParseResult<u32> {
    u32::try_from(n).map_err(|_| ParseError::Overflow)
}

fn ascii_tab_or_new_line(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
}

fn c0_control_or_space(c: char) -> bool {
    c <= ' '
}

pub(crate) fn ascii_alpha(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// A `char` cursor over the remaining input, transparently skipping ASCII
/// tab/newline per <https://infra.spec.whatwg.org/#ascii-tab-or-newline> —
/// the URL Standard says these are removed wherever they occur, not just at
/// the ends of the input.
#[derive(Clone)]
pub(crate) struct Input<'i> {
    chars: Chars<'i>,
}

impl<'i> Input<'i> {
    /// Trims leading/trailing C0-control-or-space, matching
    /// <https://url.spec.whatwg.org/#url-parsing> steps 1-2.
    fn new_trim_c0_control_and_space(
        original: &'i str,
        vfn: Option<&dyn Fn(SyntaxViolation)>,
    ) -> Self {
        let trimmed = original.trim_matches(c0_control_or_space);
        if let Some(vfn) = vfn {
            if trimmed.len() < original.len() {
                vfn(SyntaxViolation::C0SpaceIgnored)
            }
            if trimmed.chars().any(ascii_tab_or_new_line) {
                vfn(SyntaxViolation::TabOrNewlineIgnored)
            }
        }
        Input {
            chars: trimmed.chars(),
        }
    }

    /// No trimming: used by setters, whose input is already an isolated
    /// component value (a scheme, path, query, or fragment), not a full URL
    /// string that might have stray leading/trailing whitespace.
    pub(crate) fn new_no_trim(original: &'i str) -> Self {
        Input {
            chars: original.chars(),
        }
    }

    fn starts_with_char(&self, c: char) -> bool {
        self.clone().next() == Some(c)
    }

    fn starts_with_str(&self, s: &str) -> bool {
        let mut remaining = self.clone();
        for c in s.chars() {
            if remaining.next() != Some(c) {
                return false;
            }
        }
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.clone().next().is_none()
    }

    /// If the input starts with `prefix`, return the input after it.
    fn split_prefix_str(&self, prefix: &str) -> Option<Self> {
        let mut remaining = self.clone();
        for c in prefix.chars() {
            if remaining.next() != Some(c) {
                return None;
            }
        }
        Some(remaining)
    }

    fn split_prefix_char(&self, c: char) -> Option<Self> {
        let mut remaining = self.clone();
        if remaining.next() == Some(c) {
            Some(remaining)
        } else {
            None
        }
    }

    fn split_first(&self) -> (Option<char>, Self) {
        let mut remaining = self.clone();
        (remaining.next(), remaining)
    }

    /// Count how many leading chars match `f`, and return the input after them.
    fn count_matching<F: Fn(char) -> bool>(&self, f: F) -> (u32, Self) {
        let mut count = 0;
        let mut remaining = self.clone();
        loop {
            let mut probe = remaining.clone();
            if matches!(probe.next(), Some(c) if f(c)) {
                remaining = probe;
                count += 1;
            } else {
                return (count, remaining);
            }
        }
    }

    /// Like `next`, but also returns the UTF-8 slice of the returned char.
    fn next_utf8(&mut self) -> Option<(char, &'i str)> {
        loop {
            let rest = self.chars.as_str();
            let c = self.chars.next()?;
            if !ascii_tab_or_new_line(c) {
                return Some((c, &rest[..c.len_utf8()]));
            }
        }
    }
}

impl Iterator for Input<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        self.chars.by_ref().find(|&c| !ascii_tab_or_new_line(c))
    }
}

/// Distinguishes top-level URL parsing (`Url::parse`) from setter re-parsing
/// of a single already-isolated component (`Url::set_path`, etc.), which
/// don't treat an embedded `?`/`#` as starting a new component — the
/// setter's input string *is* that one component, nothing more. A third
/// variant, `PathSegmentSetter`, is for `PathSegmentsMut::extend`: each
/// segment string is one opaque path segment, so even `/` and `%` within it
/// must be escaped rather than treated as a separator or existing encoding.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Context {
    UrlParser,
    Setter,
    PathSegmentSetter,
}

pub(crate) struct Parser<'a> {
    pub(crate) serialization: String,
    pub(crate) context: Context,
    base_url: Option<&'a Url>,
    query_encoding_override: EncodingOverride<'a>,
    violation_fn: Option<&'a dyn Fn(SyntaxViolation)>,
}

impl<'a> Parser<'a> {
    /// <https://url.spec.whatwg.org/#concept-basic-url-parser>, resolving a
    /// possibly-relative `input` against `base_url` (`Url::join`).
    pub(crate) fn parse_url_with_base(input: &str, base_url: &'a Url) -> ParseResult<Url> {
        Self::parse_url_full(input, Some(base_url), None, None)
    }

    /// The fully-configurable entry point backing [`crate::ParseOptions::parse`].
    pub(crate) fn parse_url_full(
        input: &str,
        base_url: Option<&'a Url>,
        query_encoding_override: EncodingOverride<'a>,
        violation_fn: Option<&'a dyn Fn(SyntaxViolation)>,
    ) -> ParseResult<Url> {
        let mut parser = Parser {
            serialization: String::with_capacity(input.len()),
            context: Context::UrlParser,
            base_url,
            query_encoding_override,
            violation_fn,
        };
        let input = Input::new_trim_c0_control_and_space(input, violation_fn);
        if let Ok(remaining) = parser.parse_scheme(input.clone()) {
            return parser.parse_with_scheme(remaining);
        }

        // No-scheme state: fall back to the base URL, if any.
        let Some(base_url) = base_url else {
            return Err(ParseError::RelativeUrlWithoutBase);
        };
        if input.starts_with_char('#') {
            parser.fragment_only(base_url, input)
        } else if base_url.cannot_be_a_base() {
            Err(ParseError::RelativeUrlWithCannotBeABaseBase)
        } else {
            let scheme_type = SchemeType::from(base_url.scheme());
            if scheme_type.is_file() {
                parser.parse_file(input, Some(base_url))
            } else {
                parser.parse_relative(input, scheme_type, base_url)
            }
        }
    }

    /// A parser for re-parsing one component of an already-valid `Url` (a
    /// setter). `serialization` is the buffer to append into — often empty,
    /// but callers that splice a component back into a larger string (e.g.
    /// `set_scheme`) pre-fill it.
    pub(crate) fn for_setter(serialization: String) -> Self {
        Parser {
            serialization,
            context: Context::Setter,
            base_url: None,
            query_encoding_override: None,
            violation_fn: None,
        }
    }

    fn log_violation(&self, v: SyntaxViolation) {
        if let Some(f) = self.violation_fn {
            f(v)
        }
    }

    fn log_violation_if(&self, v: SyntaxViolation, test: impl FnOnce() -> bool) {
        if let Some(f) = self.violation_fn {
            if test() {
                f(v)
            }
        }
    }

    pub(crate) fn parse_scheme<'i>(&mut self, mut input: Input<'i>) -> Result<Input<'i>, ()> {
        if !input.clone().next().is_some_and(ascii_alpha) {
            return Err(());
        }
        while let Some(c) = input.next() {
            match c {
                'a'..='z' | '0'..='9' | '+' | '-' | '.' => self.serialization.push(c),
                'A'..='Z' => self.serialization.push(c.to_ascii_lowercase()),
                ':' => return Ok(input),
                _ => {
                    self.serialization.clear();
                    return Err(());
                }
            }
        }
        // EOF before ':': only setters accept a bare scheme with no colon
        // (`url.set_scheme("http")`, not `"http:"`).
        if self.context == Context::Setter {
            Ok(input)
        } else {
            self.serialization.clear();
            Err(())
        }
    }

    fn parse_with_scheme(mut self, input: Input<'_>) -> ParseResult<Url> {
        let scheme_end = to_u32(self.serialization.len())?;
        let scheme_type = SchemeType::from(&self.serialization[..scheme_end as usize]);
        self.serialization.push(':');
        match scheme_type {
            SchemeType::File => {
                self.log_violation_if(SyntaxViolation::ExpectedFileDoubleSlash, || {
                    !input.starts_with_str("//")
                });
                let base_file_url = self.base_url.filter(|base| base.scheme() == "file");
                self.serialization.clear();
                self.parse_file(input, base_file_url)
            }
            SchemeType::SpecialNotFile => {
                let (slashes_count, remaining) = input.count_matching(|c| matches!(c, '/' | '\\'));
                if let Some(base_url) = self.base_url {
                    if slashes_count < 2
                        && base_url.scheme() == &self.serialization[..scheme_end as usize]
                    {
                        self.serialization.clear();
                        return self.parse_relative(input, scheme_type, base_url);
                    }
                }
                self.log_violation_if(SyntaxViolation::ExpectedDoubleSlash, || {
                    input
                        .clone()
                        .take_while(|&c| matches!(c, '/' | '\\'))
                        .collect::<String>()
                        != "//"
                });
                self.after_double_slash(remaining, scheme_type, scheme_end)
            }
            SchemeType::NotSpecial => self.parse_non_special(input, scheme_type, scheme_end),
        }
    }

    /// A scheme other than `file`, `http`, `https`, `ws`, `wss`, `ftp`.
    fn parse_non_special(
        mut self,
        input: Input<'_>,
        scheme_type: SchemeType,
        scheme_end: u32,
    ) -> ParseResult<Url> {
        if let Some(input) = input.split_prefix_str("//") {
            return self.after_double_slash(input, scheme_type, scheme_end);
        }
        // No authority: either a rooted path (`unix:/run/foo.socket`) or,
        // failing that, a cannot-be-a-base opaque path (`data:text/plain,x`).
        let path_start = to_u32(self.serialization.len())?;
        let (username_end, host_start, host_end, host, port) =
            (path_start, path_start, path_start, HostInternal::None, None);
        let remaining = if let Some(input) = input.split_prefix_char('/') {
            self.serialization.push('/');
            self.parse_path(scheme_type, &mut false, path_start as usize, input)
        } else {
            self.parse_cannot_be_a_base_path(input)
        };
        self.finish(
            scheme_end,
            username_end,
            host_start,
            host_end,
            host,
            port,
            path_start,
            remaining,
        )
    }

    fn parse_file(mut self, input: Input<'_>, base_file_url: Option<&Url>) -> ParseResult<Url> {
        debug_assert!(self.serialization.is_empty());
        let scheme_end = "file".len() as u32;
        let (first_char, input_after_first_char) = input.split_first();
        if matches!(first_char, Some('/') | Some('\\')) {
            self.log_violation_if(SyntaxViolation::Backslash, || first_char == Some('\\'));
            let (next_char, input_after_next_char) = input_after_first_char.split_first();
            if matches!(next_char, Some('/') | Some('\\')) {
                self.log_violation_if(SyntaxViolation::Backslash, || next_char == Some('\\'));
                // file host state
                self.serialization.push_str("file://");
                let host_start = "file://".len() as u32;
                let (has_path_start, host, remaining) =
                    self.parse_file_host(input_after_next_char)?;
                let mut host_end = to_u32(self.serialization.len())?;
                let mut has_host = !matches!(host, HostInternal::None);
                let remaining = if has_path_start {
                    self.parse_path_start(SchemeType::File, &mut has_host, remaining)
                } else {
                    let path_start = self.serialization.len();
                    self.serialization.push('/');
                    self.parse_path(SchemeType::File, &mut has_host, path_start, remaining)
                };
                let host = if has_host {
                    host
                } else {
                    self.serialization
                        .drain(host_start as usize..host_end as usize);
                    host_end = host_start;
                    HostInternal::None
                };
                let (query_start, fragment_start) =
                    self.parse_query_and_fragment(scheme_end, remaining)?;
                return Ok(Url {
                    serialization: self.serialization,
                    scheme_end,
                    username_end: host_start,
                    host_start,
                    host_end,
                    host,
                    port: None,
                    path_start: host_end,
                    query_start,
                    fragment_start,
                });
            }
            // file slash state, single '/' or '\\' not followed by another:
            // may inherit a host or Windows drive letter from the base URL.
            self.serialization.push_str("file://");
            let host_start = "file://".len();
            let mut host_end = host_start;
            let mut host = HostInternal::None;
            if !starts_with_windows_drive_letter_segment(&input_after_first_char) {
                if let Some(base_url) = base_file_url {
                    let first_segment = first_path_segment(base_url);
                    if is_normalized_windows_drive_letter(first_segment) {
                        self.serialization.push('/');
                        self.serialization.push_str(first_segment);
                    } else if let Some(host_str) = base_url.host_str() {
                        self.serialization.push_str(host_str);
                        host_end = self.serialization.len();
                        host = base_url.host;
                    }
                }
            }
            // Keep the leading '/' in the input fed to parse_path so it
            // becomes the path's own leading slash when there's no host.
            let parse_path_input = match first_char {
                Some('/') | Some('\\') | Some('?') | Some('#') => input,
                _ => input_after_first_char,
            };
            let remaining =
                self.parse_path(SchemeType::File, &mut false, host_end, parse_path_input);
            let host_start = host_start as u32;
            let (query_start, fragment_start) =
                self.parse_query_and_fragment(scheme_end, remaining)?;
            let host_end = host_end as u32;
            return Ok(Url {
                serialization: self.serialization,
                scheme_end,
                username_end: host_start,
                host_start,
                host_end,
                host,
                port: None,
                path_start: host_end,
                query_start,
                fragment_start,
            });
        }
        if let Some(base_url) = base_file_url {
            match first_char {
                None => {
                    let before_fragment = match base_url.fragment_start {
                        Some(i) => &base_url.serialization[..i as usize],
                        None => base_url.serialization.as_str(),
                    };
                    self.serialization.push_str(before_fragment);
                    Ok(Url {
                        serialization: self.serialization,
                        fragment_start: None,
                        ..*base_url
                    })
                }
                Some('?') => {
                    let before_query = match (base_url.query_start, base_url.fragment_start) {
                        (None, None) => base_url.serialization.as_str(),
                        (Some(i), _) | (None, Some(i)) => &base_url.serialization[..i as usize],
                    };
                    self.serialization.push_str(before_query);
                    let (query_start, fragment_start) =
                        self.parse_query_and_fragment(base_url.scheme_end, input)?;
                    Ok(Url {
                        serialization: self.serialization,
                        query_start,
                        fragment_start,
                        ..*base_url
                    })
                }
                Some('#') => self.fragment_only(base_url, input),
                _ => {
                    if !starts_with_windows_drive_letter_segment(&input) {
                        let before_query = match (base_url.query_start, base_url.fragment_start) {
                            (None, None) => base_url.serialization.as_str(),
                            (Some(i), _) | (None, Some(i)) => &base_url.serialization[..i as usize],
                        };
                        self.serialization.push_str(before_query);
                        self.shorten_path(SchemeType::File, base_url.path_start as usize);
                        let remaining = self.parse_path(
                            SchemeType::File,
                            &mut true,
                            base_url.path_start as usize,
                            input,
                        );
                        self.finish(
                            base_url.scheme_end,
                            base_url.username_end,
                            base_url.host_start,
                            base_url.host_end,
                            base_url.host,
                            base_url.port,
                            base_url.path_start,
                            remaining,
                        )
                    } else {
                        self.serialization.push_str("file:///");
                        let path_start = "file://".len();
                        let remaining =
                            self.parse_path(SchemeType::File, &mut false, path_start, input);
                        let (query_start, fragment_start) =
                            self.parse_query_and_fragment(scheme_end, remaining)?;
                        let path_start = path_start as u32;
                        Ok(Url {
                            serialization: self.serialization,
                            scheme_end,
                            username_end: path_start,
                            host_start: path_start,
                            host_end: path_start,
                            host: HostInternal::None,
                            port: None,
                            path_start,
                            query_start,
                            fragment_start,
                        })
                    }
                }
            }
        } else {
            // file state, next char is not '/' or '\\' (or EOF), and no base:
            // no authority slashes at all.
            self.serialization.push_str("file:///");
            let path_start = "file://".len();
            let remaining = self.parse_path(SchemeType::File, &mut false, path_start, input);
            let (query_start, fragment_start) =
                self.parse_query_and_fragment(scheme_end, remaining)?;
            let path_start = path_start as u32;
            Ok(Url {
                serialization: self.serialization,
                scheme_end,
                username_end: path_start,
                host_start: path_start,
                host_end: path_start,
                host: HostInternal::None,
                port: None,
                path_start,
                query_start,
                fragment_start,
            })
        }
    }

    /// <https://url.spec.whatwg.org/#relative-state> — `input` has no
    /// scheme, so it's resolved relative to `base_url` (`Url::join`).
    /// `base_url`'s scheme is not `file` (that's handled by `parse_file`).
    fn parse_relative(
        mut self,
        input: Input<'_>,
        scheme_type: SchemeType,
        base_url: &Url,
    ) -> ParseResult<Url> {
        debug_assert!(self.serialization.is_empty());
        let (first_char, input_after_first_char) = input.split_first();
        match first_char {
            None => {
                let before_fragment = match base_url.fragment_start {
                    Some(i) => &base_url.serialization[..i as usize],
                    None => base_url.serialization.as_str(),
                };
                self.serialization.push_str(before_fragment);
                Ok(Url {
                    serialization: self.serialization,
                    fragment_start: None,
                    ..*base_url
                })
            }
            Some('?') => {
                let before_query = match (base_url.query_start, base_url.fragment_start) {
                    (None, None) => base_url.serialization.as_str(),
                    (Some(i), _) | (None, Some(i)) => &base_url.serialization[..i as usize],
                };
                self.serialization.push_str(before_query);
                let (query_start, fragment_start) =
                    self.parse_query_and_fragment(base_url.scheme_end, input)?;
                Ok(Url {
                    serialization: self.serialization,
                    query_start,
                    fragment_start,
                    ..*base_url
                })
            }
            Some('#') => self.fragment_only(base_url, input),
            Some('/') | Some('\\') => {
                let (slashes_count, remaining) = input.count_matching(|c| matches!(c, '/' | '\\'));
                if slashes_count >= 2 {
                    self.log_violation_if(SyntaxViolation::ExpectedDoubleSlash, || {
                        input
                            .clone()
                            .take_while(|&c| matches!(c, '/' | '\\'))
                            .collect::<String>()
                            != "//"
                    });
                    let scheme_end = base_url.scheme_end;
                    self.serialization
                        .push_str(&base_url.serialization[..scheme_end as usize + 1]);
                    if let Some(after_prefix) = input.split_prefix_str("//") {
                        return self.after_double_slash(after_prefix, scheme_type, scheme_end);
                    }
                    return self.after_double_slash(remaining, scheme_type, scheme_end);
                }
                let path_start = base_url.path_start;
                self.serialization
                    .push_str(&base_url.serialization[..path_start as usize]);
                self.serialization.push('/');
                let remaining = self.parse_path(
                    scheme_type,
                    &mut true,
                    path_start as usize,
                    input_after_first_char,
                );
                self.finish(
                    base_url.scheme_end,
                    base_url.username_end,
                    base_url.host_start,
                    base_url.host_end,
                    base_url.host,
                    base_url.port,
                    base_url.path_start,
                    remaining,
                )
            }
            _ => {
                let before_query = match (base_url.query_start, base_url.fragment_start) {
                    (None, None) => base_url.serialization.as_str(),
                    (Some(i), _) | (None, Some(i)) => &base_url.serialization[..i as usize],
                };
                self.serialization.push_str(before_query);
                self.pop_path(scheme_type, base_url.path_start as usize);
                // A special URL always has a path, and a path always starts with '/'.
                if self.serialization.len() == base_url.path_start as usize
                    && (scheme_type.is_special() || !input.is_empty())
                {
                    self.serialization.push('/');
                }
                let remaining = match input.split_first() {
                    (Some('/'), remaining) => self.parse_path(
                        scheme_type,
                        &mut true,
                        base_url.path_start as usize,
                        remaining,
                    ),
                    _ => {
                        self.parse_path(scheme_type, &mut true, base_url.path_start as usize, input)
                    }
                };
                self.finish(
                    base_url.scheme_end,
                    base_url.username_end,
                    base_url.host_start,
                    base_url.host_end,
                    base_url.host,
                    base_url.port,
                    base_url.path_start,
                    remaining,
                )
            }
        }
    }

    fn fragment_only(mut self, base_url: &Url, input: Input<'_>) -> ParseResult<Url> {
        let before_fragment = match base_url.fragment_start {
            Some(i) => &base_url.serialization[..i as usize],
            None => base_url.serialization.as_str(),
        };
        self.serialization.push_str(before_fragment);
        self.serialization.push('#');
        let mut input = input;
        let next = input.next();
        debug_assert!(next == Some('#'));
        self.parse_fragment(input);
        Ok(Url {
            serialization: self.serialization,
            fragment_start: Some(to_u32(before_fragment.len())?),
            ..*base_url
        })
    }

    fn after_double_slash(
        mut self,
        input: Input<'_>,
        scheme_type: SchemeType,
        scheme_end: u32,
    ) -> ParseResult<Url> {
        self.serialization.push('/');
        self.serialization.push('/');
        let before_authority = self.serialization.len();
        let (username_end, remaining) = self.parse_userinfo(input, scheme_type)?;
        let has_authority = before_authority != self.serialization.len();
        let host_start = to_u32(self.serialization.len())?;
        let (host_end, host, port, remaining) =
            self.parse_host_and_port(remaining, scheme_end, scheme_type)?;
        if host == HostInternal::None && has_authority {
            return Err(ParseError::EmptyHost);
        }
        let path_start = to_u32(self.serialization.len())?;
        let mut has_host = host != HostInternal::None;
        let remaining = self.parse_path_start(scheme_type, &mut has_host, remaining);
        self.finish(
            scheme_end,
            username_end,
            host_start,
            host_end,
            host,
            port,
            path_start,
            remaining,
        )
    }

    /// Return (username_end, remaining).
    fn parse_userinfo<'i>(
        &mut self,
        input: Input<'i>,
        scheme_type: SchemeType,
    ) -> ParseResult<(u32, Input<'i>)> {
        let mut last_at = None;
        let mut remaining = input.clone();
        let mut char_count = 0u32;
        while let Some(c) = remaining.next() {
            match c {
                '@' => {
                    if last_at.is_some() {
                        self.log_violation(SyntaxViolation::UnencodedAtSign)
                    } else {
                        self.log_violation(SyntaxViolation::EmbeddedCredentials)
                    }
                    last_at = Some((char_count, remaining.clone()))
                }
                '/' | '?' | '#' => break,
                '\\' if scheme_type.is_special() => break,
                _ => (),
            }
            char_count += 1;
        }
        let (mut userinfo_char_count, remaining) = match last_at {
            None => return Ok((to_u32(self.serialization.len())?, input)),
            Some((0, remaining)) => {
                if let (Some(c), _) = remaining.split_first() {
                    if c == '/' || c == '?' || c == '#' || (scheme_type.is_special() && c == '\\') {
                        return Err(ParseError::EmptyHost);
                    }
                }
                return Ok((to_u32(self.serialization.len())?, remaining));
            }
            Some(x) => x,
        };

        let mut input = input;
        let mut username_end = None;
        let mut has_password = false;
        let mut has_username = false;
        while userinfo_char_count > 0 {
            let (c, utf8_c) = input.next_utf8().unwrap();
            userinfo_char_count -= 1;
            if c == ':' && username_end.is_none() {
                username_end = Some(to_u32(self.serialization.len())?);
                if userinfo_char_count > 0 {
                    self.serialization.push(':');
                    has_password = true;
                }
            } else {
                if !has_password {
                    has_username = true;
                }
                self.serialization
                    .push_str(&utf8_percent_encode(utf8_c, USERINFO));
            }
        }
        let username_end = match username_end {
            Some(i) => i,
            None => to_u32(self.serialization.len())?,
        };
        if has_username || has_password {
            self.serialization.push('@');
        }
        Ok((username_end, remaining))
    }

    fn parse_host_and_port<'i>(
        &mut self,
        input: Input<'i>,
        scheme_end: u32,
        scheme_type: SchemeType,
    ) -> ParseResult<(u32, HostInternal, Option<u16>, Input<'i>)> {
        let (host, remaining) = Self::parse_host(input, scheme_type)?;
        write!(&mut self.serialization, "{host}").unwrap();
        let host_end = to_u32(self.serialization.len())?;
        if let Host::Domain(h) = &host {
            if h.is_empty() && (remaining.starts_with_char(':') || scheme_type.is_special()) {
                return Err(ParseError::EmptyHost);
            }
        }

        let (port, remaining) = if let Some(remaining) = remaining.split_prefix_char(':') {
            let scheme = &self.serialization[..scheme_end as usize];
            let (port, remaining) = Self::parse_port(remaining, || default_port(scheme))?;
            if let Some(port) = port {
                self.serialization.push(':');
                write!(&mut self.serialization, "{port}").unwrap();
            }
            (port, remaining)
        } else {
            (None, remaining)
        };
        Ok((host_end, host.into(), port, remaining))
    }

    fn parse_host(
        input: Input<'_>,
        scheme_type: SchemeType,
    ) -> ParseResult<(Host<String>, Input<'_>)> {
        if scheme_type.is_file() {
            return Self::get_file_host(input);
        }
        let mut host_chars = String::new();
        let mut inside_brackets = false;
        let mut remaining = input.clone();
        loop {
            let (c, next) = remaining.split_first();
            match c {
                Some(':') if !inside_brackets => break,
                Some('\\') if scheme_type.is_special() => break,
                Some('/') | Some('?') | Some('#') => break,
                Some('[') => {
                    inside_brackets = true;
                    host_chars.push('[');
                }
                Some(']') => {
                    inside_brackets = false;
                    host_chars.push(']');
                }
                Some(c) => host_chars.push(c),
                None => break,
            }
            remaining = next;
        }
        if scheme_type == SchemeType::SpecialNotFile && host_chars.is_empty() {
            return Err(ParseError::EmptyHost);
        }
        let host = if scheme_type.is_special() {
            Host::parse(&host_chars)?
        } else {
            Host::parse_opaque(&host_chars)?
        };
        Ok((host, remaining))
    }

    fn get_file_host(input: Input<'_>) -> ParseResult<(Host<String>, Input<'_>)> {
        let (_, host_str, remaining) = Self::file_host(input)?;
        let host = match Host::parse(&host_str)? {
            Host::Domain(d) if d == "localhost" => Host::Domain(String::new()),
            host => host,
        };
        Ok((host, remaining))
    }

    fn parse_file_host<'i>(
        &mut self,
        input: Input<'i>,
    ) -> ParseResult<(bool, HostInternal, Input<'i>)> {
        let (_, host_str, remaining) = Self::file_host(input)?;
        let (has_host, host) = if host_str.is_empty() {
            (false, HostInternal::None)
        } else {
            match Host::parse(&host_str)? {
                Host::Domain(d) if d == "localhost" => (false, HostInternal::None),
                host => {
                    write!(&mut self.serialization, "{host}").unwrap();
                    (true, host.into())
                }
            }
        };
        Ok((has_host, host, remaining))
    }

    /// Returns (is_a_host, host_str, remaining). A Windows drive letter
    /// segment (`c:`) is never a host.
    fn file_host(input: Input<'_>) -> ParseResult<(bool, String, Input<'_>)> {
        let mut host_chars = String::new();
        let mut remaining = input.clone();
        loop {
            let (c, next) = remaining.split_first();
            match c {
                Some('/') | Some('\\') | Some('?') | Some('#') | None => break,
                Some(c) => host_chars.push(c),
            }
            remaining = next;
        }
        if is_windows_drive_letter(&host_chars) {
            return Ok((false, String::new(), input));
        }
        Ok((true, host_chars, remaining))
    }

    fn parse_port<P: Fn() -> Option<u16>>(
        input: Input<'_>,
        default_port: P,
    ) -> ParseResult<(Option<u16>, Input<'_>)> {
        let mut port: u32 = 0;
        let mut has_any_digit = false;
        let mut input = input;
        loop {
            let (c, remaining) = input.split_first();
            match c.and_then(|c| c.to_digit(10)) {
                Some(digit) => {
                    port = port * 10 + digit;
                    if port > u16::MAX as u32 {
                        return Err(ParseError::InvalidPort);
                    }
                    has_any_digit = true;
                }
                None => {
                    if !matches!(c, None | Some('/') | Some('\\') | Some('?') | Some('#')) {
                        return Err(ParseError::InvalidPort);
                    }
                    break;
                }
            }
            input = remaining;
        }
        let mut opt_port = Some(port as u16);
        if !has_any_digit || opt_port == default_port() {
            opt_port = None;
        }
        Ok((opt_port, input))
    }

    pub(crate) fn parse_path_start<'i>(
        &mut self,
        scheme_type: SchemeType,
        has_host: &mut bool,
        input: Input<'i>,
    ) -> Input<'i> {
        let path_start = self.serialization.len();
        let (maybe_c, remaining) = input.split_first();
        if scheme_type.is_special() {
            if maybe_c == Some('\\') {
                self.log_violation(SyntaxViolation::Backslash);
            }
            if !self.serialization.ends_with('/') {
                self.serialization.push('/');
                if maybe_c == Some('/') || maybe_c == Some('\\') {
                    return self.parse_path(scheme_type, has_host, path_start, remaining);
                }
            }
            return self.parse_path(scheme_type, has_host, path_start, input);
        } else if maybe_c == Some('?') || maybe_c == Some('#') {
            return input;
        }
        if maybe_c.is_some() && maybe_c != Some('/') {
            self.serialization.push('/');
        }
        self.parse_path(scheme_type, has_host, path_start, input)
    }

    pub(crate) fn parse_path<'i>(
        &mut self,
        scheme_type: SchemeType,
        has_host: &mut bool,
        path_start: usize,
        mut input: Input<'i>,
    ) -> Input<'i> {
        fn push_pending(
            serialization: &mut String,
            start_str: &str,
            remaining_len: usize,
            context: Context,
            scheme_type: SchemeType,
        ) {
            let text = &start_str[..start_str.len() - remaining_len];
            if text.is_empty() {
                return;
            }
            let set = if context == Context::PathSegmentSetter {
                if scheme_type.is_special() {
                    SPECIAL_PATH_SEGMENT
                } else {
                    PATH_SEGMENT
                }
            } else {
                PATH
            };
            serialization.push_str(&utf8_percent_encode(text, set));
        }

        loop {
            let segment_start = self.serialization.len();
            let mut ends_with_slash = false;
            let mut start_str = input.chars.as_str();
            loop {
                let input_before_c = input.clone();
                let c = if let Some(c) = input.chars.next() {
                    c
                } else {
                    push_pending(
                        &mut self.serialization,
                        start_str,
                        0,
                        self.context,
                        scheme_type,
                    );
                    break;
                };
                match c {
                    '\t' | '\n' | '\r' => {
                        push_pending(
                            &mut self.serialization,
                            start_str,
                            input_before_c.chars.as_str().len(),
                            self.context,
                            scheme_type,
                        );
                        start_str = input.chars.as_str();
                    }
                    '/' if self.context != Context::PathSegmentSetter => {
                        push_pending(
                            &mut self.serialization,
                            start_str,
                            input_before_c.chars.as_str().len(),
                            self.context,
                            scheme_type,
                        );
                        self.serialization.push('/');
                        ends_with_slash = true;
                        break;
                    }
                    '\\' if self.context != Context::PathSegmentSetter
                        && scheme_type.is_special() =>
                    {
                        push_pending(
                            &mut self.serialization,
                            start_str,
                            input_before_c.chars.as_str().len(),
                            self.context,
                            scheme_type,
                        );
                        self.log_violation(SyntaxViolation::Backslash);
                        self.serialization.push('/');
                        ends_with_slash = true;
                        break;
                    }
                    '?' | '#' if self.context == Context::UrlParser => {
                        push_pending(
                            &mut self.serialization,
                            start_str,
                            input_before_c.chars.as_str().len(),
                            self.context,
                            scheme_type,
                        );
                        input = input_before_c;
                        break;
                    }
                    _ => {
                        self.check_url_code_point(c, &input);
                    }
                }
            }

            let segment_before_slash = if ends_with_slash {
                &self.serialization[segment_start..self.serialization.len() - 1]
            } else {
                &self.serialization[segment_start..]
            };
            match segment_before_slash {
                ".." | "%2e%2e" | "%2e%2E" | "%2E%2e" | "%2E%2E" | "%2e." | "%2E." | ".%2e"
                | ".%2E" => {
                    self.serialization.truncate(segment_start);
                    if self.serialization.ends_with('/')
                        && Self::last_slash_can_be_removed(&self.serialization, path_start)
                    {
                        self.serialization.pop();
                    }
                    self.shorten_path(scheme_type, path_start);
                    if ends_with_slash && !self.serialization.ends_with('/') {
                        self.serialization.push('/');
                    }
                }
                "." | "%2e" | "%2E" => {
                    self.serialization.truncate(segment_start);
                    if !self.serialization.ends_with('/') {
                        self.serialization.push('/');
                    }
                }
                _ => {
                    if scheme_type.is_file()
                        && segment_start == path_start + 1
                        && is_windows_drive_letter(segment_before_slash)
                    {
                        if let Some(c) = segment_before_slash.chars().next() {
                            self.serialization.truncate(segment_start);
                            self.serialization.push(c);
                            self.serialization.push(':');
                            if ends_with_slash {
                                self.serialization.push('/');
                            }
                        }
                        // https://url.spec.whatwg.org/#path-state step on
                        // Windows drive letters: a file: URL can't have both
                        // a host and a drive-letter path.
                        if *has_host {
                            self.log_violation(SyntaxViolation::FileWithHostAndWindowsDrive);
                            *has_host = false;
                        }
                    }
                }
            }
            if !ends_with_slash {
                break;
            }
        }
        if scheme_type.is_file() {
            let path = self.serialization.split_off(path_start);
            self.serialization.push('/');
            self.serialization.push_str(path.trim_start_matches('/'));
        }
        input
    }

    fn last_slash_can_be_removed(serialization: &str, path_start: usize) -> bool {
        let before_segment = &serialization[..serialization.len() - 1];
        match before_segment.rfind('/') {
            Some(i) => {
                i >= path_start && !path_starts_with_windows_drive_letter(&serialization[i..])
            }
            None => false,
        }
    }

    /// <https://url.spec.whatwg.org/#shorten-a-urls-path>
    fn shorten_path(&mut self, scheme_type: SchemeType, path_start: usize) {
        if self.serialization.len() == path_start {
            return;
        }
        if scheme_type.is_file()
            && is_normalized_windows_drive_letter(&self.serialization[path_start..])
        {
            return;
        }
        self.pop_path(scheme_type, path_start);
    }

    /// <https://url.spec.whatwg.org/#pop-a-urls-path>
    fn pop_path(&mut self, scheme_type: SchemeType, path_start: usize) {
        if self.serialization.len() > path_start {
            let slash = self.serialization[path_start..].rfind('/').unwrap();
            let segment_start = path_start + slash + 1;
            if !(scheme_type.is_file()
                && is_normalized_windows_drive_letter(&self.serialization[segment_start..]))
            {
                self.serialization.truncate(segment_start);
            }
        }
    }

    pub(crate) fn parse_cannot_be_a_base_path<'i>(&mut self, mut input: Input<'i>) -> Input<'i> {
        loop {
            let input_before_c = input.clone();
            match input.next_utf8() {
                Some(('?', _)) | Some(('#', _)) if self.context == Context::UrlParser => {
                    return input_before_c
                }
                Some((c, utf8_c)) => {
                    self.check_url_code_point(c, &input);
                    self.serialization
                        .push_str(&utf8_percent_encode(utf8_c, CONTROLS));
                }
                None => return input,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        mut self,
        scheme_end: u32,
        username_end: u32,
        host_start: u32,
        host_end: u32,
        host: HostInternal,
        port: Option<u16>,
        mut path_start: u32,
        remaining: Input<'_>,
    ) -> ParseResult<Url> {
        // Avoids e.g. `web+demo:/.//not-a-host/` (host = null, opaque path
        // with a leading empty segment) round-tripping through
        // parse -> serialize -> parse as though it had an authority.
        let scheme_end_usize = scheme_end as usize;
        let path_start_usize = path_start as usize;
        if path_start_usize == scheme_end_usize + 1
            && self.serialization[path_start_usize..].starts_with("//")
        {
            self.serialization.insert_str(path_start_usize, "/.");
            path_start += 2;
        }

        let (query_start, fragment_start) = self.parse_query_and_fragment(scheme_end, remaining)?;
        Ok(Url {
            serialization: self.serialization,
            scheme_end,
            username_end,
            host_start,
            host_end,
            host,
            port,
            path_start,
            query_start,
            fragment_start,
        })
    }

    /// Return (query_start, fragment_start).
    fn parse_query_and_fragment(
        &mut self,
        scheme_end: u32,
        mut input: Input<'_>,
    ) -> ParseResult<(Option<u32>, Option<u32>)> {
        let scheme_type = SchemeType::from(&self.serialization[..scheme_end as usize]);
        let mut query_start = None;
        match input.next() {
            Some('#') => {}
            Some('?') => {
                query_start = Some(to_u32(self.serialization.len())?);
                self.serialization.push('?');
                match self.parse_query(scheme_type, scheme_end, input) {
                    Some(remaining) => input = remaining,
                    None => return Ok((query_start, None)),
                }
            }
            None => return Ok((None, None)),
            Some(_) => unreachable!("parse_query_and_fragment called without ? or #"),
        }
        let fragment_start = to_u32(self.serialization.len())?;
        self.serialization.push('#');
        self.parse_fragment(input);
        Ok((query_start, Some(fragment_start)))
    }

    /// Consume up to (not including) a `#`, percent-encoding into
    /// `self.serialization`. Returns the remaining input starting *after*
    /// the `#` if one was found, `None` at EOF. In `Setter` context, `#`
    /// isn't a terminator — the whole input is the query, since the caller
    /// (`Url::set_query`) already isolated it from any fragment.
    pub(crate) fn parse_query<'i>(
        &mut self,
        scheme_type: SchemeType,
        scheme_end: u32,
        input: Input<'i>,
    ) -> Option<Input<'i>> {
        let set = if scheme_type.is_special() {
            SPECIAL_QUERY
        } else {
            QUERY
        };
        let encoding_override = self.query_encoding_override.filter(|_| {
            matches!(
                &self.serialization[..scheme_end as usize],
                "http" | "https" | "file" | "ftp"
            )
        });
        let mut input = input;
        loop {
            let start = input.chars.as_str();
            loop {
                let before_len = input.chars.as_str().len();
                match input.chars.next() {
                    None => {
                        let text = &start[..start.len() - before_len];
                        if !text.is_empty() {
                            push_encoded(&mut self.serialization, text, set, encoding_override);
                        }
                        return None;
                    }
                    Some('\t') | Some('\n') | Some('\r') => {
                        let text = &start[..start.len() - before_len];
                        push_encoded(&mut self.serialization, text, set, encoding_override);
                        break;
                    }
                    Some('#') if self.context == Context::UrlParser => {
                        let text = &start[..start.len() - before_len];
                        push_encoded(&mut self.serialization, text, set, encoding_override);
                        return Some(input);
                    }
                    Some(c) => {
                        self.check_url_code_point(c, &input);
                    }
                }
            }
        }
    }

    pub(crate) fn parse_fragment(&mut self, input: Input<'_>) {
        let mut input = input;
        loop {
            let start = input.chars.as_str();
            loop {
                let before_len = input.chars.as_str().len();
                match input.chars.next() {
                    None => {
                        let text = &start[..start.len() - before_len];
                        if !text.is_empty() {
                            self.serialization
                                .push_str(&utf8_percent_encode(text, FRAGMENT));
                        }
                        return;
                    }
                    Some('\t') | Some('\n') | Some('\r') => {
                        let text = &start[..start.len() - before_len];
                        self.serialization
                            .push_str(&utf8_percent_encode(text, FRAGMENT));
                        break;
                    }
                    Some('\0') => {
                        self.log_violation(SyntaxViolation::NullInFragment);
                    }
                    Some(c) => {
                        self.check_url_code_point(c, &input);
                    }
                }
            }
        }
    }

    #[inline]
    fn check_url_code_point(&self, c: char, input: &Input<'_>) {
        if let Some(vfn) = self.violation_fn {
            check_url_code_point(vfn, c, input)
        }
    }
}

/// Percent-encode `text`, running it through `encoding_override` first if
/// present (used for `query_encoding_override`, which may replace the query
/// string's bytes with a non-UTF-8 legacy encoding).
fn push_encoded(
    serialization: &mut String,
    text: &str,
    set: &'static AsciiSet,
    encoding_override: EncodingOverride<'_>,
) {
    match encoding_override {
        Some(o) => {
            let overridden = o(text);
            let encoded = percent_encode::percent_encode(&overridden, set);
            serialization
                .push_str(std::str::from_utf8(&encoded).expect("percent-encoding is ASCII-only"));
        }
        None => serialization.push_str(&utf8_percent_encode(text, set)),
    }
}

fn check_url_code_point(vfn: &dyn Fn(SyntaxViolation), c: char, input: &Input<'_>) {
    if c == '%' {
        let mut input = input.clone();
        if !matches!((input.next(), input.next()), (Some(a), Some(b))
            if a.is_ascii_hexdigit() && b.is_ascii_hexdigit())
        {
            vfn(SyntaxViolation::PercentDecode)
        }
    } else if !is_url_code_point(c) {
        vfn(SyntaxViolation::NonUrlCodePoint)
    }
}

// Non URL code points:
// U+0000 to U+0020 (space)
// " # % < > [ \ ] ^ ` { | }
// U+007F to U+009F
// surrogates
// U+FDD0 to U+FDEF
// Last two of each plane: U+__FFFE to U+__FFFF for __ in 00 to 10 hex
#[inline]
fn is_url_code_point(c: char) -> bool {
    matches!(c,
        'a'..='z' |
        'A'..='Z' |
        '0'..='9' |
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | '-' |
        '.' | '/' | ':' | ';' | '=' | '?' | '@' | '_' | '~' |
        '\u{A0}'..='\u{D7FF}' | '\u{E000}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' |
        '\u{10000}'..='\u{1FFFD}' | '\u{20000}'..='\u{2FFFD}' |
        '\u{30000}'..='\u{3FFFD}' | '\u{40000}'..='\u{4FFFD}' |
        '\u{50000}'..='\u{5FFFD}' | '\u{60000}'..='\u{6FFFD}' |
        '\u{70000}'..='\u{7FFFD}' | '\u{80000}'..='\u{8FFFD}' |
        '\u{90000}'..='\u{9FFFD}' | '\u{A0000}'..='\u{AFFFD}' |
        '\u{B0000}'..='\u{BFFFD}' | '\u{C0000}'..='\u{CFFFD}' |
        '\u{D0000}'..='\u{DFFFD}' | '\u{E1000}'..='\u{EFFFD}' |
        '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// The first `/`-separated path segment, or `""` for a cannot-be-a-base URL
/// or an empty path. A minimal stand-in for the public `path_segments()`
/// iterator (a later parity-loop issue) — used only internally here, for
/// `file:` base-URL Windows-drive-letter inheritance in `parse_file`.
fn first_path_segment(url: &Url) -> &str {
    url.path()
        .strip_prefix('/')
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
}

pub(crate) fn is_windows_drive_letter(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && ascii_alpha(bytes[0] as char) && matches!(bytes[1], b':' | b'|')
}

fn is_normalized_windows_drive_letter(segment: &str) -> bool {
    is_windows_drive_letter(segment) && segment.as_bytes()[1] == b':'
}

fn path_starts_with_windows_drive_letter(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !matches!(bytes[0], b'/' | b'\\' | b'?' | b'#') {
        return false;
    }
    let rest_bytes = &bytes[1..];
    rest_bytes.len() >= 2
        && ascii_alpha(rest_bytes[0] as char)
        && matches!(rest_bytes[1], b':' | b'|')
        && (rest_bytes.len() == 2 || matches!(rest_bytes[2], b'/' | b'\\' | b'?' | b'#'))
}

/// <https://url.spec.whatwg.org/#start-with-a-windows-drive-letter>
fn starts_with_windows_drive_letter_segment(input: &Input<'_>) -> bool {
    let mut input = input.clone();
    match (input.next(), input.next(), input.next()) {
        (Some(a), Some(b), Some(c))
            if ascii_alpha(a) && matches!(b, ':' | '|') && matches!(c, '/' | '\\' | '?' | '#') =>
        {
            true
        }
        (Some(a), Some(b), None) if ascii_alpha(a) && matches!(b, ':' | '|') => true,
        _ => false,
    }
}
