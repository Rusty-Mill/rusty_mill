//! The [`Url`] type: a parsed, always-valid URL.

use std::cmp::Ordering;
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::mem;
use std::net::IpAddr;
#[cfg(any(
    unix,
    windows,
    target_os = "redox",
    target_os = "wasi",
    target_os = "hermit"
))]
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::file_path;
use crate::form_urlencoded::EncodingOverride;
use crate::host::{Host, HostInternal};
use crate::origin::{self, Origin};
use crate::parser::{self, Parser, SchemeType, SyntaxViolation, USERINFO};
use crate::percent_encode::utf8_percent_encode;
use crate::ParseError;

/// A parsed URL record, per the [WHATWG URL Standard](https://url.spec.whatwg.org/).
///
/// Internally, a `Url` is its own serialization (a `String`) plus byte
/// offsets marking each component's boundary — so `as_str`/`Display`/
/// `into<String>` are all free, and component accessors are cheap slices.
#[derive(Clone)]
pub struct Url {
    pub(crate) serialization: String,
    pub(crate) scheme_end: u32,
    pub(crate) username_end: u32,
    pub(crate) host_start: u32,
    pub(crate) host_end: u32,
    pub(crate) host: HostInternal,
    pub(crate) port: Option<u16>,
    pub(crate) path_start: u32,
    pub(crate) query_start: Option<u32>,
    pub(crate) fragment_start: Option<u32>,
}

/// A fluent builder for [`Url::parse`], letting callers set a base URL, a
/// query-string encoding override, and/or a [`SyntaxViolation`] callback
/// before parsing. Build one with [`Url::options`].
#[derive(Copy, Clone)]
#[must_use]
pub struct ParseOptions<'a> {
    base_url: Option<&'a Url>,
    encoding_override: EncodingOverride<'a>,
    violation_fn: Option<&'a dyn Fn(SyntaxViolation)>,
}

impl<'a> ParseOptions<'a> {
    /// Resolve the parsed URL against this base URL, as in [`Url::join`].
    pub fn base_url(mut self, new: Option<&'a Url>) -> Self {
        self.base_url = new;
        self
    }

    /// Override the character encoding of the query string. A legacy
    /// concept only relevant for HTML — see [`crate::form_urlencoded`].
    pub fn encoding_override(mut self, new: EncodingOverride<'a>) -> Self {
        self.encoding_override = new;
        self
    }

    /// Call the given function for each non-fatal [`SyntaxViolation`]
    /// encountered while parsing. The URL still parses successfully;
    /// violations are purely informational.
    pub fn syntax_violation_callback(mut self, new: Option<&'a dyn Fn(SyntaxViolation)>) -> Self {
        self.violation_fn = new;
        self
    }

    /// Parse a URL string with the configuration built up so far.
    ///
    /// # Errors
    ///
    /// Same as [`Url::parse`].
    pub fn parse(self, input: &str) -> Result<Url, ParseError> {
        Parser::parse_url_full(
            input,
            self.base_url,
            self.encoding_override,
            self.violation_fn,
        )
    }
}

