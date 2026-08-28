//! The URL [origin](https://url.spec.whatwg.org/#origin) concept: a
//! (scheme, host, port) tuple for network schemes, or an opaque identifier
//! for everything else.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::host::Host;
use crate::parser::default_port;
use crate::Url;

/// The origin of a URL. Two URLs with the same origin are considered to
/// come from the same source and may trust each other.
///
/// - `blob:` URLs take the origin of the URL in their path (or an opaque
///   origin if that fails to parse).
/// - `ftp`, `http`, `https`, `ws`, and `wss` URLs get a `Tuple` origin of
///   their scheme, host, and port.
/// - Every other scheme (including `file:`) gets a fresh [`OpaqueOrigin`],
///   equal only to itself.
///
/// See the [HTML Standard](https://html.spec.whatwg.org/multipage/#origin).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A globally unique identifier, equal only to itself.
    Opaque(OpaqueOrigin),
    /// A (scheme, host, port) tuple.
    Tuple(String, Host<String>, u16),
}

impl Origin {
    /// Create a fresh opaque origin, equal only to itself (and not to any
    /// other opaque origin, even one created for the same URL).
    pub fn new_opaque() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        Origin::Opaque(OpaqueOrigin(COUNTER.fetch_add(1, Ordering::SeqCst)))
    }

    /// Return whether this is a `(scheme, host, port)` tuple origin, as
    /// opposed to an opaque one.
    pub fn is_tuple(&self) -> bool {
        matches!(self, Origin::Tuple(..))
    }

    /// <https://html.spec.whatwg.org/multipage/#ascii-serialisation-of-an-origin>
    pub fn ascii_serialization(&self) -> String {
        match self {
            Origin::Opaque(_) => "null".to_owned(),
            Origin::Tuple(scheme, host, port) => serialize_tuple(scheme, host, *port),
        }
    }

    /// <https://html.spec.whatwg.org/multipage/#unicode-serialisation-of-an-origin>
    pub fn unicode_serialization(&self) -> String {
        match self {
            Origin::Opaque(_) => "null".to_owned(),
            Origin::Tuple(scheme, host, port) => {
                let unicode_host;
                let host = match host {
                    Host::Domain(domain) => {
                        let (domain, _errors) = idna::domain_to_unicode(domain);
                        unicode_host = Host::Domain(domain);
                        &unicode_host
                    }
                    _ => host,
                };
                serialize_tuple(scheme, host, *port)
            }
        }
    }
}

fn serialize_tuple(scheme: &str, host: &Host<String>, port: u16) -> String {
    let mut out = format!("{scheme}://{host}");
    if default_port(scheme) != Some(port) {
        write!(out, ":{port}").unwrap();
    }
    out
}

/// An opaque, globally unique origin identifier for URLs that don't have a
/// `(scheme, host, port)` tuple origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpaqueOrigin(usize);

/// <https://url.spec.whatwg.org/#concept-url-origin>
pub(crate) fn url_origin(url: &Url) -> Origin {
    match url.scheme() {
        "blob" => match Url::parse(url.path()) {
            Ok(ref inner) => url_origin(inner),
            Err(_) => Origin::new_opaque(),
        },
        "ftp" | "http" | "https" | "ws" | "wss" => Origin::Tuple(
            url.scheme().to_owned(),
            url.host().unwrap().to_owned(),
            url.port_or_known_default().unwrap(),
        ),
        // file: URLs are deliberately opaque — see the WHATWG URL Standard's
        // note that browsers' actual file: origin behavior is not
        // standardized.
        _ => Origin::new_opaque(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_origin_for_special_network_schemes() {
        let url = Url::parse("https://example.com:8443/path").unwrap();
        let origin = url.origin();
        assert!(origin.is_tuple());
        assert_eq!(origin.ascii_serialization(), "https://example.com:8443");
    }

    #[test]
    fn default_port_is_omitted_from_serialization() {
        let url = Url::parse("https://example.com/path").unwrap();
        assert_eq!(url.origin().ascii_serialization(), "https://example.com");
    }

    #[test]
    fn opaque_origin_for_file_and_data() {
        assert!(!Url::parse("file:///tmp/foo").unwrap().origin().is_tuple());
        assert_eq!(
            Url::parse("file:///tmp/foo")
                .unwrap()
                .origin()
                .ascii_serialization(),
            "null"
        );
        assert!(!Url::parse("data:text/plain,x").unwrap().origin().is_tuple());
    }

    #[test]
    fn opaque_origins_are_never_equal_even_for_the_same_url() {
        let url = Url::parse("data:text/plain,x").unwrap();
        assert_ne!(url.origin(), url.origin());
    }

    #[test]
    fn blob_url_takes_origin_of_inner_url() {
        let url = Url::parse("blob:https://example.com:8443/uuid").unwrap();
        let origin = url.origin();
        assert!(origin.is_tuple());
        assert_eq!(origin.ascii_serialization(), "https://example.com:8443");
    }

    #[test]
    fn blob_url_with_unparseable_inner_url_is_opaque() {
        let url = Url::parse("blob:not a valid url").unwrap();
        assert!(!url.origin().is_tuple());
    }

    #[test]
    fn unicode_serialization_decodes_punycode() {
        let url = Url::parse("https://bücher.example/").unwrap();
        assert_eq!(
            url.origin().unicode_serialization(),
            "https://bücher.example"
        );
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://xn--bcher-kva.example"
        );
    }

    #[test]
    fn equal_tuple_origins_compare_equal() {
        let a = Url::parse("https://example.com/a").unwrap().origin();
        let b = Url::parse("https://example.com/b").unwrap().origin();
        assert_eq!(a, b);
    }
}
