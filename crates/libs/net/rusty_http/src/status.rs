//! An HTTP status code.

/// An HTTP status code. Ported verbatim from `rusty_request`'s `status.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatusCode(u16);

impl StatusCode {
    /// `100 Continue`.
    pub const CONTINUE: StatusCode = StatusCode(100);
    /// `101 Switching Protocols`.
    pub const SWITCHING_PROTOCOLS: StatusCode = StatusCode(101);
    /// `102 Processing`.
    pub const PROCESSING: StatusCode = StatusCode(102);
    /// `103 Early Hints`.
    pub const EARLY_HINTS: StatusCode = StatusCode(103);

    /// `200 OK`.
    pub const OK: StatusCode = StatusCode(200);
    /// `201 Created`.
    pub const CREATED: StatusCode = StatusCode(201);
    /// `202 Accepted`.
    pub const ACCEPTED: StatusCode = StatusCode(202);
    /// `203 Non-Authoritative Information`.
    pub const NON_AUTHORITATIVE_INFORMATION: StatusCode = StatusCode(203);
    /// `204 No Content`.
    pub const NO_CONTENT: StatusCode = StatusCode(204);
    /// `205 Reset Content`.
    pub const RESET_CONTENT: StatusCode = StatusCode(205);
    /// `206 Partial Content`.
    pub const PARTIAL_CONTENT: StatusCode = StatusCode(206);
    /// `207 Multi-Status`.
    pub const MULTI_STATUS: StatusCode = StatusCode(207);
    /// `208 Already Reported`.
    pub const ALREADY_REPORTED: StatusCode = StatusCode(208);
    /// `226 IM Used`.
    pub const IM_USED: StatusCode = StatusCode(226);

    /// `300 Multiple Choices`.
    pub const MULTIPLE_CHOICES: StatusCode = StatusCode(300);
    /// `301 Moved Permanently`.
    pub const MOVED_PERMANENTLY: StatusCode = StatusCode(301);
    /// `302 Found`.
    pub const FOUND: StatusCode = StatusCode(302);
    /// `303 See Other`.
    pub const SEE_OTHER: StatusCode = StatusCode(303);
    /// `304 Not Modified`.
    pub const NOT_MODIFIED: StatusCode = StatusCode(304);
    /// `305 Use Proxy`.
    pub const USE_PROXY: StatusCode = StatusCode(305);
    /// `307 Temporary Redirect`.
    pub const TEMPORARY_REDIRECT: StatusCode = StatusCode(307);
    /// `308 Permanent Redirect`.
    pub const PERMANENT_REDIRECT: StatusCode = StatusCode(308);

    /// `400 Bad Request`.
    pub const BAD_REQUEST: StatusCode = StatusCode(400);
    /// `401 Unauthorized`.
    pub const UNAUTHORIZED: StatusCode = StatusCode(401);
    /// `402 Payment Required`.
    pub const PAYMENT_REQUIRED: StatusCode = StatusCode(402);
    /// `403 Forbidden`.
    pub const FORBIDDEN: StatusCode = StatusCode(403);
    /// `404 Not Found`.
    pub const NOT_FOUND: StatusCode = StatusCode(404);
    /// `405 Method Not Allowed`.
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(405);
    /// `406 Not Acceptable`.
    pub const NOT_ACCEPTABLE: StatusCode = StatusCode(406);
    /// `407 Proxy Authentication Required`.
    pub const PROXY_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(407);
    /// `408 Request Timeout`.
    pub const REQUEST_TIMEOUT: StatusCode = StatusCode(408);
    /// `409 Conflict`.
    pub const CONFLICT: StatusCode = StatusCode(409);
    /// `410 Gone`.
    pub const GONE: StatusCode = StatusCode(410);
    /// `411 Length Required`.
    pub const LENGTH_REQUIRED: StatusCode = StatusCode(411);
    /// `412 Precondition Failed`.
    pub const PRECONDITION_FAILED: StatusCode = StatusCode(412);
    /// `413 Payload Too Large`.
    pub const PAYLOAD_TOO_LARGE: StatusCode = StatusCode(413);
    /// `414 URI Too Long`.
    pub const URI_TOO_LONG: StatusCode = StatusCode(414);
    /// `415 Unsupported Media Type`.
    pub const UNSUPPORTED_MEDIA_TYPE: StatusCode = StatusCode(415);
    /// `416 Range Not Satisfiable`.
    pub const RANGE_NOT_SATISFIABLE: StatusCode = StatusCode(416);
    /// `417 Expectation Failed`.
    pub const EXPECTATION_FAILED: StatusCode = StatusCode(417);
    /// `418 I'm a teapot`.
    pub const IM_A_TEAPOT: StatusCode = StatusCode(418);
    /// `421 Misdirected Request`.
    pub const MISDIRECTED_REQUEST: StatusCode = StatusCode(421);
    /// `422 Unprocessable Entity`.
    pub const UNPROCESSABLE_ENTITY: StatusCode = StatusCode(422);
    /// `423 Locked`.
    pub const LOCKED: StatusCode = StatusCode(423);
    /// `424 Failed Dependency`.
    pub const FAILED_DEPENDENCY: StatusCode = StatusCode(424);
    /// `425 Too Early`.
    pub const TOO_EARLY: StatusCode = StatusCode(425);
    /// `426 Upgrade Required`.
    pub const UPGRADE_REQUIRED: StatusCode = StatusCode(426);
    /// `428 Precondition Required`.
    pub const PRECONDITION_REQUIRED: StatusCode = StatusCode(428);
    /// `429 Too Many Requests`.
    pub const TOO_MANY_REQUESTS: StatusCode = StatusCode(429);
    /// `431 Request Header Fields Too Large`.
    pub const REQUEST_HEADER_FIELDS_TOO_LARGE: StatusCode = StatusCode(431);
    /// `451 Unavailable For Legal Reasons`.
    pub const UNAVAILABLE_FOR_LEGAL_REASONS: StatusCode = StatusCode(451);