impl Url {
    /// Parse an absolute URL from a string.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::RelativeUrlWithoutBase`] if `input` has no
    /// scheme — use [`Url::join`] to parse a possibly-relative reference
    /// against a base URL — or another [`ParseError`] variant if `input`
    /// is malformed.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        Self::options().parse(input)
    }

    /// Parse an absolute URL from a string, then append `iter`'s pairs to
    /// its query string (existing params, if any, are kept).
    ///
    /// # Errors
    ///
    /// Same as [`Url::parse`].
    pub fn parse_with_params<I, K, V>(input: &str, iter: I) -> Result<Self, ParseError>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut url = Self::options().parse(input)?;
        url.query_pairs_mut().extend_pairs(iter);
        Ok(url)
    }

    /// A fluent builder for parsing with a base URL, a query-string
    /// encoding override, and/or a [`SyntaxViolation`] callback — see
    /// [`ParseOptions`].
    pub fn options<'a>() -> ParseOptions<'a> {
        ParseOptions {
            base_url: None,
            encoding_override: None,
            violation_fn: None,
        }
    }

    /// Parse `input`, resolving it against `self` as a base URL if `input`
    /// is a relative reference. The inverse of [`Url::make_relative`].
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::RelativeUrlWithCannotBeABaseBase`] if `self`
    /// is cannot-be-a-base, or another [`ParseError`] variant if `input`
    /// (resolved against `self`) is malformed.
    pub fn join(&self, input: &str) -> Result<Self, ParseError> {
        Parser::parse_url_with_base(input, self)
    }

    /// Make a relative URL that, when [`Url::join`]ed to `self`, yields
    /// `url`. Returns `None` if `self` is cannot-be-a-base, or if `self`
    /// and `url` don't share a scheme, host, and port (username, password,
    /// query, and fragment are not considered part of that comparison).
    pub fn make_relative(&self, url: &Self) -> Option<String> {
        if self.cannot_be_a_base() {
            return None;
        }
        if self.scheme() != url.scheme() || self.host() != url.host() || self.port() != url.port() {
            return None;
        }

        let mut relative = String::new();

        fn split_path_and_filename(s: &str) -> (&str, &str) {
            let last_slash = s.rfind('/').unwrap_or(0);
            let (path, filename) = s.split_at(last_slash);
            match filename.strip_prefix('/') {
                Some(filename) if !filename.is_empty() => (path, filename),
                _ => (path, ""),
            }
        }

        let (base_path, base_filename) = split_path_and_filename(self.path());
        let (url_path, url_filename) = split_path_and_filename(url.path());

        let mut base_segments = base_path.split('/').peekable();
        let mut url_segments = url_path.split('/').peekable();

        while base_segments.peek().is_some() && base_segments.peek() == url_segments.peek() {
            base_segments.next();
            url_segments.next();
        }

        for segment in base_segments {
            // The final empty segment (from the trailing '/' before the
            // filename) doesn't correspond to a directory to climb out of.
            if segment.is_empty() {
                break;
            }
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str("..");
        }

        for segment in url_segments {
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(segment);
        }

        if !relative.is_empty() || base_filename != url_filename {
            if url_filename.is_empty() {
                relative.push('/');
            } else {
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(url_filename);
            }
        }

        if let Some(query) = url.query() {
            relative.push('?');
            relative.push_str(query);
        }
        if let Some(fragment) = url.fragment() {
            relative.push('#');
            relative.push_str(fragment);
        }

        Some(relative)
    }

    /// Convert a file path into a `file:` URL.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `path` isn't absolute, or (on Windows) doesn't have
    /// a disk or UNC prefix.
    #[cfg(any(
        unix,
        windows,
        target_os = "redox",
        target_os = "wasi",
        target_os = "hermit"
    ))]
    #[allow(clippy::result_unit_err)]
    pub fn from_file_path<P: AsRef<Path>>(path: P) -> Result<Self, ()> {
        let mut serialization = "file://".to_owned();
        let host_start = serialization.len() as u32;
        let (host_end, host) =
            file_path::path_to_file_url_segments(path.as_ref(), &mut serialization)?;
        Ok(Self {
            serialization,
            scheme_end: "file".len() as u32,
            username_end: host_start,
            host_start,
            host_end,
            host,
            port: None,
            path_start: host_end,
            query_start: None,
            fragment_start: None,
        })
    }

    /// Convert a directory path into a `file:` URL, ensuring the result
    /// ends with a trailing slash so it can be used as a base URL for
    /// further relative paths under that directory.
    ///
    /// # Errors
    ///
    /// Same as [`Url::from_file_path`].
    #[cfg(any(
        unix,
        windows,
        target_os = "redox",
        target_os = "wasi",
        target_os = "hermit"
    ))]
    #[allow(clippy::result_unit_err)]
    pub fn from_directory_path<P: AsRef<Path>>(path: P) -> Result<Self, ()> {
        let mut url = Self::from_file_path(path)?;
        if !url.serialization.ends_with('/') {
            url.serialization.push('/');
        }
        Ok(url)
    }

    /// Convert this URL's path back to a `std::path::Path`, assuming it's a
    /// `file:` URL or similar. Does not check `self.scheme()`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the host is neither empty nor `"localhost"` (except
    /// on Windows, where `file:` URLs may have a non-local host), or if the
    /// percent-decoded path isn't representable as a `Path` on this
    /// platform.
    #[cfg(any(
        unix,
        windows,
        target_os = "redox",
        target_os = "wasi",
        target_os = "hermit"
    ))]
    #[allow(clippy::result_unit_err)]
    pub fn to_file_path(&self) -> Result<PathBuf, ()> {
        let Some(segments) = self.path_segments() else {
            return Err(());
        };
        let host = match self.host() {
            None | Some(Host::Domain("localhost")) => None,
            Some(_) if cfg!(windows) && self.scheme() == "file" => {
                Some(&self.serialization[self.host_start as usize..self.host_end as usize])
            }
            _ => return Err(()),
        };
        file_path::file_url_segments_to_pathbuf(host, segments)
    }

    /// Return the serialization of this URL.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.serialization
    }

    /// Consume this `Url`, returning its serialization.
    #[inline]
    pub fn into_string(self) -> String {
        self.serialization
    }

    /// Return the scheme, lowercased, without the trailing `:`.
    #[inline]
    pub fn scheme(&self) -> &str {
        &self.serialization[..self.scheme_end as usize]
    }

    /// Return whether the scheme is one of the WHATWG-defined special
    /// schemes (`http`, `https`, `ws`, `wss`, `ftp`, `file`).
    pub fn is_special(&self) -> bool {
        SchemeType::from(self.scheme()).is_special()
    }

    /// Return this URL's [origin](https://url.spec.whatwg.org/#origin).
    pub fn origin(&self) -> Origin {
        origin::url_origin(self)
    }

    /// Return whether the URL has an authority (`//` followed by an
    /// optional userinfo, a host, and an optional port).
    #[inline]
    pub fn has_authority(&self) -> bool {
        self.serialization[self.scheme_end as usize..].starts_with("://")
    }

    /// Return the `user:password@host:port` authority component, or `""`
    /// if the URL has no authority.
    pub fn authority(&self) -> &str {
        let sep = "://".len() as u32;
        if self.has_authority() && self.path_start > self.scheme_end + sep {
            &self.serialization[(self.scheme_end + sep) as usize..self.path_start as usize]
        } else {
            ""
        }
    }

    /// Return whether this is a "cannot-be-a-base" URL — one whose scheme
    /// is not followed by a `/`, such as `data:` or `mailto:` URLs. A
    /// relative reference cannot be resolved against a cannot-be-a-base
    /// base URL.
    #[inline]
    pub fn cannot_be_a_base(&self) -> bool {
        !self.serialization[self.scheme_end as usize + 1..].starts_with('/')
    }

    /// Return the percent-encoded username (the empty string if none).
    pub fn username(&self) -> &str {
        let sep = "://".len() as u32;
        if self.has_authority() && self.username_end > self.scheme_end + sep {
            &self.serialization[(self.scheme_end + sep) as usize..self.username_end as usize]
        } else {
            ""
        }
    }

    /// Return the percent-encoded password, if any.
    pub fn password(&self) -> Option<&str> {
        if self.has_authority()
            && self.username_end != self.serialization.len() as u32
            && self.serialization.as_bytes()[self.username_end as usize] == b':'
        {
            Some(&self.serialization[self.username_end as usize + 1..self.host_start as usize - 1])
        } else {
            None
        }
    }

    /// Equivalent to `url.host().is_some()`.
    pub fn has_host(&self) -> bool {
        !matches!(self.host, HostInternal::None)
    }

    /// Return the host (domain or IP address) as a string, if any.
    pub fn host_str(&self) -> Option<&str> {
        self.has_host()
            .then(|| &self.serialization[self.host_start as usize..self.host_end as usize])
    }

    /// Return the parsed [`Host`], if any.
    pub fn host(&self) -> Option<Host<&str>> {
        match self.host {
            HostInternal::None => None,
            HostInternal::Domain => Some(Host::Domain(
                &self.serialization[self.host_start as usize..self.host_end as usize],
            )),
            HostInternal::Ipv4(addr) => Some(Host::Ipv4(addr)),
            HostInternal::Ipv6(addr) => Some(Host::Ipv6(addr)),
        }
    }

    /// Return the host as a domain name, if it has a host and that host is
    /// a domain (not an IP address).
    pub fn domain(&self) -> Option<&str> {
        matches!(self.host, HostInternal::Domain)
            .then(|| &self.serialization[self.host_start as usize..self.host_end as usize])
    }

    /// Return the port, if it was explicit and differs from the scheme's
    /// default (a matching default port is never stored — see
    /// [`Url::port_or_known_default`]).
    #[inline]
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Return the port, falling back to the scheme's default port if known
    /// (`http`, `https`, `ws`, `wss`, `ftp`).
    #[inline]
    pub fn port_or_known_default(&self) -> Option<u16> {
        self.port.or_else(|| parser::default_port(self.scheme()))
    }

    /// Resolve this URL's host and port to one or more [`SocketAddr`]s,
    /// suitable for `TcpStream::connect`.
    ///
    /// A domain host is resolved via the standard library's DNS support. If
    /// the URL's scheme has no [known default port](Url::port_or_known_default)
    /// and it doesn't specify one explicitly, `default_port_number` is
    /// called to supply one (e.g. `|| None`, or matching on `self.scheme()`
    /// for an application-specific scheme).
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`](std::io::Error) if the URL has no host, has
    /// no resolvable port, or (for a domain host) if DNS resolution fails.
    #[cfg(any(
        unix,
        windows,
        target_os = "redox",
        target_os = "wasi",
        target_os = "hermit"
    ))]
    pub fn socket_addrs(
        &self,
        default_port_number: impl Fn() -> Option<u16>,
    ) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use std::net::ToSocketAddrs;

        fn io_result<T>(opt: Option<T>, message: &str) -> std::io::Result<T> {
            opt.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, message))
        }

        let host = io_result(self.host(), "No host name in the URL")?;
        let port = io_result(
            self.port_or_known_default().or_else(default_port_number),
            "No port number in the URL",
        )?;
        Ok(match host {
            Host::Domain(domain) => (domain, port).to_socket_addrs()?.collect(),
            Host::Ipv4(ip) => vec![(ip, port).into()],
            Host::Ipv6(ip) => vec![(ip, port).into()],
        })
    }

    /// Return the path.
    pub fn path(&self) -> &str {
        match (self.query_start, self.fragment_start) {
            (None, None) => &self.serialization[self.path_start as usize..],
            (Some(end), _) | (None, Some(end)) => {
                &self.serialization[self.path_start as usize..end as usize]
            }
        }
    }

    /// Unless this URL is cannot-be-a-base, return an iterator of `/`-
    /// separated path segments, each a percent-encoded ASCII string.
    ///
    /// When `Some` is returned, the iterator always yields at least one
    /// string (possibly empty).
    pub fn path_segments(&self) -> Option<std::str::Split<'_, char>> {
        self.path().strip_prefix('/').map(|rest| rest.split('/'))
    }

    /// Return a [`PathSegmentsMut`] for editing this URL's path as a
    /// sequence of segments. Fails if this URL is cannot-be-a-base.
    #[allow(clippy::result_unit_err)]
    pub fn path_segments_mut(&mut self) -> Result<crate::PathSegmentsMut<'_>, ()> {
        if self.cannot_be_a_base() {
            Err(())
        } else {
            Ok(crate::path_segments::new(self))
        }
    }

    /// Return the query string (without the leading `?`), if any.
    pub fn query(&self) -> Option<&str> {
        let query_start = self.query_start?;
        let start = query_start as usize + 1;
        Some(match self.fragment_start {
            Some(end) => &self.serialization[start..end as usize],
            None => &self.serialization[start..],
        })
    }

    /// Parse this URL's query string as `application/x-www-form-urlencoded`
    /// and return an iterator of `(key, value)` pairs.
    #[inline]
    pub fn query_pairs(&self) -> crate::form_urlencoded::Parse<'_> {
        crate::form_urlencoded::parse(self.query().unwrap_or("").as_bytes())
    }

    /// Manipulate this URL's query string as a sequence of
    /// `application/x-www-form-urlencoded` name/value pairs. Method-chains
    /// like a normal [`form_urlencoded::Serializer`](crate::form_urlencoded::Serializer):
    ///
    /// ```
    /// # use rusty_url::Url;
    /// let mut url = Url::parse("https://example.net?lang=fr#nav").unwrap();
    /// url.query_pairs_mut().append_pair("foo", "bar");
    /// assert_eq!(url.query(), Some("lang=fr&foo=bar"));
    /// ```
    ///
    /// `url.query_pairs_mut().clear()` is equivalent to
    /// `url.set_query(Some(""))`, not `url.set_query(None)`.
    pub fn query_pairs_mut(&mut self) -> crate::form_urlencoded::Serializer<'_, UrlQuery<'_>> {
        let fragment = self.take_fragment();

        let query_start = match self.query_start {
            Some(start) => start as usize,
            None => {
                let start = self.serialization.len();
                self.query_start = Some(start as u32);
                self.serialization.push('?');
                start
            }
        };

        let query = UrlQuery {
            url: Some(self),
            fragment,
        };
        crate::form_urlencoded::Serializer::for_suffix(query, query_start + "?".len())
    }

    /// Return the fragment (without the leading `#`), if any.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment_start
            .map(|start| &self.serialization[start as usize + 1..])
    }

    /// Change this URL's scheme.
    ///
    /// Fails (leaving the URL unchanged) if: `scheme` isn't a valid scheme
    /// (`[a-zA-Z][a-zA-Z0-9+.-]*`); switching between a special and a
    /// non-special scheme; switching to `file` while the URL has
    /// credentials or a port; or switching to a special scheme on a URL
    /// with no host.
    #[allow(clippy::result_unit_err)]
    pub fn set_scheme(&mut self, scheme: &str) -> Result<(), ()> {
        let mut parser = Parser::for_setter(String::new());
        let remaining = parser.parse_scheme(parser::Input::new_no_trim(scheme))?;
        let new_scheme_type = SchemeType::from(parser.serialization.as_str());
        let old_scheme_type = SchemeType::from(self.scheme());
        if (new_scheme_type.is_special() && !old_scheme_type.is_special())
            || (!new_scheme_type.is_special() && old_scheme_type.is_special())
            || (new_scheme_type.is_file() && self.has_authority())
        {
            return Err(());
        }
        if !remaining.is_empty() || (!self.has_host() && new_scheme_type.is_special()) {
            return Err(());
        }

        let old_scheme_end = self.scheme_end;
        let new_scheme_end = parser.serialization.len() as u32;
        let adjust = |index: &mut u32| {
            *index = *index - old_scheme_end + new_scheme_end;
        };
        self.scheme_end = new_scheme_end;
        adjust(&mut self.username_end);
        adjust(&mut self.host_start);
        adjust(&mut self.host_end);
        adjust(&mut self.path_start);
        if let Some(ref mut index) = self.query_start {
            adjust(index);
        }
        if let Some(ref mut index) = self.fragment_start {
            adjust(index);
        }

        parser
            .serialization
            .push_str(&self.serialization[old_scheme_end as usize..]);
        self.serialization = parser.serialization;

        // A scheme change can make a stored port equal to the new scheme's
        // default (or stop being equal to the old one's) — re-normalize.
        let previous_port = self.port();
        let _ = self.set_port(previous_port);

        Ok(())
    }

    /// Change this URL's username. Percent-encodes `username` as needed.
    ///
    /// Fails if this URL is cannot-be-a-base, has no host, or is a `file:`
    /// URL (none of which have credentials).
    #[allow(clippy::result_unit_err)]
    pub fn set_username(&mut self, username: &str) -> Result<(), ()> {
        if !self.has_host() || self.host() == Some(Host::Domain("")) || self.scheme() == "file" {
            return Err(());
        }
        let username_start = self.scheme_end + 3;
        if &self.serialization[username_start as usize..self.username_end as usize] == username {
            return Ok(());
        }
        let after_username = self.serialization[self.username_end as usize..].to_owned();
        self.serialization.truncate(username_start as usize);
        self.serialization
            .push_str(&utf8_percent_encode(username, USERINFO));

        let mut removed_bytes = self.username_end;
        self.username_end = self.serialization.len() as u32;
        let mut added_bytes = self.username_end;

        let new_username_is_empty = self.username_end == username_start;
        match (new_username_is_empty, after_username.chars().next()) {
            (true, Some('@')) => {
                removed_bytes += 1;
                self.serialization.push_str(&after_username[1..]);
            }
            (false, Some('@')) | (_, Some(':')) | (true, _) => {
                self.serialization.push_str(&after_username);
            }
            (false, _) => {
                added_bytes += 1;
                self.serialization.push('@');
                self.serialization.push_str(&after_username);
            }
        }

        let adjust = |index: &mut u32| {
            *index = *index - removed_bytes + added_bytes;
        };
        adjust(&mut self.host_start);
        adjust(&mut self.host_end);
        adjust(&mut self.path_start);
        if let Some(ref mut index) = self.query_start {
            adjust(index);
        }
        if let Some(ref mut index) = self.fragment_start {
            adjust(index);
        }
        Ok(())
    }

    /// Change this URL's password, or remove it (`None`). Percent-encodes
    /// the password as needed.
    ///
    /// Fails if this URL is cannot-be-a-base, has no host, or is a `file:`
    /// URL.
    #[allow(clippy::result_unit_err)]
    pub fn set_password(&mut self, password: Option<&str>) -> Result<(), ()> {
        if !self.has_host() || self.host() == Some(Host::Domain("")) || self.scheme() == "file" {
            return Err(());
        }
        let password = password.unwrap_or_default();
        if !password.is_empty() {
            let host_and_after = self.serialization[self.host_start as usize..].to_owned();
            self.serialization.truncate(self.username_end as usize);
            self.serialization.push(':');
            self.serialization
                .push_str(&utf8_percent_encode(password, USERINFO));
            self.serialization.push('@');

            let old_host_start = self.host_start;
            let new_host_start = self.serialization.len() as u32;
            let adjust = |index: &mut u32| {
                *index = *index - old_host_start + new_host_start;
            };
            self.host_start = new_host_start;
            adjust(&mut self.host_end);
            adjust(&mut self.path_start);
            if let Some(ref mut index) = self.query_start {
                adjust(index);
            }
            if let Some(ref mut index) = self.fragment_start {
                adjust(index);
            }

            self.serialization.push_str(&host_and_after);
        } else if self.byte_at(self.username_end) == b':' {
            // There is a password to remove.
            let username_start = self.scheme_end + 3;
            let empty_username = username_start == self.username_end;
            let start = self.username_end;
            let end = if empty_username {
                self.host_start // also remove the separating '@'
            } else {
                self.host_start - 1 // keep '@' separating username from host
            };
            self.serialization.drain(start as usize..end as usize);
            let offset = end - start;
            self.host_start -= offset;
            self.host_end -= offset;
            self.path_start -= offset;
            if let Some(ref mut index) = self.query_start {
                *index -= offset;
            }
            if let Some(ref mut index) = self.fragment_start {
                *index -= offset;
            }
        }
        Ok(())
    }

    /// Change this URL's host.
    ///
    /// Passing `None` removes the host, and also removes any username,
    /// password, and port. Fails if this URL is cannot-be-a-base, if
    /// `host` fails to parse, or if removing the host on a special
    /// (non-`file`) scheme, since those always require a host.
    pub fn set_host(&mut self, host: Option<&str>) -> Result<(), ParseError> {
        if self.cannot_be_a_base() {
            return Err(ParseError::SetHostOnCannotBeABaseUrl);
        }
        let scheme_type = SchemeType::from(self.scheme());

        if let Some(host) = host {
            if host.is_empty() && scheme_type.is_special() && !scheme_type.is_file() {
                return Err(ParseError::EmptyHost);
            }
            let mut host_substr = host;
            if !host.starts_with('[') || !host.ends_with(']') {
                match host.find(':') {
                    Some(0) => return Err(ParseError::InvalidDomainCharacter),
                    Some(colon_index) => host_substr = &host[..colon_index],
                    None => {}
                }
            }
            let parsed = if scheme_type.is_special() {
                Host::parse(host_substr)?
            } else {
                Host::parse_opaque(host_substr)?
            };
            self.set_host_internal(parsed, None);
        } else if self.has_host() {
            if scheme_type.is_special() && !scheme_type.is_file() {
                return Err(ParseError::EmptyHost);
            } else if self.serialization.len() == self.path_start as usize {
                self.serialization.push('/');
            }

            let new_path_start = if scheme_type.is_file() {
                self.scheme_end + 3
            } else {
                self.scheme_end + 1
            };
            self.serialization
                .drain(new_path_start as usize..self.path_start as usize);
            let offset = self.path_start - new_path_start;
            self.path_start = new_path_start;
            self.username_end = new_path_start;
            self.host_start = new_path_start;
            self.host_end = new_path_start;
            // Deliberately not reset to `HostInternal::None` here, matching
            // the reference crate: this leaves an empty-domain host, so
            // `has_host()` stays `true` and `host_str()` returns `Some("")`
            // rather than `None` after removing a host this way.
            self.port = None;
            if let Some(ref mut index) = self.query_start {
                *index -= offset;
            }
            if let Some(ref mut index) = self.fragment_start {
                *index -= offset;
            }
        }
        Ok(())
    }

    /// `opt_new_port`: `None` leaves the port unchanged, `Some(None)` removes it.
    fn set_host_internal(&mut self, host: Host<String>, opt_new_port: Option<Option<u16>>) {
        let old_suffix_pos = if opt_new_port.is_some() {
            self.path_start
        } else {
            self.host_end
        };
        let suffix = self.serialization[old_suffix_pos as usize..].to_owned();
        self.serialization.truncate(self.host_start as usize);
        if !self.has_authority() {
            self.serialization.push('/');
            self.serialization.push('/');
            self.username_end += 2;
            self.host_start += 2;
        }
        write!(&mut self.serialization, "{host}").unwrap();
        self.host_end = self.serialization.len() as u32;
        self.host = host.into();

        if let Some(new_port) = opt_new_port {
            self.port = new_port;
            if let Some(port) = new_port {
                write!(&mut self.serialization, ":{port}").unwrap();
            }
        }
        let new_suffix_pos = self.serialization.len() as u32;
        self.serialization.push_str(&suffix);

        let adjust = |index: &mut u32| {
            *index = *index - old_suffix_pos + new_suffix_pos;
        };
        adjust(&mut self.path_start);
        if let Some(ref mut index) = self.query_start {
            adjust(index);
        }
        if let Some(ref mut index) = self.fragment_start {
            adjust(index);
        }
    }

    /// Change this URL's host to an IP address directly, skipping the host
    /// parser. Fails if this URL is cannot-be-a-base.
    #[allow(clippy::result_unit_err)]
    pub fn set_ip_host(&mut self, address: IpAddr) -> Result<(), ()> {
        if self.cannot_be_a_base() {
            return Err(());
        }
        let host = match address {
            IpAddr::V4(addr) => Host::Ipv4(addr),
            IpAddr::V6(addr) => Host::Ipv6(addr),
        };
        self.set_host_internal(host, None);
        Ok(())
    }

    /// Change this URL's port, or remove it (`None`). A port equal to the
    /// scheme's default is normalized away, matching [`Url::port`]. Fails
    /// if the URL has no host, has an empty host, or is a `file:` URL.
    #[allow(clippy::result_unit_err)]
    pub fn set_port(&mut self, mut port: Option<u16>) -> Result<(), ()> {
        if !self.has_host() || self.host() == Some(Host::Domain("")) || self.scheme() == "file" {
            return Err(());
        }
        if port.is_some() && port == parser::default_port(self.scheme()) {
            port = None;
        }
        self.set_port_internal(port);
        Ok(())
    }

    fn set_port_internal(&mut self, port: Option<u16>) {
        match (self.port, port) {
            (None, None) => {}
            (Some(_), None) => {
                self.serialization
                    .drain(self.host_end as usize..self.path_start as usize);
                let offset = self.path_start - self.host_end;
                self.path_start = self.host_end;
                if let Some(ref mut index) = self.query_start {
                    *index -= offset;
                }
                if let Some(ref mut index) = self.fragment_start {
                    *index -= offset;
                }
            }
            (Some(old), Some(new)) if old == new => {}
            (_, Some(new)) => {
                let path_and_after = self.serialization[self.path_start as usize..].to_owned();
                self.serialization.truncate(self.host_end as usize);
                write!(&mut self.serialization, ":{new}").unwrap();
                let old_path_start = self.path_start;
                let new_path_start = self.serialization.len() as u32;
                self.path_start = new_path_start;
                let adjust = |index: &mut u32| {
                    *index = *index - old_path_start + new_path_start;
                };
                if let Some(ref mut index) = self.query_start {
                    adjust(index);
                }
                if let Some(ref mut index) = self.fragment_start {
                    adjust(index);
                }
                self.serialization.push_str(&path_and_after);
            }
        }
        self.port = port;
    }

    /// Change this URL's path. Percent-encodes `path` as needed (without
    /// double-encoding anything already percent-encoded).
    pub fn set_path(&mut self, mut path: &str) {
        let after_path = self.take_after_path();
        let old_after_path_pos = self.serialization.len() as u32;
        let cannot_be_a_base = self.cannot_be_a_base();
        let scheme_type = SchemeType::from(self.scheme());
        self.serialization.truncate(self.path_start as usize);
        self.mutate(|parser| {
            if cannot_be_a_base {
                if let Some(stripped) = path.strip_prefix('/') {
                    parser.serialization.push_str("%2F");
                    path = stripped;
                }
                parser.parse_cannot_be_a_base_path(parser::Input::new_no_trim(path));
            } else {
                let mut has_host = true;
                parser.parse_path_start(
                    scheme_type,
                    &mut has_host,
                    parser::Input::new_no_trim(path),
                );
            }
        });
        self.restore_after_path(old_after_path_pos, &after_path);
    }

    pub(crate) fn take_after_path(&mut self) -> String {
        match (self.query_start, self.fragment_start) {
            (Some(i), _) | (None, Some(i)) => {
                let after_path = self.serialization[i as usize..].to_owned();
                self.serialization.truncate(i as usize);
                after_path
            }
            (None, None) => String::new(),
        }
    }

    pub(crate) fn restore_after_path(&mut self, old_after_path_position: u32, after_path: &str) {
        let new_after_path_position = self.serialization.len() as u32;
        let adjust = |index: &mut u32| {
            *index = *index - old_after_path_position + new_after_path_position;
        };
        if let Some(ref mut index) = self.query_start {
            adjust(index);
        }
        if let Some(ref mut index) = self.fragment_start {
            adjust(index);
        }
        self.serialization.push_str(after_path);
    }

    /// Change this URL's query string, or remove it (`None`).
    pub fn set_query(&mut self, query: Option<&str>) {
        let fragment = self.take_fragment();

        if let Some(start) = self.query_start.take() {
            self.serialization.truncate(start as usize);
        }
        if let Some(input) = query {
            self.query_start = Some(self.serialization.len() as u32);
            self.serialization.push('?');
            let scheme_type = SchemeType::from(self.scheme());
            let scheme_end = self.scheme_end;
            self.mutate(|parser| {
                parser.parse_query(scheme_type, scheme_end, parser::Input::new_no_trim(input))
            });
        } else {
            self.query_start = None;
            if fragment.is_none() {
                self.strip_trailing_spaces_from_opaque_path();
            }
        }

        self.restore_already_parsed_fragment(fragment);
    }

    /// Change this URL's fragment, or remove it (`None`).
    pub fn set_fragment(&mut self, fragment: Option<&str>) {
        if let Some(start) = self.fragment_start {
            self.serialization.truncate(start as usize);
        }
        if let Some(input) = fragment {
            self.fragment_start = Some(self.serialization.len() as u32);
            self.serialization.push('#');
            self.mutate(|parser| parser.parse_fragment(parser::Input::new_no_trim(input)));
        } else {
            self.fragment_start = None;
            self.strip_trailing_spaces_from_opaque_path();
        }
    }

    fn take_fragment(&mut self) -> Option<String> {
        self.fragment_start.take().map(|start| {
            let fragment = self.serialization[start as usize + 1..].to_owned();
            self.serialization.truncate(start as usize);
            fragment
        })
    }

    fn restore_already_parsed_fragment(&mut self, fragment: Option<String>) {
        if let Some(fragment) = fragment {
            debug_assert!(self.fragment_start.is_none());
            self.fragment_start = Some(self.serialization.len() as u32);
            self.serialization.push('#');
            self.serialization.push_str(&fragment);
        }
    }

    /// <https://url.spec.whatwg.org/#potentially-strip-trailing-spaces-from-an-opaque-path>
    fn strip_trailing_spaces_from_opaque_path(&mut self) {
        if !self.cannot_be_a_base() || self.fragment_start.is_some() || self.query_start.is_some() {
            return;
        }
        let trailing_spaces = self
            .serialization
            .chars()
            .rev()
            .take_while(|&c| c == ' ')
            .count();
        let start = self.serialization.len() - trailing_spaces;
        self.serialization.truncate(start);
    }

    pub(crate) fn mutate<F: FnOnce(&mut Parser) -> R, R>(&mut self, f: F) -> R {
        let mut parser = Parser::for_setter(mem::take(&mut self.serialization));
        let result = f(&mut parser);
        self.serialization = parser.serialization;
        result
    }

    pub(crate) fn byte_at(&self, i: u32) -> u8 {
        self.serialization.as_bytes()[i as usize]
    }
}

