//! Compiled header modifiers.
//!
//! A `requestHeaderModifier` or `responseHeaderModifier` is the same operation
//! wherever it lands, and it now lands in three places: the HTTP proxy's
//! upstream request, the gateway's response for every backend kind, and the
//! `ai` backend's request to a model provider. One implementation rather than
//! three, so a `set` that replaces on one path cannot append on another.

use agentgateway_config::HeaderModifier;
use http::{HeaderMap, HeaderName, HeaderValue};

/// A header name or value in the configuration that HTTP cannot represent.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{value}` is not a valid HTTP {kind}")]
pub struct HeaderError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
    /// `header name` or `header value`.
    pub kind: &'static str,
}

/// A compiled [`HeaderModifier`].
#[derive(Debug, Clone, Default)]
pub struct Headers {
    add: Vec<(HeaderName, HeaderValue)>,
    set: Vec<(HeaderName, HeaderValue)>,
    remove: Vec<HeaderName>,
}

impl Headers {
    /// Compile a modifier, reporting the first name or value HTTP rejects.
    pub fn new(modifier: &HeaderModifier, at: &str) -> Result<Self, HeaderError> {
        let pair =
            |name: &String, value: &String| -> Result<(HeaderName, HeaderValue), HeaderError> {
                let header = HeaderName::try_from(name.as_str()).map_err(|_| HeaderError {
                    at: at.to_string(),
                    value: name.clone(),
                    kind: "header name",
                })?;
                let value = HeaderValue::try_from(value.as_str()).map_err(|_| HeaderError {
                    at: at.to_string(),
                    value: value.clone(),
                    kind: "header value",
                })?;
                Ok((header, value))
            };

        let mut add = Vec::with_capacity(modifier.add.len());
        for (name, value) in &modifier.add {
            add.push(pair(name, value)?);
        }
        let mut set = Vec::with_capacity(modifier.set.len());
        for (name, value) in &modifier.set {
            set.push(pair(name, value)?);
        }
        let mut remove = Vec::with_capacity(modifier.remove.len());
        for name in &modifier.remove {
            remove.push(
                HeaderName::try_from(name.as_str()).map_err(|_| HeaderError {
                    at: at.to_string(),
                    value: name.clone(),
                    kind: "header name",
                })?,
            );
        }

        Ok(Headers { add, set, remove })
    }

    /// Apply the modifier.
    ///
    /// Order follows Gateway API: `set` replaces, `add` appends, `remove`
    /// wins over both. Removing last means a config that both sets and removes
    /// a header removes it, which is the reading that fails safe.
    pub fn apply(&self, headers: &mut HeaderMap) {
        for (name, value) in &self.set {
            headers.insert(name, value.clone());
        }
        for (name, value) in &self.add {
            headers.append(name, value.clone());
        }
        for name in &self.remove {
            headers.remove(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn modifier(add: &[(&str, &str)], set: &[(&str, &str)], remove: &[&str]) -> HeaderModifier {
        let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        HeaderModifier {
            add: map(add),
            set: map(set),
            remove: remove.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::try_from(*name).expect("a valid name"),
                HeaderValue::try_from(*value).expect("a valid value"),
            );
        }
        map
    }

    #[test]
    fn set_replaces_and_add_appends() {
        let compiled = Headers::new(
            &modifier(&[("x-multi", "second")], &[("x-one", "new")], &[]),
            "test",
        )
        .expect("should compile");

        let mut map = headers(&[("x-one", "old"), ("x-multi", "first")]);
        compiled.apply(&mut map);

        assert_eq!(map.get_all("x-one").iter().count(), 1);
        assert_eq!(map.get("x-one").and_then(|v| v.to_str().ok()), Some("new"));
        assert_eq!(
            map.get_all("x-multi").iter().count(),
            2,
            "add appends rather than replacing"
        );
    }

    #[test]
    fn remove_wins_over_set() {
        // A config that both sets and removes a header should end up without
        // it: that is the reading that fails safe.
        let compiled = Headers::new(
            &modifier(&[], &[("x-secret", "value")], &["x-secret"]),
            "test",
        )
        .expect("should compile");

        let mut map = HeaderMap::new();
        compiled.apply(&mut map);
        assert!(map.get("x-secret").is_none());
    }

    #[test]
    fn an_invalid_header_name_fails_to_compile() {
        let err = Headers::new(&modifier(&[], &[("not a header", "v")], &[]), "route[0]")
            .expect_err("should not compile");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
    }

    #[test]
    fn a_value_http_rejects_names_where_it_came_from() {
        let err = Headers::new(&modifier(&[], &[("x-c", "bad\nvalue")], &[]), "route[0]")
            .expect_err("should not compile");
        assert!(err.to_string().contains("header value"), "got: {err}");
    }
}