    /// `500 Internal Server Error`.
    pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode(500);
    /// `501 Not Implemented`.
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(501);
    /// `502 Bad Gateway`.
    pub const BAD_GATEWAY: StatusCode = StatusCode(502);
    /// `503 Service Unavailable`.
    pub const SERVICE_UNAVAILABLE: StatusCode = StatusCode(503);
    /// `504 Gateway Timeout`.
    pub const GATEWAY_TIMEOUT: StatusCode = StatusCode(504);
    /// `505 HTTP Version Not Supported`.
    pub const HTTP_VERSION_NOT_SUPPORTED: StatusCode = StatusCode(505);
    /// `506 Variant Also Negotiates`.
    pub const VARIANT_ALSO_NEGOTIATES: StatusCode = StatusCode(506);
    /// `507 Insufficient Storage`.
    pub const INSUFFICIENT_STORAGE: StatusCode = StatusCode(507);
    /// `508 Loop Detected`.
    pub const LOOP_DETECTED: StatusCode = StatusCode(508);
    /// `510 Not Extended`.
    pub const NOT_EXTENDED: StatusCode = StatusCode(510);
    /// `511 Network Authentication Required`.
    pub const NETWORK_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(511);

    /// Wraps a raw status code. Never validates the range -- a peer can
    /// send anything in a status line, and rejecting it is the head
    /// parser's call, not this type's.
    pub fn from_u16(code: u16) -> Self {
        StatusCode(code)
    }

    /// The raw numeric status code.
    pub fn as_u16(&self) -> u16 {
        self.0
    }

    /// `1xx`.
    pub fn is_informational(&self) -> bool {
        (100..200).contains(&self.0)
    }

    /// `2xx`.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.0)
    }

    /// `3xx`.
    pub fn is_redirection(&self) -> bool {
        (300..400).contains(&self.0)
    }

    /// `4xx`.
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.0)
    }

    /// `5xx`.
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.0)
    }

    /// The standard reason phrase for this status code (e.g. `"Not Found"`
    /// for `404`), or `None` if the code isn't in the IANA registry.
    pub fn canonical_reason(&self) -> Option<&'static str> {
        let reason = match self.0 {
            100 => "Continue",
            101 => "Switching Protocols",
            102 => "Processing",
            103 => "Early Hints",
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            203 => "Non-Authoritative Information",
            204 => "No Content",
            205 => "Reset Content",
            206 => "Partial Content",
            207 => "Multi-Status",
            208 => "Already Reported",
            226 => "IM Used",
            300 => "Multiple Choices",
            301 => "Moved Permanently",
            302 => "Found",
            303 => "See Other",
            304 => "Not Modified",
            305 => "Use Proxy",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            401 => "Unauthorized",
            402 => "Payment Required",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            406 => "Not Acceptable",
            407 => "Proxy Authentication Required",
            408 => "Request Timeout",
            409 => "Conflict",
            410 => "Gone",
            411 => "Length Required",
            412 => "Precondition Failed",
            413 => "Payload Too Large",
            414 => "URI Too Long",
            415 => "Unsupported Media Type",
            416 => "Range Not Satisfiable",
            417 => "Expectation Failed",
            418 => "I'm a teapot",
            421 => "Misdirected Request",
            422 => "Unprocessable Entity",
            423 => "Locked",
            424 => "Failed Dependency",
            425 => "Too Early",
            426 => "Upgrade Required",
            428 => "Precondition Required",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            451 => "Unavailable For Legal Reasons",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            505 => "HTTP Version Not Supported",
            506 => "Variant Also Negotiates",
            507 => "Insufficient Storage",
            508 => "Loop Detected",
            510 => "Not Extended",
            511 => "Network Authentication Required",
            _ => return None,
        };
        Some(reason)
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_constants_match_their_numeric_value() {
        assert_eq!(StatusCode::OK.as_u16(), 200);
        assert_eq!(StatusCode::NOT_FOUND.as_u16(), 404);
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR.as_u16(), 500);
        assert_eq!(StatusCode::OK, StatusCode::from_u16(200));
    }

    #[test]
    fn canonical_reason_known_codes() {
        assert_eq!(StatusCode::OK.canonical_reason(), Some("OK"));
        assert_eq!(StatusCode::NOT_FOUND.canonical_reason(), Some("Not Found"));
        assert_eq!(
            StatusCode::IM_A_TEAPOT.canonical_reason(),
            Some("I'm a teapot")
        );
    }

    #[test]
    fn canonical_reason_unknown_code_is_none() {
        assert_eq!(StatusCode::from_u16(999).canonical_reason(), None);
        assert_eq!(StatusCode::from_u16(0).canonical_reason(), None);
    }
}