/// The [`crate::form_urlencoded::Target`] backing [`Url::query_pairs_mut`].
///
/// Not constructible outside this crate — it only exists as the target type
/// parameter of the `Serializer` that `query_pairs_mut` returns.
#[derive(Debug)]
pub struct UrlQuery<'a> {
    url: Option<&'a mut Url>,
    fragment: Option<String>,
}

impl<'a> crate::form_urlencoded::Target for UrlQuery<'a> {
    type Finished = &'a mut Url;

    fn as_mut_string(&mut self) -> &mut String {
        &mut self.url.as_mut().unwrap().serialization
    }

    fn finish(mut self) -> &'a mut Url {
        let url = self.url.take().unwrap();
        url.restore_already_parsed_fragment(self.fragment.take());
        url
    }
}

impl Drop for UrlQuery<'_> {
    fn drop(&mut self) {
        if let Some(url) = self.url.take() {
            url.restore_already_parsed_fragment(self.fragment.take());
        }
    }
}

impl fmt::Display for Url {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.serialization)
    }
}

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Url")
            .field("scheme", &self.scheme())
            .field("cannot_be_a_base", &self.cannot_be_a_base())
            .field("username", &self.username())
            .field("password", &self.password())
            .field("host", &self.host())
            .field("port", &self.port())
            .field("path", &self.path())
            .field("query", &self.query())
            .field("fragment", &self.fragment())
            .finish()
    }
}

