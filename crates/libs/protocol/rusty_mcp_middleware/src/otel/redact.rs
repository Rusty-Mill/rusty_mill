//! What may be said about a collector in a log line or an error message.
//!
//! An endpoint can carry a credential (`?token=...`, a secret path segment), and
//! the HTTP and URL errors built from one quote it back. So diagnostics name
//! the origin only, and describe a failure by kind, never by its text.

use rusty_http::Url;

/// `scheme://host[:port]` of `url`, or a placeholder when it does not parse.
///
/// Never echoes the input: a URL that fails to parse is the one most likely to
/// hold something it should not, such as userinfo.
pub(crate) fn origin(url: &str) -> String {
    match Url::parse(url) {
        Ok(url) => format!("{}://{}", url.scheme, url.host_header()),
        Err(_) => "<unparseable url>".to_owned(),
    }
}

/// A fixed-vocabulary description of a failed request.
pub(crate) fn reason(err: &rusty_request::Error) -> String {
    use rusty_request::Error;
    match err {
        Error::Timeout => "timed out".to_owned(),
        Error::Status(status) => format!("the collector answered {status}"),
        Error::Io(err) => format!("i/o error ({:?})", err.kind()),
        Error::Tls(_) => "tls error".to_owned(),
        Error::InvalidResponse(_) => "invalid response".to_owned(),
        _ => "request failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_drops_credentials_path_and_query() {
        assert_eq!(
            origin("https://collector.example:4318/v1/metrics?token=SECRET"),
            "https://collector.example:4318"
        );
        assert_eq!(
            origin("http://collector.example/secret-path-SECRET"),
            "http://collector.example"
        );
    }

    #[test]
    fn an_unparseable_url_is_not_echoed() {
        for url in ["https://user:SECRET@host/v1/metrics", "SECRET", ""] {
            let said = origin(url);
            assert!(!said.contains("SECRET"), "{said}");
        }
    }

    #[test]
    fn a_reason_never_quotes_the_error_text() {
        let err = rusty_request::Error::InvalidUrl("https://x/?token=SECRET".to_owned());
        assert!(!reason(&err).contains("SECRET"));
        let err = rusty_request::Error::UnsupportedScheme("`SECRET`".to_owned());
        assert!(!reason(&err).contains("SECRET"));
    }
}
