//! External authorization.
//!
//! Before a request is served, an authorization service is asked whether it
//! may be. A 2xx allows it; anything else denies it, and the authorizer's own
//! status and body are returned to the caller so the reason survives — an
//! authorizer that answers `403 {"reason": "not in group"}` is telling the
//! caller something a generic "forbidden" would throw away.
//!
//! # Fail closed
//!
//! When the authorizer cannot be reached, the request is **denied** unless
//! `failOpen` says otherwise. An authorization service that is down must not
//! become an open door, so serving traffic it never approved has to be a
//! deliberate choice rather than the default. `failOpen: true` exists because
//! some deployments would rather serve than stall — but they have to say so.
//!
//! # Both header lists are allow-lists
//!
//! Forwarding every request header would hand the authorizer cookies and
//! payloads it has no need for, widening what a compromised one can read.
//!
//! The other direction matters more. `allowedUpstreamHeaders` bounds what the
//! authorizer may set on the request that continues upstream; without it, an
//! authorizer could write any header the upstream trusts — `x-user-id`,
//! `x-is-admin` — which turns an authorization service into an impersonation
//! service. Anything not on the list is dropped.

use std::time::Duration;

use agentgateway_config::ExtAuthzPolicy;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};

/// Failure to build an authorizer from configuration.
#[derive(Debug, thiserror::Error)]
pub enum ExtAuthzError {
    /// A header name HTTP cannot represent.
    #[error("{at}: `{value}` is not a valid header name")]
    HeaderName {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        value: String,
    },

    /// The target could not be parsed as a URL.
    #[error("{at}: `{target}` is not a valid authorization service URL")]
    Target {
        /// Where in the configuration it came from.
        at: String,
        /// The offending text.
        target: String,
    },
}

/// Budget for an authorization call when the policy names none.
///
/// This sits in front of every request on the route, so a slow authorizer is a
/// slow gateway.
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(250);

/// What the authorizer decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorization {
    /// Serve the request, after adding these headers to it.
    Allow(Vec<(HeaderName, HeaderValue)>),
    /// Refuse, answering with this status, headers and body.
    Deny {
        /// Status to answer with.
        status: StatusCode,
        /// Headers from the authorizer worth passing on.
        headers: HeaderMap,
        /// The authorizer's body, so its reason reaches the caller.
        body: Vec<u8>,
    },
}

/// An external authorization service.
pub struct ExtAuthz {
    target: String,
    include: Vec<HeaderName>,
    allowed_upstream: Vec<HeaderName>,
    fail_open: bool,
    client: reqwest::Client,
}

impl std::fmt::Debug for ExtAuthz {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtAuthz")
            .field("target", &self.target)
            .field("fail_open", &self.fail_open)
            .finish_non_exhaustive()
    }
}

impl ExtAuthz {
    /// Build an authorizer from a route's policy.
    pub fn new(policy: &ExtAuthzPolicy, at: &str) -> Result<Self, ExtAuthzError> {
        let target = policy.target.trim_end_matches('/').to_string();
        if target.parse::<Uri>().is_err() || !target.starts_with("http") {
            return Err(ExtAuthzError::Target {
                at: at.to_string(),
                target: policy.target.clone(),
            });
        }

        let names = |values: &[String], field: &str| -> Result<Vec<HeaderName>, ExtAuthzError> {
            values
                .iter()
                .map(|value| {
                    HeaderName::try_from(value.as_str()).map_err(|_| ExtAuthzError::HeaderName {
                        at: format!("{at}.extAuthz.{field}"),
                        value: value.clone(),
                    })
                })
                .collect()
        };

        let timeout = policy.timeout.map(Duration::from).unwrap_or(DEFAULT_TIMEOUT);

        Ok(ExtAuthz {
            target,
            include: names(&policy.include_headers, "includeHeaders")?,
            allowed_upstream: names(&policy.allowed_upstream_headers, "allowedUpstreamHeaders")?,
            fail_open: policy.fail_open.unwrap_or(false),
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
        })
    }

    /// Ask the authorizer about a request.
    ///
    /// The original method and path are used for the call, so the authorizer
    /// sees what is being authorized and can route on it, rather than having
    /// to read it back out of a header.
    pub async fn check(&self, method: &Method, path: &str, headers: &HeaderMap) -> Authorization {
        let url = format!("{}{}", self.target, path);
        let mut request = self.client.request(method.clone(), &url);

        for name in &self.include {
            if let Some(value) = headers.get(name) {
                request = request.header(name, value);
            }
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    target = %self.target,
                    fail_open = self.fail_open,
                    %err,
                    "the authorization service could not be reached"
                );
                return self.unreachable();
            }
        };

        let status = StatusCode::from_u16(response.status().as_u16())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let response_headers = response.headers().clone();

        if status.is_success() {
            return Authorization::Allow(self.upstream_headers(&response_headers));
        }

        // The authorizer's own body is what tells the caller why. Passing it
        // through unchanged beats replacing it with a generic refusal.
        let body = response.bytes().await.unwrap_or_default().to_vec();
        Authorization::Deny {
            status,
            headers: passthrough_headers(&response_headers),
            body,
        }
    }

    /// What to do when the authorizer never answered.
    fn unreachable(&self) -> Authorization {
        if self.fail_open {
            return Authorization::Allow(Vec::new());
        }
        Authorization::Deny {
            // 503, not 403: nothing decided this request was forbidden. Saying
            // "forbidden" would send someone to check their permissions when
            // the real problem is a service being down.
            status: StatusCode::SERVICE_UNAVAILABLE,
            headers: HeaderMap::new(),
            body: b"the authorization service could not be reached".to_vec(),
        }
    }

    /// The subset of the authorizer's headers allowed onto the upstream call.
    fn upstream_headers(&self, from: &reqwest::header::HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
        self.allowed_upstream
            .iter()
            .filter_map(|name| {
                let value = from.get(name.as_str())?;
                let value = HeaderValue::from_bytes(value.as_bytes()).ok()?;
                Some((name.clone(), value))
            })
            .collect()
    }
}

