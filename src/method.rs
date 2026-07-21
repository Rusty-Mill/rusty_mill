//! The HTTP request method named on a request line.

use std::fmt;

/// An HTTP request method. The common methods are named variants (matching
/// `rusty_request`'s donor set, plus `Connect`/`Trace` for completeness);
/// [`Method::Extension`] carries anything else -- unlike a client, which
/// only ever sends methods it names itself, a server-side parser has to
/// accept whatever token a request line actually contains.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
    /// `CONNECT`
    Connect,
    /// `TRACE`
    Trace,
    /// Any other method token, verbatim.
    Extension(String),
}

impl Method {
    /// The method's wire representation, e.g. `"GET"`.
    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
            Method::Connect => "CONNECT",
            Method::Trace => "TRACE",
            Method::Extension(s) => s.as_str(),
        }
    }

    /// Parses a request-line method token. Never fails -- an unrecognized
    /// token becomes [`Method::Extension`]; rejecting it is the parser's
    /// job (a malformed/empty token never reaches here, since it's split
    /// off the request line by whitespace).
    pub fn parse(token: &str) -> Method {
        match token {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            "CONNECT" => Method::Connect,
            "TRACE" => Method::Trace,
            other => Method::Extension(other.to_string()),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_methods() {
        assert_eq!(Method::parse("GET"), Method::Get);
        assert_eq!(Method::parse("PATCH"), Method::Patch);
    }

    #[test]
    fn unknown_token_becomes_extension() {
        assert_eq!(
            Method::parse("PROPFIND"),
            Method::Extension("PROPFIND".to_string())
        );
        assert_eq!(Method::parse("PROPFIND").as_str(), "PROPFIND");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(Method::Get.to_string(), "GET");
    }
}
