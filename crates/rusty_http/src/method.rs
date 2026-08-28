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

    /// Whether the method is defined as safe by RFC 7231 §4.2.1 --
    /// conventionally read-only, generating no side effects a client
    /// requested. `Extension` is never safe: an unrecognized method can't
    /// be assumed to be.
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            Method::Get | Method::Head | Method::Options | Method::Trace
        )
    }

    /// Whether the method is defined as idempotent by RFC 7231 §4.2.2 --
    /// repeating an identical request has the same effect as sending it
    /// once. `Extension` is never idempotent: an unrecognized method can't
    /// be assumed to be.
    pub fn is_idempotent(&self) -> bool {
        matches!(
            self,
            Method::Get
                | Method::Head
                | Method::Put
                | Method::Delete
                | Method::Options
                | Method::Trace
        )
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

    #[test]
    fn safe_methods() {
        assert!(Method::Get.is_safe());
        assert!(Method::Head.is_safe());
        assert!(Method::Options.is_safe());
        assert!(Method::Trace.is_safe());
        assert!(!Method::Post.is_safe());
        assert!(!Method::Put.is_safe());
        assert!(!Method::Delete.is_safe());
        assert!(!Method::Extension("PROPFIND".to_string()).is_safe());
    }

    #[test]
    fn idempotent_methods() {
        assert!(Method::Get.is_idempotent());
        assert!(Method::Put.is_idempotent());
        assert!(Method::Delete.is_idempotent());
        assert!(!Method::Post.is_idempotent());
        assert!(!Method::Patch.is_idempotent());
        assert!(!Method::Extension("PROPFIND".to_string()).is_idempotent());
    }
}
