//! GitHub adapter for the [`Forge`] port (REST v3, issues only).
//!
//! Blocking: see [`crate::http`] (call it from a plain thread).

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::http::Http;
use crate::sync::{Draft, Forge, Issue, SyncError};

const PAGE: usize = 100;
/// Longest slice of an error body kept in a message.
const BODY_SNIPPET: usize = 200;

/// A repository and the token to reach it.
#[derive(Clone)]
pub struct GithubConfig {
    api: String,
    repo: String,
    token: String,
}

impl fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubConfig")
            .field("api", &self.api)
            .field("repo", &self.repo)
            .field("token", &"***redacted***")
            .finish()
    }
}

impl GithubConfig {
    /// `repo` is `owner/name`; `api` is the REST base, e.g. `https://api.github.com`.
    ///
    /// # Errors
    /// If the repo is not `owner/name` of plain name characters (it goes into
    /// a URL path), the API base is not http(s), or the token is empty.
    pub fn new(api: &str, repo: &str, token: &str) -> Result<Self, String> {
        // Both segments go into a URL path: plain name characters, and never `.`/`..`.
        let plain = |s: &str| {
            s != "."
                && s != ".."
                && !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        };
        match repo.split_once('/') {
            Some((owner, name)) if plain(owner) && plain(name) => {}
            _ => return Err("repo must look like owner/name".into()),
        }
        if !(api.starts_with("https://") || api.starts_with("http://")) {
            return Err("API base must start with https:// or http://".into());
        }
        if token.trim().is_empty() {
            return Err("GitHub token is empty".into());
        }
        Ok(Self {
            api: api.trim_end_matches('/').to_owned(),
            repo: repo.to_owned(),
            token: token.trim().to_owned(),
        })
    }
}

/// The forge over GitHub issues.
pub struct GithubForge {
    config: GithubConfig,
    http: Http,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Deserialize)]
struct RawIssue {
    number: u64,
    title: String,
    body: Option<String>,
    labels: Vec<RawLabel>,
    state: String,
    pull_request: Option<serde_json::Value>,
}

impl RawIssue {
    fn into_issue(self) -> Issue {
        Issue {
            number: self.number,
            title: self.title,
            body: self.body.unwrap_or_default(),
            labels: self.labels.into_iter().map(|l| l.name).collect(),
            closed: self.state == "closed",
        }
    }
}

fn forge(why: impl fmt::Display) -> SyncError {
    SyncError::Forge(why.to_string())
}

impl GithubForge {
    /// Build the client. Nothing connects until the first call.
    ///
    /// # Errors
    /// If the private runtime cannot start.
    pub fn new(config: GithubConfig) -> Result<Self, SyncError> {
        Ok(Self {
            config,
            http: Http::new(Duration::from_secs(30)).map_err(forge)?,
        })
    }

    fn url(&self, tail: &str) -> String {
        format!(
            "{}/repos/{}/issues{tail}",
            self.config.api, self.config.repo
        )
    }

    /// One request; returns the body of a 2xx, else an error naming the status.
    fn call(&self, method: &str, url: &str, body: Option<String>) -> Result<String, SyncError> {
        let auth = format!("Bearer {}", self.config.token);
        let headers = [
            ("Authorization", auth.as_str()),
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
            ("User-Agent", "rusty_mill_board"),
        ];
        let (status, text) = self.http.send(method, url, &headers, body).map_err(forge)?;
        if (200..300).contains(&status) {
            return Ok(text);
        }
        let snippet: String = text.chars().take(BODY_SNIPPET).collect();
        Err(SyncError::Forge(format!(
            "{method} {} -> {status}: {snippet}",
            url_path(url)
        )))
    }

    fn one(&self, method: &str, url: &str, body: serde_json::Value) -> Result<Issue, SyncError> {
        let text = self.call(method, url, Some(body.to_string()))?;
        Ok(serde_json::from_str::<RawIssue>(&text)
            .map_err(forge)?
            .into_issue())
    }
}

/// Path part of a URL, for messages (keeps host details out of them).
fn url_path(url: &str) -> &str {
    url.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or(url)
}

fn payload(d: &Draft) -> serde_json::Value {
    json!({ "title": d.title, "body": d.body, "labels": d.labels, "state": if d.closed { "closed" } else { "open" } })
}

impl Forge for GithubForge {
    fn list_issues(&self) -> Result<Vec<Issue>, SyncError> {
        let mut out = Vec::new();
        for page in 1.. {
            let text = self.call(
                "GET",
                &self.url(&format!("?state=all&per_page={PAGE}&page={page}")),
                None,
            )?;
            let raw: Vec<RawIssue> = serde_json::from_str(&text).map_err(forge)?;
            let n = raw.len();
            out.extend(
                raw.into_iter()
                    .filter(|i| i.pull_request.is_none())
                    .map(RawIssue::into_issue),
            );
            if n < PAGE {
                break;
            }
        }
        Ok(out)
    }

    fn create_issue(&self, draft: &Draft) -> Result<Issue, SyncError> {
        // `state` is not accepted on create; a Done card without an issue is created, then closed.
        let created = self.one(
            "POST",
            &self.url(""),
            json!({ "title": draft.title, "body": draft.body, "labels": draft.labels }),
        )?;
        if draft.closed {
            self.update_issue(created.number, draft)
        } else {
            Ok(created)
        }
    }

    fn update_issue(&self, number: u64, draft: &Draft) -> Result<Issue, SyncError> {
        self.one("PATCH", &self.url(&format!("/{number}")), payload(draft))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validates_inputs_and_redacts_token() {
        assert!(GithubConfig::new("https://api.github.com/", "owner/name", "t").is_ok());
        assert!(GithubConfig::new("https://api.github.com", "owner", "t").is_err());
        assert!(GithubConfig::new("https://api.github.com", "a/b/c", "t").is_err());
        assert!(GithubConfig::new("https://api.github.com", "../x", "t").is_err());
        assert!(GithubConfig::new("https://api.github.com", "x/..", "t").is_err());
        assert!(GithubConfig::new("https://api.github.com", "x/.github", "t").is_ok());
        assert!(GithubConfig::new("ftp://x", "a/b", "t").is_err());
        assert!(GithubConfig::new("https://x", "a/b", " ").is_err());
        let shown = format!(
            "{:?}",
            GithubConfig::new("https://x", "a/b", "hunter2").expect("config")
        );
        assert!(!shown.contains("hunter2"));
    }

    #[test]
    fn url_path_hides_host() {
        assert_eq!(
            url_path("https://api.github.com/repos/a/b/issues?x=1"),
            "/repos/a/b/issues?x=1"
        );
    }
}