impl From<Url> for String {
    fn from(url: Url) -> Self {
        url.serialization
    }
}

impl Eq for Url {}

impl PartialEq for Url {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.serialization == other.serialization
    }
}

impl Ord for Url {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.serialization.cmp(&other.serialization)
    }
}

impl PartialOrd for Url {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Url {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.serialization.hash(state);
    }
}

impl AsRef<str> for Url {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.serialization
    }
}

impl FromStr for Url {
    type Err = ParseError;

    #[inline]
    fn from_str(input: &str) -> Result<Self, ParseError> {
        Self::parse(input)
    }
}

impl<'a> TryFrom<&'a str> for Url {
    type Error = ParseError;

    #[inline]
    fn try_from(input: &'a str) -> Result<Self, ParseError> {
        Self::parse(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_as_str() {
        let s = "https://example.net/";
        assert_eq!(Url::parse(s).unwrap().as_str(), s);
    }

    #[test]
    fn scheme_is_lowercased() {
        assert_eq!(Url::parse("HTTP://example.com/").unwrap().scheme(), "http");
    }

    #[test]
    fn is_special_by_scheme() {
        assert!(Url::parse("http://x/").unwrap().is_special());
        assert!(Url::parse("file:///x").unwrap().is_special());
        assert!(!Url::parse("moz://x/").unwrap().is_special());
    }

    #[test]
    fn has_authority_and_authority_string() {
        let url = Url::parse("https://user:pass@example.com:8080/path").unwrap();
        assert!(url.has_authority());
        assert_eq!(url.authority(), "user:pass@example.com:8080");

        let url = Url::parse("unix:/run/foo.socket").unwrap();
        assert!(!url.has_authority());
        assert_eq!(url.authority(), "");
    }

    #[test]
    fn cannot_be_a_base_for_opaque_schemes() {
        assert!(Url::parse("data:text/plain,Stuff")
            .unwrap()
            .cannot_be_a_base());
        assert!(!Url::parse("ftp://rms@example.com")
            .unwrap()
            .cannot_be_a_base());
        assert!(!Url::parse("unix:/run/foo.socket")
            .unwrap()
            .cannot_be_a_base());
    }

    #[test]
    fn username_and_password() {
        let url = Url::parse("ftp://rms:secret@example.com").unwrap();
        assert_eq!(url.username(), "rms");
        assert_eq!(url.password(), Some("secret"));

        let url = Url::parse("ftp://:secret@example.com").unwrap();
        assert_eq!(url.username(), "");
        assert_eq!(url.password(), Some("secret"));

        let url = Url::parse("https://example.com").unwrap();
        assert_eq!(url.username(), "");
        assert_eq!(url.password(), None);
    }

    #[test]
    fn host_domain_and_ip() {
        let url = Url::parse("https://Example.COM/").unwrap();
        assert!(url.has_host());
        assert_eq!(url.host_str(), Some("example.com"));
        assert_eq!(url.domain(), Some("example.com"));

        let url = Url::parse("https://127.0.0.1/").unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.domain(), None);

        let url = Url::parse("data:text/plain,x").unwrap();
        assert!(!url.has_host());
        assert_eq!(url.host_str(), None);
    }

    #[test]
    fn port_and_known_default() {
        let url = Url::parse("https://example.com").unwrap();
        assert_eq!(url.port(), None);
        assert_eq!(url.port_or_known_default(), Some(443));

        let url = Url::parse("https://example.com:443/").unwrap();
        assert_eq!(url.port(), None, "default port is not stored");

        let url = Url::parse("ssh://example.com:22").unwrap();
        assert_eq!(url.port(), Some(22));

        let url = Url::parse("foo://example.com:1456").unwrap();
        assert_eq!(url.port_or_known_default(), Some(1456));
        let url = Url::parse("foo://example.com").unwrap();
        assert_eq!(url.port_or_known_default(), None);
    }

    #[test]
    fn path_query_fragment() {
        let url = Url::parse("https://example.com/a/b?x=1&y=2#frag").unwrap();
        assert_eq!(url.path(), "/a/b");
        assert_eq!(url.query(), Some("x=1&y=2"));
        assert_eq!(url.fragment(), Some("frag"));

        let url = Url::parse("https://example.com").unwrap();
        assert_eq!(url.path(), "/");
        assert_eq!(url.query(), None);
        assert_eq!(url.fragment(), None);
    }

    #[test]
    fn dot_segments_are_normalized() {
        let url = Url::parse("http://example.com/a/b/../c").unwrap();
        assert_eq!(url.path(), "/a/c");

        let url = Url::parse("http://example.com/a/./b").unwrap();
        assert_eq!(url.path(), "/a/b");

        let url = Url::parse("http://example.com/../../a").unwrap();
        assert_eq!(url.path(), "/a");
    }

    #[test]
    fn percent_encodes_reserved_and_non_ascii() {
        let url = Url::parse("https://example.com/a b?<c>#<d>").unwrap();
        assert_eq!(url.path(), "/a%20b");
        assert_eq!(url.query(), Some("%3Cc%3E"));
        assert_eq!(url.fragment(), Some("%3Cd%3E"));
    }

    #[test]
    fn userinfo_is_percent_encoded() {
        let url = Url::parse("https://user name:p@ss@example.com/").unwrap();
        assert_eq!(url.username(), "user%20name");
        assert_eq!(url.password(), Some("p%40ss"));
    }

    #[test]
    fn idna_domain_round_trips_to_punycode() {
        let url = Url::parse("https://bücher.example/").unwrap();
        assert_eq!(url.host_str(), Some("xn--bcher-kva.example"));
    }

    #[test]
    fn ipv6_host_is_bracketed() {
        let url = Url::parse("http://[::1]:8080/").unwrap();
        assert_eq!(url.host_str(), Some("[::1]"));
        assert_eq!(url.port(), Some(8080));
    }

    #[test]
    fn file_url_without_host() {
        let url = Url::parse("file:///tmp/foo").unwrap();
        assert_eq!(url.scheme(), "file");
        assert!(!url.has_host());
        assert_eq!(url.path(), "/tmp/foo");
    }

    #[test]
    fn file_url_windows_drive_letter() {
        let url = Url::parse("file:///C:/Users/").unwrap();
        assert_eq!(url.path(), "/C:/Users/");
    }

    #[test]
    fn empty_host_on_special_scheme_is_an_error() {
        // Leading slashes beyond "//" are consumed as part of the authority
        // marker (`http:///path` == `http://path/`, host "path"), so this
        // needs an authority that is present but truly empty.
        assert_eq!(Url::parse("http://"), Err(ParseError::EmptyHost));
        assert_eq!(Url::parse("http://@/"), Err(ParseError::EmptyHost));
    }

    #[test]
    fn extra_leading_slashes_become_part_of_the_authority() {
        // Matches the reference `url` crate: all of "///" after "http:" is
        // treated as the authority-slashes marker, so "path" is the host.
        let url = Url::parse("http:///path").unwrap();
        assert_eq!(url.host_str(), Some("path"));
        assert_eq!(url.path(), "/");
    }

    #[test]
    fn invalid_port_is_an_error() {
        assert_eq!(
            Url::parse("http://example.com:notaport/"),
            Err(ParseError::InvalidPort)
        );
        assert_eq!(
            Url::parse("http://example.com:99999/"),
            Err(ParseError::InvalidPort)
        );
    }

    #[test]
    fn relative_reference_without_scheme_is_an_error() {
        assert_eq!(
            Url::parse("/just/a/path"),
            Err(ParseError::RelativeUrlWithoutBase)
        );
    }

    #[test]
    fn equality_ordering_and_hash_follow_serialization() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher as _;

        let a = Url::parse("https://a.example/").unwrap();
        let b = Url::parse("https://b.example/").unwrap();
        let a2 = Url::parse("https://a.example/").unwrap();
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert!(a < b);

        let mut ha = DefaultHasher::new();
        a.hash(&mut ha);
        let mut ha2 = DefaultHasher::new();
        a2.hash(&mut ha2);
        assert_eq!(ha.finish(), ha2.finish());
    }

