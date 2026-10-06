//! Fetching a calendar feed on the user's behalf (`POST /api/fetch-ics`).
//!
//! Browsers cannot read most calendar feeds (no CORS headers), so the server
//! does. That makes it a request forger, so this module is deliberately
//! narrow: HTTPS only, port 443 only, no credentials in the URL, every
//! resolved address must be public (checked before connecting, and the
//! connection goes to the checked address, so a DNS answer cannot change
//! between check and use), at most three redirects each re-checked the same
//! way, a short timeout per step, a size cap, and a body that must look like
//! a calendar. It speaks `rusty_http` over `rusty_tls`: no new third-party
//! crate.

use rusty_http::body::{self, Framing};
use rusty_http::head::RequestHead;
use rusty_http::sync::SyncTransport;
use rusty_http::{HeaderMap, Method, Url, Version};
use rusty_tls::{TlsConnector, TrustPolicy};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Largest feed accepted, in bytes.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Redirects followed before giving up.
pub const MAX_REDIRECTS: usize = 3;
/// Connect, read and write timeout for each step.
const STEP_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_HEAD_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FetchError {
    /// The URL itself is unusable: not https, wrong port, malformed.
    #[error("{0}")]
    Invalid(String),
    /// The address is not one this server will connect to.
    #[error("{0}")]
    Refused(String),
    /// The remote end failed or sent something that is not a calendar.
    #[error("{0}")]
    Upstream(String),
    /// Too many fetches are already running; try again shortly.
    #[error("too many calendar fetches are running; try again shortly")]
    Busy,
}

/// Fetches allowed in flight at once. Each holds a connection thread for up
/// to its timeouts, so this keeps a burst of them from using up the
/// server's connections.
pub const MAX_IN_FLIGHT: usize = 4;

/// A place among the running fetches, given back on drop.
struct Slot<'a>(&'a AtomicUsize);

impl<'a> Slot<'a> {
    /// A slot, unless `max` are already taken.
    fn take(counter: &'a AtomicUsize, max: usize) -> Option<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(counter))
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Whether the server may connect to `ip`: only globally routable unicast
/// addresses. Loopback, private, link-local, shared (CGNAT), unspecified,
/// multicast, documentation and IPv4-mapped/ULA ranges are all out.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..=127).contains(&b)) // 100.64.0.0/10, shared address space
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24, IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // 198.18.0.0/15, benchmarking
        || a >= 240) // reserved
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00 // fc00::/7, unique local
        || (first & 0xffc0) == 0xfe80 // fe80::/10, link-local
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)) // documentation
}

/// Parse `raw` and refuse anything but a plain `https://host/...` on port 443.
/// `webcal://` (the usual way a feed is shared) is read as `https://`.
pub fn check_url(raw: &str) -> Result<Url, FetchError> {
    let raw = raw.trim();
    let swapped;
    let raw = match raw.get(..9).map(str::to_ascii_lowercase).as_deref() {
        Some("webcal://") => {
            swapped = format!("https://{}", &raw[9..]);
            &swapped
        }
        _ => raw,
    };
    let url = Url::parse(raw).map_err(|e| FetchError::Invalid(format!("not a usable URL: {e}")))?;
    if url.scheme != "https" {
        return Err(FetchError::Invalid(
            "only https:// feeds are supported".into(),
        ));
    }
    if url.port != 443 {
        return Err(FetchError::Invalid(
            "only the default https port (443) is supported".into(),
        ));
    }
    if url.host.is_empty() || url.host.contains(['[', ']']) {
        return Err(FetchError::Invalid(
            "use a host name, not an IP literal".into(),
        ));
    }
    Ok(url)
}

/// Where a `Location` header sends us, as an absolute URL string.
pub fn resolve_location(from: &Url, location: &str) -> String {
    if location.contains("://") {
        location.to_string()
    } else if location.starts_with('/') {
        format!("https://{}{location}", from.host)
    } else {
        let base = from.path.rsplit_once('/').map_or("", |(dir, _)| dir);
        format!("https://{}{base}/{location}", from.host)
    }
}

enum Hop {
    Body(String),
    Redirect(String),
}

