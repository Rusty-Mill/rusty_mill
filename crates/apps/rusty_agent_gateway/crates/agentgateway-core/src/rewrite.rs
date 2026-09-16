//! Compiled URL rewrites.
//!
//! `urlRewrite` names parts of the one address the gateway dials, and it now
//! names them for four backend kinds: the HTTP proxy's upstream, an MCP
//! target, an `ai` provider endpoint, and — through the proxy — an `a2a`
//! agent. The rules for trimming slashes and anchoring a `prefix` live here
//! once, because a `prefix` that behaved differently on one path than another
//! would be a difference nobody could predict from the configuration.

use agentgateway_config::{PathRewrite, UrlRewrite};
use http::uri::Authority;

/// A compiled [`UrlRewrite`].
#[derive(Debug, Clone, Default)]
pub struct Rewrite {
    authority: Option<Authority>,
    path: Option<PathRewrite>,
}

/// Failure to compile a rewrite.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{value}` is not a valid authority{because}")]
pub struct RewriteError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
    /// Why, when there is something useful to add.
    pub because: &'static str,
}

impl Rewrite {
    /// Compile a rewrite policy.
    pub fn new(rewrite: &UrlRewrite, at: &str) -> Result<Self, RewriteError> {
        let authority = match &rewrite.authority {
            Some(value) => Some(parse_authority(value, at)?),
            None => None,
        };
        Ok(Rewrite {
            authority,
            path: rewrite.path.clone(),
        })
    }

    /// The authority this rewrite forces, if any.
    pub fn authority(&self) -> Option<&Authority> {
        self.authority.as_ref()
    }

    /// The same rewrite with its authority dropped.
    ///
    /// For a route whose upstream is chosen per request: a forced authority
    /// there would mean the request never chooses one, which is the opposite
    /// of what the backend is for. The path half still applies.
    pub fn without_authority(mut self) -> Self {
        self.authority = None;
        self
    }

    /// Rewrite a path, given the route prefix that matched.
    ///
    /// `matched_prefix` is what a `prefix` rewrite replaces; without it the
    /// rewrite has nothing to anchor on and the path is left alone rather than
    /// mangled.
    pub fn path(&self, path: &str, matched_prefix: Option<&str>) -> Option<String> {
        match self.path.as_ref()? {
            PathRewrite::Full(replacement) => Some(replacement.clone()),
            PathRewrite::Prefix(replacement) => {
                let prefix = matched_prefix?;
                let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
                let rest = path.strip_prefix(prefix).unwrap_or(path);
                let replacement = replacement.strip_suffix('/').unwrap_or(replacement);

                let rewritten = match (replacement.is_empty(), rest.is_empty()) {
                    // Replacing `/api` with `/` on a request for `/api` must
                    // produce `/`, not an empty path, which is not a valid
                    // origin-form target.
                    (true, true) => "/".to_string(),
                    (true, false) => rest.to_string(),
                    (false, true) => replacement.to_string(),
                    (false, false) => format!("{replacement}{rest}"),
                };
                Some(rewritten)
            }
        }
    }
}

/// Parse a replacement authority, refusing one that carries a credential.
///
/// An authority may legally hold `user:password@`, and that is exactly the
/// problem: a credential in an upstream address hides somewhere nobody thinks
/// to look and is sent on every request. `backendAuth` is where one belongs.
pub fn parse_authority(value: &str, at: &str) -> Result<Authority, RewriteError> {
    if value.contains('@') {
        return Err(RewriteError {
            at: at.to_string(),
            value: value.to_string(),
            because: ": userinfo does not belong in an upstream address, use `backendAuth`",
        });
    }
    Authority::try_from(value).map_err(|_| RewriteError {
        at: at.to_string(),
        value: value.to_string(),
        because: "",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(authority: Option<&str>, path: Option<PathRewrite>) -> Rewrite {
        Rewrite::new(
            &UrlRewrite {
                authority: authority.map(|a| a.to_string()),
                path,
            },
            "test",
        )
        .expect("should compile")
    }

    #[test]
    fn a_full_rewrite_replaces_the_whole_path() {
        let compiled = rewrite(None, Some(PathRewrite::Full("/v1/rpc".into())));
        assert_eq!(
            compiled.path("/anything/at/all", Some("/anything")),
            Some("/v1/rpc".to_string())
        );
    }

    #[test]
    fn a_prefix_rewrite_replaces_only_the_matched_prefix() {
        let compiled = rewrite(None, Some(PathRewrite::Prefix("/v1".into())));
        assert_eq!(
            compiled.path("/api/things", Some("/api")),
            Some("/v1/things".to_string())
        );
    }

    #[test]
    fn a_prefix_rewrite_with_nothing_to_anchor_on_leaves_the_path_alone() {
        // Better than mangling it: without the matched prefix there is no way
        // to know which part of the path the replacement is meant to replace.
        let compiled = rewrite(None, Some(PathRewrite::Prefix("/v1".into())));
        assert_eq!(compiled.path("/api/things", None), None);
    }

    #[test]
    fn replacing_a_prefix_with_a_root_still_produces_a_valid_target() {
        let compiled = rewrite(None, Some(PathRewrite::Prefix("/".into())));
        assert_eq!(compiled.path("/api", Some("/api")), Some("/".to_string()));
        assert_eq!(
            compiled.path("/api/things", Some("/api")),
            Some("/things".to_string())
        );
    }

    #[test]
    fn an_authority_carrying_a_credential_is_refused() {
        // It would be sent on every request from a place nobody reads.
        let err = Rewrite::new(
            &UrlRewrite {
                authority: Some("user:secret@upstream:8080".into()),
                path: None,
            },
            "route[0].urlRewrite",
        )
        .expect_err("should not compile");
        assert!(err.to_string().contains("backendAuth"), "got: {err}");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
    }

    #[test]
    fn an_authority_http_rejects_names_where_it_came_from() {
        let err = Rewrite::new(
            &UrlRewrite {
                authority: Some("not a host".into()),
                path: None,
            },
            "route[0].urlRewrite",
        )
        .expect_err("should not compile");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
    }
}