/// Headers worth relaying from a denial.
///
/// `WWW-Authenticate` is the one that matters: a `401` without it leaves the
/// client no way to learn how to authenticate. Content headers are left to the
/// caller, which sets them from the body it actually sends.
fn passthrough_headers(from: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for name in [http::header::WWW_AUTHENTICATE, http::header::CONTENT_TYPE] {
        if let Some(value) = from.get(name.as_str())
            && let Ok(value) = HeaderValue::from_bytes(value.as_bytes())
        {
            headers.insert(name, value);
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(target: &str) -> ExtAuthzPolicy {
        ExtAuthzPolicy {
            target: target.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_target_that_is_not_a_url_fails_at_startup() {
        let err = ExtAuthz::new(&policy("authz:9000"), "binds[0]")
            .expect_err("a bare host:port is not a URL");
        assert!(err.to_string().contains("binds[0]"), "got: {err}");

        ExtAuthz::new(&policy("http://authz:9000"), "t").expect("a real URL should build");
    }

    #[test]
    fn a_trailing_slash_does_not_produce_a_double_one() {
        let authz = ExtAuthz::new(&policy("http://authz:9000/"), "t").expect("should build");
        assert_eq!(authz.target, "http://authz:9000");
    }

    #[test]
    fn an_invalid_header_name_names_which_list_it_came_from() {
        let err = ExtAuthz::new(
            &ExtAuthzPolicy {
                target: "http://authz:9000".into(),
                allowed_upstream_headers: vec!["not a header".into()],
                ..Default::default()
            },
            "route[0]",
        )
        .expect_err("should not build");
        assert!(
            err.to_string().contains("allowedUpstreamHeaders"),
            "got: {err}"
        );
    }

    #[test]
    fn failing_closed_is_the_default() {
        // An authorization service that is down must not become an open door.
        let authz = ExtAuthz::new(&policy("http://authz:9000"), "t").expect("should build");
        assert!(!authz.fail_open);

        match authz.unreachable() {
            Authorization::Deny { status, .. } => assert_eq!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "nothing decided this request was forbidden, so 503 rather than 403"
            ),
            Authorization::Allow(_) => panic!("an unreachable authorizer must not allow"),
        }
    }

    #[test]
    fn failing_open_has_to_be_asked_for() {
        let authz = ExtAuthz::new(
            &ExtAuthzPolicy {
                target: "http://authz:9000".into(),
                fail_open: Some(true),
                ..Default::default()
            },
            "t",
        )
        .expect("should build");

        assert_eq!(authz.unreachable(), Authorization::Allow(Vec::new()));
    }

    #[test]
    fn only_allow_listed_headers_reach_the_upstream() {
        // Without the list an authorizer could set any header the upstream
        // trusts, which turns authorization into impersonation.
        let authz = ExtAuthz::new(
            &ExtAuthzPolicy {
                target: "http://authz:9000".into(),
                allowed_upstream_headers: vec!["x-user-id".into()],
                ..Default::default()
            },
            "t",
        )
        .expect("should build");

        let mut from = reqwest::header::HeaderMap::new();
        from.insert("x-user-id", "u-1".parse().expect("valid"));
        from.insert("x-is-admin", "true".parse().expect("valid"));

        let allowed = authz.upstream_headers(&from);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].0.as_str(), "x-user-id");
        assert_eq!(allowed[0].1, "u-1");
    }

    #[test]
    fn nothing_reaches_the_upstream_without_a_list() {
        let authz = ExtAuthz::new(&policy("http://authz:9000"), "t").expect("should build");
        let mut from = reqwest::header::HeaderMap::new();
        from.insert("x-user-id", "u-1".parse().expect("valid"));
        assert!(
            authz.upstream_headers(&from).is_empty(),
            "an empty allow-list allows nothing, not everything"
        );
    }

    #[test]
    fn a_denial_relays_the_authenticate_challenge() {
        // A 401 without it leaves the client no way to learn how to
        // authenticate.
        let mut from = reqwest::header::HeaderMap::new();
        from.insert("www-authenticate", "Bearer".parse().expect("valid"));
        from.insert("x-internal", "leak".parse().expect("valid"));

        let headers = passthrough_headers(&from);
        assert_eq!(
            headers
                .get(http::header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some("Bearer")
        );
        assert!(
            headers.get("x-internal").is_none(),
            "an authorizer's internal headers are not the caller's business"
        );
    }
}
