//! GitHub adapter for the [`Forge`] port (REST v3, issues only).
//!
//! Blocking: see [`crate::http`] (call it from a plain thread).

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::backfill::{Lookup, Merged};
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
        match repo.split_once('/') {
            Some((owner, name)) if plain(owner) && plain(name) => {}
            _ => return Err("repo must look like owner/name".into()),
        }
        let (api, token) = base(api, token)?;
        Ok(Self {
            api,
            repo: repo.to_owned(),
            token,
        })
    }
}

/// A URL path segment: plain name characters, and never `.`/`..`.
fn plain(s: &str) -> bool {
    s != "."
        && s != ".."
        && !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A checked API base (no trailing `/`) and token.
fn base(api: &str, token: &str) -> Result<(String, String), String> {
    if !(api.starts_with("https://") || api.starts_with("http://")) {
        return Err("API base must start with https:// or http://".into());
    }
    if token.trim().is_empty() {
        return Err("GitHub token is empty".into());
    }
    Ok((
        api.trim_end_matches('/').to_owned(),
        token.trim().to_owned(),
    ))
}

/// `(status, body)` of one GitHub REST request.
fn send(
    http: &Http,
    token: &str,
    method: &str,
    url: &str,
    body: Option<String>,
) -> Result<(u16, String), String> {
    let auth = format!("Bearer {token}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
        ("User-Agent", "rusty_mill_board"),
    ];
    http.send(method, url, &headers, body)
}

/// The backfill's merge check over `GET /repos/OWNER/NAME/pulls/N` (read-only; the token
/// needs pull requests read on each repository).
pub struct GithubMerged {
    api: String,
    token: String,
    http: Http,
}

impl fmt::Debug for GithubMerged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubMerged")
            .field("api", &self.api)
            .field("token", &"***redacted***")
            .finish()
    }
}

#[derive(Deserialize)]
struct RawPull {
    merged: bool,
}

impl GithubMerged {
    /// `api` is the REST base, e.g. `https://api.github.com`.
    ///
    /// # Errors
    /// If the API base is not http(s), the token is empty, or the runtime cannot start.
    pub fn new(api: &str, token: &str) -> Result<Self, String> {
        let (api, token) = base(api, token)?;
        Ok(Self {
            api,
            token,
            http: Http::new(Duration::from_secs(30))?,
        })
    }
}

impl Merged for GithubMerged {
    fn merged(&self, owner: &str, name: &str, number: u64) -> Result<bool, Lookup> {
        if !plain(owner) || !plain(name) {
            return Err(Lookup::Failed(format!("{owner}/{name}: not a repository")));
        }
        let url = format!("{}/repos/{owner}/{name}/pulls/{number}", self.api);
        let (status, text) =
            send(&self.http, &self.token, "GET", &url, None).map_err(Lookup::Failed)?;
        match status {
            200 => serde_json::from_str::<RawPull>(&text)
                .map(|p| p.merged)
                .map_err(|e| Lookup::Failed(e.to_string())),
            404 => Err(Lookup::Missing),
            _ => {
                let snippet: String = text.chars().take(BODY_SNIPPET).collect();
                Err(Lookup::Failed(format!(
                    "GET {} -> {status}: {snippet}",
                    url_path(&url)
                )))
            }
        }
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
        let (status, text) =
            send(&self.http, &self.config.token, method, url, body).map_err(forge)?;
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

    /// Serve one canned response per connection, in order, on a loopback port; returns the
    /// API base and the request lines seen.
    fn serve(replies: &[(u16, &str)]) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let replies: Vec<(u16, String)> = replies.iter().map(|(c, b)| (*c, (*b).into())).collect();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (code, body) in replies {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("request line");
                seen.push(line.trim().to_owned());
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).expect("header");
                    if header.trim().is_empty() {
                        break;
                    }
                    if header.to_ascii_lowercase().starts_with("authorization:") {
                        assert_eq!(header.trim(), "Authorization: Bearer hunter2");
                    }
                }
                let reply = format!(
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).expect("reply");
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn merged_reads_the_pull_request_and_tells_missing_from_failed() {
        let (base, server) = serve(&[
            (200, r#"{"number":1,"merged":true}"#),
            (200, r#"{"number":2,"merged":false}"#),
            (404, r#"{"message":"Not Found"}"#),
            (403, r#"{"message":"API rate limit exceeded"}"#),
        ]);
        let forge = GithubMerged::new(&base, "hunter2").expect("client");
        assert!(!format!("{forge:?}").contains("hunter2"), "token redacted");
        assert_eq!(forge.merged("o", "r", 1), Ok(true));
        assert_eq!(forge.merged("o", "r", 2), Ok(false));
        assert_eq!(forge.merged("o", "r", 3), Err(Lookup::Missing));
        match forge.merged("o", "r", 4) {
            Err(Lookup::Failed(e)) => assert!(e.contains("403") && e.contains("rate limit"), "{e}"),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert!(matches!(forge.merged("..", "r", 5), Err(Lookup::Failed(_))));
        let seen = server.join().expect("server");
        assert_eq!(seen[0], "GET /repos/o/r/pulls/1 HTTP/1.1");
        assert_eq!(seen.len(), 4, "the bad repository never left the process");
    }
}
