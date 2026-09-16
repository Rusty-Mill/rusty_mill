//! Request predicates.
//!
//! These mirror Gateway API's `HTTPRouteMatch`, which is where agentgateway
//! takes them from. The types are pure data: regexes are stored as strings and
//! compiled once by the router, so a config can be cloned, compared and
//! serialized without dragging compiled state along. [`RouteMatch::validate`]
//! is what proves those strings compile, and it runs at load time so a bad
//! pattern is a startup error rather than a per-request surprise.

use serde::{Deserialize, Serialize};

use crate::ConfigError;
use crate::oneof::one_of_enum;

/// A conjunction of predicates. Every populated field must match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteMatch {
    /// Path predicate. Defaults to a `/` prefix match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathMatch>,

    /// Header predicates, all of which must match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderMatch>,

    /// HTTP method predicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    /// Query parameter predicates, all of which must match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<QueryMatch>,
}

impl RouteMatch {
    /// Compile every regex in this match, reporting the first bad pattern.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(PathMatch::Regex(pattern)) = &self.path {
            compile(pattern, "path")?;
        }
        for header in &self.headers {
            if let HeaderMatchValue::Regex(pattern) = &header.value {
                compile(pattern, &format!("header {}", header.name))?;
            }
        }
        for query in &self.query {
            if let QueryMatchValue::Regex(pattern) = &query.value {
                compile(pattern, &format!("query {}", query.name))?;
            }
        }
        Ok(())
    }
}

fn compile(pattern: &str, what: &str) -> Result<regex::Regex, ConfigError> {
    regex::Regex::new(pattern)
        .map_err(|err| ConfigError::Invalid(format!("{what} regex `{pattern}`: {err}")))
}

one_of_enum! {
    /// How to match the request path.
    ///
    /// Written as a single-key map, so `{pathPrefix: /a}` and `{exact: /a}` are
    /// told apart by which key is present, matching upstream.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PathMatch {
        /// Full, exact path equality.
        "exact" => Exact(String),
        /// Match on whole path segments. `/api` matches `/api` and `/api/v1`,
        /// but not `/apixyz` — segment-aware, per Gateway API, not a raw
        /// `starts_with`.
        "pathPrefix" => PathPrefix(String),
        /// Unanchored regular expression over the path.
        "regex" => Regex(String),
    }
}

impl Default for PathMatch {
    fn default() -> Self {
        PathMatch::PathPrefix("/".into())
    }
}

/// A header name plus the condition its value must satisfy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderMatch {
    /// Header name. Compared case-insensitively, as HTTP requires.
    pub name: String,

    /// The condition on the value.
    #[serde(flatten)]
    pub value: HeaderMatchValue,
}

one_of_enum! {
    /// The condition a header value must satisfy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum HeaderMatchValue {
        /// Exact, case-sensitive value equality.
        "exact" => Exact(String),
        /// Unanchored regular expression over the value.
        "regex" => Regex(String),
        /// Header must be present, with any value.
        "present" => Present(bool),
    }
}

/// A query parameter name plus the condition its value must satisfy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryMatch {
    /// Query parameter name.
    pub name: String,

    /// The condition on the value.
    #[serde(flatten)]
    pub value: QueryMatchValue,
}

one_of_enum! {
    /// The condition a query parameter value must satisfy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum QueryMatchValue {
        /// Exact value equality.
        "exact" => Exact(String),
        /// Unanchored regular expression over the value.
        "regex" => Regex(String),
        /// Parameter must be present, with any value.
        "present" => Present(bool),
    }
}
