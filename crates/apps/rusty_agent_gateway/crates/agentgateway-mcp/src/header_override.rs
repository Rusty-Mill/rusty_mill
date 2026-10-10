//! A guardrail's `headerMutation`, as it travels with one upstream call.
//!
//! `mcpGuardrails` lets a processor change the headers of the upstream HTTP
//! request that carries an MCP call. The change is handed to the native client
//! with the call ([`HeaderOverride::to_native`]), which folds it into that one
//! request.
//!
//! # Only `mcp:` targets
//!
//! A `stdio` target speaks over a pipe: there is no HTTP request, so there are
//! no headers to change. Upstream says the same, and a mutation aimed at one is
//! dropped rather than quietly appearing somewhere else.

use http::{HeaderName, HeaderValue};

/// Header changes attached to one outgoing MCP request.
#[derive(Debug, Clone, Default)]
pub struct HeaderOverride {
    /// Headers to add or overwrite.
    pub set: Vec<(HeaderName, HeaderValue)>,
    /// Header names to drop.
    pub remove: Vec<HeaderName>,
}

impl HeaderOverride {
    /// Whether there is anything to do.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }
}

impl HeaderOverride {
    /// The same change for the native client. A value that is not text is
    /// dropped: it could not be a legitimate MCP header.
    pub fn to_native(&self) -> rusty_mcp_client_native::HeaderOverride {
        rusty_mcp_client_native::HeaderOverride {
            set: self
                .set
                .iter()
                .filter_map(|(n, v)| Some((n.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                .collect(),
            remove: self.remove.iter().map(|n| n.as_str().to_owned()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> HeaderName {
        HeaderName::try_from(text).expect("valid header name")
    }

    fn value(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).expect("valid header value")
    }

    #[test]
    fn it_converts_for_the_native_client() {
        let native = HeaderOverride {
            set: vec![(name("x-user"), value("u-1"))],
            remove: vec![name("x-internal")],
        }
        .to_native();
        assert_eq!(native.set, [("x-user".to_owned(), "u-1".to_owned())]);
        assert_eq!(native.remove, ["x-internal".to_owned()]);
    }

    #[test]
    fn a_value_that_is_not_text_is_dropped() {
        let binary = HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque bytes are allowed");
        let native = HeaderOverride {
            set: vec![(name("x-bin"), binary), (name("x-ok"), value("1"))],
            remove: Vec::new(),
        }
        .to_native();
        assert_eq!(native.set, [("x-ok".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn an_empty_override_is_empty() {
        assert!(HeaderOverride::default().is_empty());
        assert!(HeaderOverride::default().to_native().is_empty());
    }
}