    #[test]
    fn from_str_and_try_from_and_into_string() {
        let url: Url = "https://example.com/".parse().unwrap();
        assert_eq!(url.as_str(), "https://example.com/");

        let url = Url::try_from("https://example.com/").unwrap();
        let s: String = url.into();
        assert_eq!(s, "https://example.com/");
    }

    #[test]
    fn debug_includes_components() {
        let url = Url::parse("https://user@example.com/path?q#f").unwrap();
        let debug = format!("{url:?}");
        assert!(debug.contains("example.com"));
        assert!(debug.contains("/path"));
    }

    #[test]
    fn set_fragment_add_change_remove() {
        let mut url = Url::parse("https://example.com/data.csv").unwrap();
        url.set_fragment(Some("cell=4,1-6,2"));
        assert_eq!(url.as_str(), "https://example.com/data.csv#cell=4,1-6,2");
        assert_eq!(url.fragment(), Some("cell=4,1-6,2"));
        url.set_fragment(None);
        assert_eq!(url.as_str(), "https://example.com/data.csv");
        assert!(url.fragment().is_none());
    }

    #[test]
    fn set_query_add_change_remove() {
        let mut url = Url::parse("https://example.com/products").unwrap();
        url.set_query(Some("page=2"));
        assert_eq!(url.as_str(), "https://example.com/products?page=2");
        assert_eq!(url.query(), Some("page=2"));
        url.set_query(None);
        assert_eq!(url.as_str(), "https://example.com/products");
    }