/// Send the request over `io` and read the answer: the calendar text, or where to go next.
fn exchange<T: Read + Write>(io: T, url: &Url) -> Result<Hop, FetchError> {
    let upstream =
        |what: &str, e: &dyn std::fmt::Display| FetchError::Upstream(format!("{what}: {e}"));
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("Host", url.host.as_str()),
        ("User-Agent", "rusty_tick"),
        ("Accept", "text/calendar, text/plain;q=0.5"),
        ("Accept-Encoding", "identity"),
        ("Connection", "close"),
    ] {
        headers
            .insert(name, value)
            .map_err(|e| upstream("building the request", &e))?;
    }
    let target = match &url.query {
        Some(q) => format!("{}?{q}", url.path),
        None => url.path.clone(),
    };
    let mut transport = SyncTransport::new(io);
    transport
        .write_request_head(&RequestHead {
            method: Method::Get,
            target,
            version: Version::Http11,
            headers,
        })
        .map_err(|e| upstream("sending the request", &e))?;
    let head = transport
        .read_response_head(MAX_HEAD_BYTES)
        .map_err(|e| upstream("reading the response", &e))?;

    if head.status.is_redirection() {
        return match head.headers.get("location") {
            Some(l) => Ok(Hop::Redirect(l.to_string())),
            None => Err(FetchError::Upstream("a redirect with no Location".into())),
        };
    }
    if !head.status.is_success() {
        return Err(FetchError::Upstream(format!(
            "the feed answered {}",
            head.status.as_u16()
        )));
    }
    let bytes = match body::response_framing(&head.headers, &Method::Get, head.status)
        .map_err(|e| upstream("framing", &e))?
    {
        Framing::None => Vec::new(),
        Framing::ContentLength(n) if n > MAX_BYTES => return Err(too_big()),
        Framing::ContentLength(n) => transport
            .read_content_length_body(n, MAX_BYTES)
            .map_err(|e| upstream("reading the feed", &e))?,
        Framing::Chunked => transport
            .read_chunked_body(body::DEFAULT_MAX_LINE_LEN)
            .map_err(|e| upstream("reading the feed", &e))?,
        Framing::Close => transport
            .read_close_delimited_body(MAX_BYTES)
            .map_err(|e| upstream("reading the feed", &e))?,
    };
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_big());
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if !text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_uppercase()
        .starts_with("BEGIN:VCALENDAR")
    {
        return Err(FetchError::Upstream(
            "the response is not an iCalendar feed".into(),
        ));
    }
    Ok(Hop::Body(text))
}

fn too_big() -> FetchError {
    FetchError::Upstream(format!(
        "the feed is larger than {} MiB",
        MAX_BYTES / (1024 * 1024)
    ))
}

fn connector() -> Result<&'static TlsConnector, FetchError> {
    static CONNECTOR: OnceLock<Result<TlsConnector, String>> = OnceLock::new();
    CONNECTOR
        .get_or_init(|| TlsConnector::new(&TrustPolicy::default()).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| FetchError::Upstream(format!("TLS is unavailable: {e}")))
}

/// Resolve `url`'s host and return an address to connect to, refusing the
/// request if any answer is non-public (one bad record is enough to refuse).
fn public_address(url: &Url) -> Result<SocketAddr, FetchError> {
    let addrs: Vec<SocketAddr> = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| FetchError::Upstream(format!("could not resolve {}: {e}", url.host)))?
        .collect();
    if let Some(bad) = addrs.iter().find(|a| !is_public(a.ip())) {
        return Err(FetchError::Refused(format!(
            "{} resolves to an address this server will not connect to ({})",
            url.host,
            bad.ip()
        )));
    }
    addrs
        .into_iter()
        .next()
        .ok_or_else(|| FetchError::Upstream(format!("{} has no address", url.host)))
}