    #[test]
    fn set_query_preserves_fragment() {
        let mut url = Url::parse("https://example.net?lang=fr#nav").unwrap();
        url.set_query(Some("lang=en"));
        assert_eq!(url.as_str(), "https://example.net/?lang=en#nav");
    }

    #[test]
    fn set_query_hash_is_literal_not_a_fragment_separator() {
        let mut url = Url::parse("https://host/oldpath").unwrap();
        url.set_query(Some("a#b"));
        assert_eq!(url.query(), Some("a%23b"));
        assert_eq!(url.fragment(), None);
    }

    #[test]
    fn set_path_basic_and_percent_encoding() {
        let mut url = Url::parse("https://example.com").unwrap();
        url.set_path("api/comments");
        assert_eq!(url.as_str(), "https://example.com/api/comments");

        let mut url = Url::parse("https://example.com").unwrap();
        url.set_path("api/some comments");
        assert_eq!(url.path(), "/api/some%20comments");

        let mut url = Url::parse("https://example.com").unwrap();
        url.set_path("api/some%20comments");
        assert_eq!(url.path(), "/api/some%20comments");
    }

    #[test]
    fn set_path_question_and_hash_are_literal_not_terminators() {
        let mut url = Url::parse("https://host/oldpath").unwrap();
        url.set_path("a?b#c");
        assert_eq!(url.path(), "/a%3Fb%23c");
    }

    #[test]
    fn set_port_change_and_remove() {
        let mut url = Url::parse("ssh://example.net:2048/").unwrap();
        url.set_port(Some(4096)).unwrap();
        assert_eq!(url.as_str(), "ssh://example.net:4096/");
        url.set_port(None).unwrap();
        assert_eq!(url.as_str(), "ssh://example.net/");
    }

    #[test]
    fn set_port_to_known_default_is_not_stored() {
        let mut url = Url::parse("https://example.org/").unwrap();
        url.set_port(Some(443)).unwrap();
        assert!(url.port().is_none());
    }

    #[test]
    fn set_port_fails_for_cannot_be_a_base() {
        let mut url = Url::parse("mailto:rms@example.net").unwrap();
        assert!(url.set_port(Some(80)).is_err());
        assert!(url.set_port(None).is_err());
    }

    #[test]
    fn set_host_change_and_remove() {
        let mut url = Url::parse("https://example.net").unwrap();
        url.set_host(Some("rust-lang.org")).unwrap();
        assert_eq!(url.as_str(), "https://rust-lang.org/");

        let mut url = Url::parse("foo://example.net").unwrap();
        url.set_host(None).unwrap();
        assert_eq!(url.as_str(), "foo:/");
    }

    #[test]
    fn set_host_none_fails_for_special_scheme() {
        let mut url = Url::parse("https://example.net").unwrap();
        assert!(url.set_host(None).is_err());
        assert_eq!(url.as_str(), "https://example.net/");
    }

    #[test]
    fn set_host_fails_for_cannot_be_a_base() {
        let mut url = Url::parse("mailto:rms@example.net").unwrap();
        assert!(url.set_host(Some("rust-lang.org")).is_err());
        assert!(url.set_host(None).is_err());
    }

    #[test]
    fn set_ip_host() {
        let mut url = Url::parse("http://example.com").unwrap();
        url.set_ip_host("127.0.0.1".parse().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.as_str(), "http://127.0.0.1/");
    }

    #[test]
    fn set_ip_host_fails_for_cannot_be_a_base() {
        let mut url = Url::parse("mailto:rms@example.com").unwrap();
        assert!(url.set_ip_host("127.0.0.1".parse().unwrap()).is_err());
    }

    #[test]
    fn set_password_and_username() {
        let mut url = Url::parse("ftp://user1:secret1@example.com").unwrap();
        url.set_password(Some("secret_password")).unwrap();
        assert_eq!(url.password(), Some("secret_password"));

        let mut url = Url::parse("ftp://:secre1@example.com/").unwrap();
        url.set_username("user1").unwrap();
        assert_eq!(url.username(), "user1");
        assert_eq!(url.as_str(), "ftp://user1:secre1@example.com/");
    }

    #[test]
    fn set_username_password_fail_for_cannot_be_a_base() {
        let mut url = Url::parse("mailto:rmz@example.com").unwrap();
        assert!(url.set_username("user1").is_err());
        assert!(url.set_password(Some("x")).is_err());
    }

    #[test]
    fn set_scheme_basic() {
        let mut url = Url::parse("https://example.net").unwrap();
        url.set_scheme("http").unwrap();
        assert_eq!(url.as_str(), "http://example.net/");

        let mut url = Url::parse("foo://example.net").unwrap();
        url.set_scheme("bar").unwrap();
        assert_eq!(url.as_str(), "bar://example.net");
    }

    #[test]
    fn set_scheme_rejects_invalid_and_incompatible_changes() {
        let mut url = Url::parse("https://example.net").unwrap();
        assert!(url.set_scheme("fo\u{f5}").is_err());
        assert_eq!(url.as_str(), "https://example.net/");

        let mut url = Url::parse("mailto:rms@example.net").unwrap();
        assert!(url.set_scheme("https").is_err());

        let mut url = Url::parse("foo://example.net").unwrap();
        assert!(url.set_scheme("https").is_err());

        let mut url = Url::parse("http://example.net").unwrap();
        assert!(url.set_scheme("foo").is_err());
    }

    #[test]
    fn join_relative_path() {
        let base = Url::parse("https://example.net/a/b.html").unwrap();
        assert_eq!(
            base.join("c.png").unwrap().as_str(),
            "https://example.net/a/c.png"
        );

        let base = Url::parse("https://example.net/a/b/").unwrap();
        assert_eq!(
            base.join("c.png").unwrap().as_str(),
            "https://example.net/a/b/c.png"
        );
    }

    #[test]
    fn join_absolute_path_and_scheme_relative() {
        let base = Url::parse("https://example.net/a/b/").unwrap();
        assert_eq!(
            base.join("/absolute").unwrap().as_str(),
            "https://example.net/absolute"
        );
        assert_eq!(
            base.join("//other.example/x").unwrap().as_str(),
            "https://other.example/x"
        );
        assert_eq!(
            base.join("http://other.example/y").unwrap().as_str(),
            "http://other.example/y"
        );
    }

    #[test]
    fn join_query_and_fragment_only() {
        let base = Url::parse("https://example.net/a/b/?old=1#frag").unwrap();
        assert_eq!(
            base.join("?new=2").unwrap().as_str(),
            "https://example.net/a/b/?new=2"
        );
        assert_eq!(
            base.join("#other").unwrap().as_str(),
            "https://example.net/a/b/?old=1#other"
        );
        assert_eq!(
            base.join("").unwrap().as_str(),
            "https://example.net/a/b/?old=1"
        );
    }

    #[test]
    fn join_dot_segments_and_backslash() {
        let base = Url::parse("https://example.net/a/b/").unwrap();
        assert_eq!(
            base.join("./x/./y/../z").unwrap().as_str(),
            "https://example.net/a/b/x/z"
        );
        assert_eq!(
            base.join("\\backslash").unwrap().as_str(),
            "https://example.net/backslash"
        );
    }

    #[test]
    fn join_file_url_windows_drive_inheritance() {
        let base = Url::parse("file:///C:/Users/foo/").unwrap();
        assert_eq!(
            base.join("../baz.txt").unwrap().as_str(),
            "file:///C:/Users/baz.txt"
        );
        let base = Url::parse("file:///C:/Users/foo/bar.txt").unwrap();
        assert_eq!(base.join("D:/other.txt").unwrap().as_str(), "d:/other.txt");
    }

    #[test]
    fn join_fails_for_cannot_be_a_base() {
        let base = Url::parse("data:text/plain,hello").unwrap();
        assert_eq!(
            base.join("world"),
            Err(ParseError::RelativeUrlWithCannotBeABaseBase)
        );
    }

    #[test]
    fn make_relative_basic_and_climbing() {
        let base = Url::parse("https://example.net/a/b.html").unwrap();
        let url = Url::parse("https://example.net/a/c.png").unwrap();
        assert_eq!(base.make_relative(&url).as_deref(), Some("c.png"));

        let base = Url::parse("https://example.net/a/b/").unwrap();
        let url = Url::parse("https://example.net/a/d/c.png").unwrap();
        assert_eq!(base.make_relative(&url).as_deref(), Some("../d/c.png"));
    }

    #[test]
    fn make_relative_query_only() {
        let base = Url::parse("https://example.net/a/b.html?c=d").unwrap();
        let url = Url::parse("https://example.net/a/b.html?e=f").unwrap();
        assert_eq!(base.make_relative(&url).as_deref(), Some("?e=f"));
    }

    #[test]
    fn make_relative_none_for_different_origin() {
        let base = Url::parse("https://example.net/a/b/").unwrap();
        let url = Url::parse("https://other.example/a/b/").unwrap();
        assert_eq!(base.make_relative(&url), None);
    }

    #[test]
    fn make_relative_is_inverse_of_join() {
        let base = Url::parse("https://example.net/a/b/c.html?x=1#f").unwrap();
        let url = Url::parse("https://example.net/a/d/e.html?y=2").unwrap();
        let relative = base.make_relative(&url).unwrap();
        assert_eq!(base.join(&relative).unwrap(), url);
    }

    #[test]
    fn path_segments_iterates_slash_separated() {
        let url = Url::parse("https://example.com/a/b/c").unwrap();
        let segments: Vec<_> = url.path_segments().unwrap().collect();
        assert_eq!(segments, vec!["a", "b", "c"]);

        let url = Url::parse("https://example.com/").unwrap();
        let segments: Vec<_> = url.path_segments().unwrap().collect();
        assert_eq!(segments, vec![""]);
    }

    #[test]
    fn path_segments_none_for_cannot_be_a_base() {
        let url = Url::parse("data:text/plain,x").unwrap();
        assert!(url.path_segments().is_none());
    }

    #[test]
    fn path_segments_mut_pop_and_push() {
        let mut url = Url::parse("http://example.net/foo/index.html").unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop()
            .push("img")
            .push("2/100%.png");
        assert_eq!(url.as_str(), "http://example.net/foo/img/2%2F100%25.png");
    }

    #[test]
    fn path_segments_mut_clear_and_push() {
        let mut url = Url::parse("https://github.com/servo/rust-url/").unwrap();
        url.path_segments_mut().unwrap().clear().push("logout");
        assert_eq!(url.as_str(), "https://github.com/logout");
    }

    #[test]
    fn path_segments_mut_pop_if_empty() {
        let mut url = Url::parse("https://github.com/servo/rust-url/").unwrap();
        url.path_segments_mut().unwrap().push("pulls");
        assert_eq!(url.as_str(), "https://github.com/servo/rust-url//pulls");

        let mut url = Url::parse("https://github.com/servo/rust-url/").unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .push("pulls");
        assert_eq!(url.as_str(), "https://github.com/servo/rust-url/pulls");
    }

    #[test]
    fn path_segments_mut_extend_skips_dot_segments() {
        let mut url = Url::parse("https://github.com/servo").unwrap();
        url.path_segments_mut()
            .unwrap()
            .extend(["..", "rust-url", ".", "pulls"]);
        assert_eq!(url.as_str(), "https://github.com/servo/rust-url/pulls");
    }

    #[test]
    fn path_segments_mut_extend_over_several_segments() {
        let mut url = Url::parse("https://github.com/").unwrap();
        url.path_segments_mut()
            .unwrap()
            .extend(["servo", "rust-url", "issues", "188"]);
        assert_eq!(url.as_str(), "https://github.com/servo/rust-url/issues/188");
    }

    #[test]
    fn path_segments_mut_fails_for_cannot_be_a_base() {
        let mut url = Url::parse("mailto:me@example.com").unwrap();
        assert!(url.path_segments_mut().is_err());
    }

    #[test]
    fn query_pairs_iterates_decoded_pairs() {
        let url = Url::parse("https://example.com/products?page=2&sort=desc").unwrap();
        let pairs: Vec<_> = url.query_pairs().collect();
        assert_eq!(
            pairs,
            vec![
                (
                    std::borrow::Cow::Borrowed("page"),
                    std::borrow::Cow::Borrowed("2")
                ),
                (
                    std::borrow::Cow::Borrowed("sort"),
                    std::borrow::Cow::Borrowed("desc")
                ),
            ]
        );
    }

    #[test]
    fn query_pairs_empty_for_no_query() {
        let url = Url::parse("https://example.com/products").unwrap();
        assert_eq!(url.query_pairs().count(), 0);
    }

    #[test]
    fn query_pairs_mut_append_preserves_fragment() {
        let mut url = Url::parse("https://example.net?lang=fr#nav").unwrap();
        url.query_pairs_mut().append_pair("foo", "bar");
        assert_eq!(url.query(), Some("lang=fr&foo=bar"));
        assert_eq!(url.as_str(), "https://example.net/?lang=fr&foo=bar#nav");
    }

    #[test]
    fn query_pairs_mut_clear_and_append() {
        let mut url = Url::parse("https://example.net?lang=fr#nav").unwrap();
        url.query_pairs_mut()
            .clear()
            .append_pair("foo", "bar & baz")
            .append_pair("saisons", "\u{c9}t\u{e9}+hiver");
        assert_eq!(
            url.query(),
            Some("foo=bar+%26+baz&saisons=%C3%89t%C3%A9%2Bhiver")
        );
        assert_eq!(
            url.as_str(),
            "https://example.net/?foo=bar+%26+baz&saisons=%C3%89t%C3%A9%2Bhiver#nav"
        );
    }

    #[test]
    fn query_pairs_mut_on_url_with_no_prior_query() {
        let mut url = Url::parse("https://example.net/path").unwrap();
        url.query_pairs_mut().append_pair("a", "1");
        assert_eq!(url.as_str(), "https://example.net/path?a=1");
    }