/// Fetch the calendar at `raw` (an `https://` or `webcal://` URL) as text.
///
/// # Errors
///
/// [`FetchError::Invalid`] for a URL this server will not use,
/// [`FetchError::Refused`] for one that resolves to a non-public address, and
/// [`FetchError::Upstream`] for a failure on the way or a response that is
/// not a calendar, and [`FetchError::Busy`] when [`MAX_IN_FLIGHT`] fetches
/// are already running.
pub fn fetch_text(raw: &str) -> Result<String, FetchError> {
    let mut url = check_url(raw)?;
    let _slot = Slot::take(&IN_FLIGHT, MAX_IN_FLIGHT).ok_or(FetchError::Busy)?;
    for _ in 0..=MAX_REDIRECTS {
        let addr = public_address(&url)?;
        let sock = TcpStream::connect_timeout(&addr, STEP_TIMEOUT)
            .map_err(|e| FetchError::Upstream(format!("could not connect to {}: {e}", url.host)))?;
        sock.set_read_timeout(Some(STEP_TIMEOUT))
            .and_then(|()| sock.set_write_timeout(Some(STEP_TIMEOUT)))
            .map_err(|e| FetchError::Upstream(e.to_string()))?;
        let tls = connector()?
            .connect(sock, &url.host)
            .map_err(|e| FetchError::Upstream(format!("TLS to {}: {e}", url.host)))?;
        match exchange(tls, &url)? {
            Hop::Body(text) => return Ok(text),
            Hop::Redirect(location) => url = check_url(&resolve_location(&url, &location))?,
        }
    }
    Err(FetchError::Upstream(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Reads a canned response; keeps what was written.
    struct Duplex {
        reply: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }
    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reply.read(buf)
        }
    }
    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn ask(reply: &str) -> Result<Hop, FetchError> {
        let io = Duplex {
            reply: Cursor::new(reply.as_bytes().to_vec()),
            sent: Vec::new(),
        };
        exchange(io, &Url::parse("https://example.com/cal.ics?x=1").unwrap())
    }

    const CAL: &str = "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n";

    #[test]
    fn only_public_addresses_pass() {
        for ok in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!is_public(bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn only_so_many_fetches_run_at_once_and_a_slot_is_given_back() {
        let counter = AtomicUsize::new(0);
        let a = Slot::take(&counter, 2).unwrap();
        let _b = Slot::take(&counter, 2).unwrap();
        assert!(Slot::take(&counter, 2).is_none(), "a third is refused");
        drop(a);
        assert!(
            Slot::take(&counter, 2).is_some(),
            "a freed slot can be taken again"
        );
    }

    #[test]
    fn urls_must_be_plain_https_on_443() {
        assert_eq!(
            check_url(" https://example.com/a.ics ").unwrap().host,
            "example.com"
        );
        assert_eq!(
            check_url("webcal://example.com/a.ics").unwrap().scheme,
            "https"
        );
        assert_eq!(
            check_url("WEBCAL://example.com/a.ics").unwrap().scheme,
            "https"
        );
        for bad in [
            "http://example.com/a.ics",
            "https://example.com:8443/a.ics",
            "https://u:p@example.com/a.ics",
            "ftp://example.com/a",
            "example.com/a.ics",
            "https://[::1]/a.ics",
            "",
        ] {
            assert!(
                matches!(check_url(bad), Err(FetchError::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn redirect_targets_are_resolved_against_the_current_url() {
        let from = Url::parse("https://example.com/feeds/a.ics").unwrap();
        assert_eq!(
            resolve_location(&from, "/b.ics"),
            "https://example.com/b.ics"
        );
        assert_eq!(
            resolve_location(&from, "b.ics"),
            "https://example.com/feeds/b.ics"
        );
        assert_eq!(
            resolve_location(&from, "https://other.org/c.ics"),
            "https://other.org/c.ics"
        );
    }

    #[test]
    fn a_calendar_body_is_returned_and_the_request_is_well_formed() {
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\n\r\n{CAL}",
            CAL.len()
        );
        let io = Duplex {
            reply: Cursor::new(reply.into_bytes()),
            sent: Vec::new(),
        };
        let url = Url::parse("https://example.com/cal.ics?x=1").unwrap();
        let mut sent = Vec::new();
        match exchange(
            &mut RecordingIo {
                inner: io,
                tap: &mut sent,
            },
            &url,
        )
        .unwrap()
        {
            Hop::Body(text) => assert_eq!(text, CAL),
            Hop::Redirect(_) => panic!("no redirect expected"),
        }
        let sent = String::from_utf8(sent).unwrap();
        assert!(sent.starts_with("GET /cal.ics?x=1 HTTP/1.1\r\n"), "{sent}");
        assert!(sent.contains("Host: example.com\r\n") && sent.contains("Connection: close\r\n"));
    }

    /// Copies what is written to `tap`, so the test can read it after `exchange` takes the io.
    struct RecordingIo<'a> {
        inner: Duplex,
        tap: &'a mut Vec<u8>,
    }
    impl Read for &mut RecordingIo<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl Write for &mut RecordingIo<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.tap.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn chunked_and_close_delimited_bodies_are_read() {
        let chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{CAL}\r\n0\r\n\r\n",
            CAL.len()
        );
        assert!(matches!(ask(&chunked), Ok(Hop::Body(t)) if t == CAL));
        let close = format!("HTTP/1.0 200 OK\r\n\r\n{CAL}");
        assert!(matches!(ask(&close), Ok(Hop::Body(t)) if t == CAL));
    }

    #[test]
    fn redirects_errors_and_non_calendars_are_reported() {
        assert!(
            matches!(ask("HTTP/1.1 302 Found\r\nLocation: /next.ics\r\nContent-Length: 0\r\n\r\n"), Ok(Hop::Redirect(l)) if l == "/next.ics")
        );
        assert!(matches!(
            ask("HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n"),
            Err(FetchError::Upstream(_))
        ));
        assert!(
            matches!(ask("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"), Err(FetchError::Upstream(m)) if m.contains("404"))
        );
        assert!(
            matches!(ask("HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\n<html>"), Err(FetchError::Upstream(m)) if m.contains("not an iCalendar"))
        );
        let huge = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BYTES + 1
        );
        assert!(matches!(ask(&huge), Err(FetchError::Upstream(m)) if m.contains("larger than")));
    }

    #[test]
    fn a_host_that_resolves_to_loopback_is_refused_before_any_connection() {
        let url = Url::parse("https://localhost/cal.ics").unwrap();
        assert!(matches!(public_address(&url), Err(FetchError::Refused(_))));
        assert!(matches!(
            fetch_text("https://localhost/cal.ics"),
            Err(FetchError::Refused(_))
        ));
    }
}