    #[test]
    fn parse_with_params_appends_to_existing_query() {
        let url = Url::parse_with_params(
            "https://example.net?dont=clobberme",
            &[("lang", "rust"), ("browser", "servo")],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.net/?dont=clobberme&lang=rust&browser=servo"
        );
    }

    #[test]
    fn parse_with_params_propagates_parse_error() {
        assert!(Url::parse_with_params("not a url", &[("a", "b")]).is_err());
    }

    #[test]
    fn options_base_url_resolves_relative_input() {
        let api = Url::parse("https://api.example.com").unwrap();
        let options = Url::options().base_url(Some(&api));
        let version_url = options.parse("version.json").unwrap();
        assert_eq!(version_url.as_str(), "https://api.example.com/version.json");
    }

    #[test]
    fn options_without_base_url_behaves_like_parse() {
        let url = Url::options().parse("https://example.net/x").unwrap();
        assert_eq!(url.as_str(), "https://example.net/x");
    }

    #[test]
    fn syntax_violation_callback_reports_expected_double_slash() {
        use std::cell::RefCell;
        let violations = RefCell::new(Vec::new());
        let url = Url::options()
            .syntax_violation_callback(Some(&|v| violations.borrow_mut().push(v)))
            .parse("https:////example.com")
            .unwrap();
        assert_eq!(url.as_str(), "https://example.com/");
        assert_eq!(
            violations.into_inner(),
            vec![SyntaxViolation::ExpectedDoubleSlash]
        );
    }

    #[test]
    fn syntax_violation_callback_reports_backslash_and_embedded_credentials() {
        use std::cell::RefCell;
        let violations = RefCell::new(Vec::new());
        let url = Url::options()
            .syntax_violation_callback(Some(&|v| violations.borrow_mut().push(v)))
            .parse("http://user:pass@example.com\\path")
            .unwrap();
        assert_eq!(url.as_str(), "http://user:pass@example.com/path");
        let logged = violations.into_inner();
        assert!(logged.contains(&SyntaxViolation::EmbeddedCredentials));
        assert!(logged.contains(&SyntaxViolation::Backslash));
    }

    #[test]
    fn syntax_violation_callback_silent_on_clean_input() {
        use std::cell::RefCell;
        let violations = RefCell::new(Vec::new());
        Url::options()
            .syntax_violation_callback(Some(&|v| violations.borrow_mut().push(v)))
            .parse("https://example.net/clean/path?q=1")
            .unwrap();
        assert!(violations.into_inner().is_empty());
    }

    #[test]
    fn encoding_override_applies_to_special_scheme_query() {
        fn windows_1252_lossy(input: &str) -> std::borrow::Cow<'_, [u8]> {
            if input.is_ascii() {
                std::borrow::Cow::Borrowed(input.as_bytes())
            } else {
                std::borrow::Cow::Owned(input.chars().map(|c| c as u8).collect())
            }
        }
        let url = Url::options()
            .encoding_override(Some(&windows_1252_lossy))
            .parse("https://example.net/?q=caf\u{e9}")
            .unwrap();
        // 'é' (U+00E9) is mapped to the single byte 0xE9, not UTF-8's 0xC3 0xA9.
        assert_eq!(url.query(), Some("q=caf%E9"));
    }

    #[test]
    fn encoding_override_ignored_for_non_special_scheme() {
        fn windows_1252_lossy(input: &str) -> std::borrow::Cow<'_, [u8]> {
            std::borrow::Cow::Owned(input.chars().map(|c| c as u8).collect())
        }
        let url = Url::options()
            .encoding_override(Some(&windows_1252_lossy))
            .parse("custom-scheme:x?q=caf\u{e9}")
            .unwrap();
        // Override only applies to http/https/file/ftp; falls back to UTF-8.
        assert_eq!(url.query(), Some("q=caf%C3%A9"));
    }

    #[cfg(unix)]
    #[test]
    fn from_file_path_and_back_roundtrip() {
        let url = Url::from_file_path("/tmp/foo bar.txt").unwrap();
        assert_eq!(url.as_str(), "file:///tmp/foo%20bar.txt");
        assert_eq!(
            url.to_file_path().unwrap(),
            std::path::PathBuf::from("/tmp/foo bar.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn from_file_path_requires_absolute() {
        assert!(Url::from_file_path("relative/path").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn from_directory_path_has_trailing_slash() {
        let url = Url::from_directory_path("/var/www").unwrap();
        assert_eq!(url.as_str(), "file:///var/www/");
        let joined = url.join("index.html").unwrap();
        assert_eq!(joined.as_str(), "file:///var/www/index.html");
    }

    #[cfg(unix)]
    #[test]
    fn to_file_path_rejects_non_local_host() {
        let url = Url::parse("file://example.net/foo").unwrap();
        assert!(url.to_file_path().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn to_file_path_accepts_localhost() {
        let url = Url::parse("file://localhost/foo/bar").unwrap();
        assert_eq!(
            url.to_file_path().unwrap(),
            std::path::PathBuf::from("/foo/bar")
        );
    }

    #[cfg(unix)]
    #[test]
    fn to_file_path_fails_for_non_base_url() {
        let url = Url::parse("data:text/plain,hi").unwrap();
        assert!(url.to_file_path().is_err());
    }

    // `file_url_segments_to_pathbuf_windows` only manipulates strings (no
    // platform-specific `Path` parsing), so it's kept available under
    // `#[cfg(any(windows, test))]` and exercised directly here — CI runs on
    // Linux, so this is the only way to cover the Windows URL->path
    // direction. The reverse direction, `path_to_file_url_segments_windows`,
    // depends on `Path::components()`'s platform-specific parsing of drive
    // letters and UNC prefixes, which Linux's `std::path` doesn't perform,
    // so it stays `#[cfg(windows)]`-only and untested here.
    #[test]
    fn windows_file_url_segments_to_pathbuf_disk() {
        let path = crate::file_path::file_url_segments_to_pathbuf_windows(
            None,
            "C:/Users/me/file.txt".split('/'),
        )
        .unwrap();
        assert_eq!(path, std::path::PathBuf::from(r"C:\Users\me\file.txt"));
    }

    #[test]
    fn windows_file_url_segments_to_pathbuf_percent_encoded_drive() {
        let path =
            crate::file_path::file_url_segments_to_pathbuf_windows(None, "C%3A/dir".split('/'))
                .unwrap();
        assert_eq!(path, std::path::PathBuf::from(r"C:\dir"));
    }

    #[test]
    fn windows_file_url_segments_to_pathbuf_unc_host() {
        let segments = "share/dir/file.txt".split('/');
        let path = crate::file_path::file_url_segments_to_pathbuf_windows(Some("server"), segments)
            .unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from(r"\\server\share\dir\file.txt")
        );
    }

    #[test]
    fn windows_file_url_segments_to_pathbuf_rejects_malformed_first_segment() {
        assert!(crate::file_path::file_url_segments_to_pathbuf_windows(
            None,
            "notadrive".split('/')
        )
        .is_err());
    }

    #[test]
    fn socket_addrs_ipv4_host_with_explicit_port() {
        let url = Url::parse("http://127.0.0.1:9000/").unwrap();
        let addrs = url.socket_addrs(|| None).unwrap();
        assert_eq!(addrs, vec!["127.0.0.1:9000".parse().unwrap()]);
    }

    #[test]
    fn socket_addrs_ipv6_host_falls_back_to_known_default_port() {
        let url = Url::parse("https://[::1]/").unwrap();
        let addrs = url.socket_addrs(|| None).unwrap();
        assert_eq!(addrs, vec!["[::1]:443".parse().unwrap()]);
    }

    #[test]
    fn socket_addrs_uses_default_port_number_callback_for_unknown_scheme() {
        let url = Url::parse("socks5://127.0.0.1/").unwrap();
        let addrs = url.socket_addrs(|| Some(1080)).unwrap();
        assert_eq!(addrs, vec!["127.0.0.1:1080".parse().unwrap()]);
    }

    #[test]
    fn socket_addrs_fails_without_a_port() {
        let url = Url::parse("socks5://127.0.0.1/").unwrap();
        assert!(url.socket_addrs(|| None).is_err());
    }

    #[test]
    fn socket_addrs_fails_without_a_host() {
        let url = Url::parse("data:text/plain,hi").unwrap();
        assert!(url.socket_addrs(|| None).is_err());
    }
}
