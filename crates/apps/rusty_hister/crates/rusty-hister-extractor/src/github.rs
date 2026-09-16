//! `GitHub` — a Rust port of
//! `server/extractor/extractors/github/github.go` (capability inventory
//! §4.5.9). Extract and preview: repository overview pages, issues, issue
//! lists, and pull requests on github.com.
//!
//! Go matches URLs with four independent regexes (repo root, one issue,
//! the issue list, one pull request) tried in order against the raw URL
//! string. Rather than add a `regex` dependency for four fairly mechanical
//! path-shape checks, this port hand-rolls the same checks as small
//! string/character-class predicates operating on the URL with its
//! `https://github.com/` prefix and `<owner>/<repo>` segment stripped —
//! see [`github_kind`] and its helpers. Two Go quirks are reproduced
//! exactly rather than "corrected": the issue-URL pattern only allows a
//! *single* non-slash character after a `#` fragment marker (almost
//! certainly meant to be `[^/]*`, not `[^/]`), while the pull-request
//! pattern allows a full non-slash run — both are presentation-only
//! matching details with no security consequence either way.
//!
//! [`GitHubExtractor::preview`] doesn't re-sanitize its whole accumulated
//! buffer the way `StackExchangeExtractor`/`LobstersExtractor`/
//! `HackerNewsExtractor` do — only the embedded README HTML (the one part
//! genuinely sourced from arbitrary repository content) passes through
//! [`crate::sanitizer::sanitize_html`]; the surrounding metadata card is
//! built entirely from HTML-escaped plain strings, matching Go's own
//! asymmetry (`sanitizer.SanitizeHTML` wraps only `info.readmeHTML`, not
//! `b.String()`).

use crate::sanitizer::sanitize_html;
use crate::stackexchange::{element_text, html_escape, selector};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::Value;
use scraper::Html;
use std::collections::HashSet;

const GITHUB_BASE: &str = "https://github.com";
const GITHUB_URL_PREFIX: &str = "https://github.com/";

/// Top-level GitHub path segments that are never repository owner
/// namespaces (Go: `githubSystemPaths`).
const GITHUB_SYSTEM_PATHS: &[&str] = &[
    "settings",
    "topics",
    "sponsors",
    "features",
    "notifications",
    "explore",
    "marketplace",
    "login",
    "organizations",
    "orgs",
    "copilot",
    "github-copilot",
    "new",
    "issues",
    "pulls",
    "gist",
    "about",
    "contact",
    "pricing",
    "security",
    "enterprise",
    "apps",
];

#[derive(Debug, Default)]
pub struct GitHubExtractor {
    config: ExtractorConfig,
}

impl Extractor for GitHubExtractor {
    fn name(&self) -> &str {
        "GitHub"
    }

    fn description(&self) -> &str {
        "Extracts repository, issue, issue list, and pull request content from GitHub project pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        let parts = url_parts(&document.url);
        if let Some(first) = parts.first() {
            if GITHUB_SYSTEM_PATHS.contains(&first.to_ascii_lowercase().as_str()) {
                return false;
            }
        }
        github_kind(&document.url).is_some()
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        match github_kind(&document.url) {
            Some(GithubKind::Repo) => extract_repo(document),
            Some(GithubKind::Issue) => extract_issue(document),
            Some(GithubKind::Issues) => extract_issues(document),
            Some(GithubKind::Pull) => extract_pull(document),
            None => ExtractOutcome::Fallback(HisterError::Extraction(format!(
                "no extractor matched for {}",
                document.url
            ))),
        }
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);
        let Some(info) = parse_repo_page(&html) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "not a repository overview page".to_string(),
            ));
        };

        let mut out = String::from(r#"<div class="gh-meta">"#);
        if !info.description.is_empty() {
            out.push_str(&format!(
                r#"<p class="gh-description">{}</p>"#,
                html_escape(&info.description)
            ));
        }
        if !info.stars.is_empty() || !info.languages.is_empty() {
            out.push_str(r#"<p class="gh-stats">"#);
            let mut parts = Vec::new();
            if !info.stars.is_empty() {
                parts.push(format!("&#9733; {} stars", html_escape(&info.stars)));
            }
            if !info.languages.is_empty() {
                parts.push(html_escape(&info.languages.join(" / ")));
            }
            out.push_str(&parts.join(" &nbsp;&middot;&nbsp; "));
            out.push_str("</p>");
        }
        if !info.topics.is_empty() {
            out.push_str(r#"<p class="gh-topics">"#);
            for topic in &info.topics {
                out.push_str(&format!("<code>{}</code> ", html_escape(topic)));
            }
            out.push_str("</p>");
        }
        out.push_str("</div>");

        if !info.readme_html.is_empty() {
            out.push_str("<hr>");
            out.push_str(&sanitize_html(&info.readme_html));
        }

        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(out),
            text: None,
            metadata: Default::default(),
        })
    }

    fn config(&self) -> &ExtractorConfig {
        &self.config
    }

    fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
        self.config = config;
        Ok(())
    }
}

enum GithubKind {
    Repo,
    Issue,
    Issues,
    Pull,
}

fn is_owner_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn is_repo_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Parses `<owner>/<repo>` from the start of `rest` (a URL with the
/// `https://github.com/` prefix already stripped). Returns the owner,
/// repo, and whatever text follows the repo segment.
fn parse_owner_repo(rest: &str) -> Option<(&str, &str, &str)> {
    let owner_end = rest.find(|c: char| !is_owner_char(c))?;
    if owner_end == 0 || rest.as_bytes().get(owner_end) != Some(&b'/') {
        return None;
    }
    let after_owner = &rest[owner_end + 1..];
    let repo_end = after_owner
        .find(|c: char| !is_repo_char(c))
        .unwrap_or(after_owner.len());
    if repo_end == 0 {
        return None;
    }
    Some((
        &rest[..owner_end],
        &after_owner[..repo_end],
        &after_owner[repo_end..],
    ))
}

fn get_repo(url: &str) -> Option<String> {
    let rest = url.strip_prefix(GITHUB_URL_PREFIX)?;
    let (owner, repo, _) = parse_owner_repo(rest)?;
    Some(format!("{owner}/{repo}"))
}

/// Matches `(?:#[^/]*|\?[^/]*)?/?$` against what follows `<owner>/<repo>`.
fn is_full_repo_remainder(remainder: &str) -> bool {
    let body = remainder.strip_suffix('/').unwrap_or(remainder);
    if body.is_empty() {
        return true;
    }
    if let Some(fragment) = body.strip_prefix('#') {
        return !fragment.contains('/');
    }
    if let Some(query) = body.strip_prefix('?') {
        return !query.contains('/');
    }
    false
}

/// Matches `/issues/?$`.
fn is_issues_remainder(remainder: &str) -> bool {
    remainder == "/issues" || remainder == "/issues/"
}

/// Matches `/issues/(\d+)(?:#[^/])?/?$` — a *single* non-slash character
/// after `#`, faithfully reproducing Go's own pattern rather than the
/// `[^/]*` it almost certainly meant.
fn is_issue_remainder(remainder: &str) -> bool {
    let Some(rest) = remainder.strip_prefix("/issues/") else {
        return false;
    };
    let digit_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digit_end == 0 {
        return false;
    }
    let tail = &rest[digit_end..];
    let tail = match tail.strip_prefix('#') {
        None => tail,
        Some(after_hash) => {
            let mut chars = after_hash.chars();
            match chars.next() {
                None => "",
                Some(c) if c != '/' => chars.as_str(),
                Some(_) => return false,
            }
        }
    };
    tail.is_empty() || tail == "/"
}

/// Matches `/pull/(\d+)(?:#[^/]+)?/?$`.
fn is_pull_remainder(remainder: &str) -> bool {
    let Some(rest) = remainder.strip_prefix("/pull/") else {
        return false;
    };
    let digit_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digit_end == 0 {
        return false;
    }
    let tail = &rest[digit_end..];
    let body = tail.strip_suffix('/').unwrap_or(tail);
    if body.is_empty() {
        return true;
    }
    match body.strip_prefix('#') {
        Some(fragment) => !fragment.is_empty() && !fragment.contains('/'),
        None => false,
    }
}

fn github_kind(url: &str) -> Option<GithubKind> {
    let rest = url.strip_prefix(GITHUB_URL_PREFIX)?;
    let (_, _, remainder) = parse_owner_repo(rest)?;
    if is_full_repo_remainder(remainder) {
        Some(GithubKind::Repo)
    } else if is_issue_remainder(remainder) {
        Some(GithubKind::Issue)
    } else if is_issues_remainder(remainder) {
        Some(GithubKind::Issues)
    } else if is_pull_remainder(remainder) {
        Some(GithubKind::Pull)
    } else {
        None
    }
}

fn url_parts(url: &str) -> Vec<&str> {
    let mut path = url.strip_prefix(GITHUB_URL_PREFIX).unwrap_or(url);
    if let Some(i) = path.find(['?', '#']) {
        path = &path[..i];
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    path.split('/').collect()
}

fn page_title(html: &Html) -> String {
    html.select(&selector("title"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default()
}

fn extract_repo(document: &Document) -> ExtractOutcome {
    let Some(html_str) = document.html.as_deref() else {
        return ExtractOutcome::Fallback(HisterError::Extraction(
            "document has no HTML".to_string(),
        ));
    };
    let html = Html::parse_document(html_str);
    let Some(info) = parse_repo_page(&html) else {
        return ExtractOutcome::Fallback(HisterError::Extraction(
            "not a repository overview page".to_string(),
        ));
    };

    let mut extracted = document.clone();
    extracted.title = Some(page_title(&html));
    extracted
        .metadata
        .insert("type".to_string(), Value::String("Repository".to_string()));
    if let Some(repo) = get_repo(&document.url) {
        extracted
            .metadata
            .insert("repo".to_string(), Value::String(repo));
    }

    let mut text = String::new();
    if !info.description.is_empty() {
        text.push_str("description: ");
        text.push_str(&info.description);
        text.push_str("\n\n");
        extracted.metadata.insert(
            "description".to_string(),
            Value::String(info.description.clone()),
        );
    }
    if !info.topics.is_empty() {
        text.push_str("topics: ");
        text.push_str(&info.topics.join(", "));
        text.push('\n');
        extracted
            .metadata
            .insert("topics".to_string(), Value::String(info.topics.join(", ")));
    }
    if !info.languages.is_empty() {
        text.push_str("languages: ");
        text.push_str(&info.languages.join(", "));
        text.push('\n');
        extracted.metadata.insert(
            "languages".to_string(),
            Value::String(info.languages.join(", ")),
        );
    }
    if !info.stars.is_empty() {
        text.push_str("stars: ");
        text.push_str(&info.stars);
        text.push('\n');
    }
    if !info.readme_html.is_empty() {
        let readme_doc = Html::parse_document(&info.readme_html);
        let readme_text = readme_doc.root_element().text().collect::<String>();
        text.push('\n');
        text.push_str(readme_text.trim());
    }

    let text = text.trim().to_string();
    if text.is_empty() && extracted.title.as_deref().unwrap_or("").is_empty() {
        return ExtractOutcome::Fallback(HisterError::Extraction("no content found".to_string()));
    }
    extracted.text = Some(text);
    ExtractOutcome::Extracted(extracted)
}

/// The extracted fields from a GitHub repository overview page.
struct RepoInfo {
    description: String,
    stars: String,
    topics: Vec<String>,
    languages: Vec<String>,
    readme_html: String,
}

/// Extracts repository metadata. Returns `None` if the page doesn't look
/// like a repository overview page (no `p.f4` "about" description).
fn parse_repo_page(html: &Html) -> Option<RepoInfo> {
    let description = html
        .select(&selector("p.f4"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    if description.is_empty() {
        return None;
    }

    let mut stars = String::new();
    for el in html.select(&selector("[aria-label]")) {
        if let Some(label) = el.value().attr("aria-label") {
            if let Some(count) = parse_stars_label(label.trim()) {
                stars = count;
            }
        }
    }

    let mut topics = Vec::new();
    let mut seen_topics = HashSet::new();
    for el in html.select(&selector(r#"a[href^="/topics/"].topic-tag-link"#)) {
        if let Some(topic) = el
            .value()
            .attr("href")
            .and_then(|href| href.strip_prefix("/topics/"))
        {
            if !topic.is_empty() && seen_topics.insert(topic.to_string()) {
                topics.push(topic.to_string());
            }
        }
    }

    let mut languages = Vec::new();
    let mut seen_langs = HashSet::new();
    for el in html.select(&selector("span.color-fg-default.text-bold.mr-1")) {
        let lang = element_text(&el).trim().to_string();
        if !lang.is_empty() && lang != "Other" && seen_langs.insert(lang.clone()) {
            languages.push(lang);
        }
    }

    let readme_html = extract_readme_html(html)
        .map(|rt| resolve_relative_urls(&rt))
        .unwrap_or_default();

    Some(RepoInfo {
        description,
        stars,
        topics,
        languages,
        readme_html,
    })
}

/// Mirrors `^([\d,]+)\s+users?\s+starred\s+this\s+repository$`.
fn parse_stars_label(label: &str) -> Option<String> {
    let words: Vec<&str> = label.split_whitespace().collect();
    let [count, user_word, "starred", "this", "repository"] = words[..] else {
        return None;
    };
    if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit() || c == ',') {
        return None;
    }
    if user_word != "user" && user_word != "users" {
        return None;
    }
    Some(count.to_string())
}

/// Searches all `<script type="application/json">` blocks for the first
/// `overviewFiles` entry with non-empty `richText` (the rendered README
/// HTML).
fn extract_readme_html(html: &Html) -> Option<String> {
    for el in html.select(&selector(r#"script[type="application/json"]"#)) {
        let raw = element_text(&el);
        if !raw.contains("overviewFiles") {
            continue;
        }
        let Ok(payload) = Value::parse(&raw) else {
            continue;
        };
        if let Some(rich_text) = find_rich_text(&payload) {
            return Some(rich_text);
        }
    }
    None
}

/// Recursively walks a JSON value, returning the first non-empty
/// `richText` string found inside an `overviewFiles` array.
fn find_rich_text(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(files) = map.get("overviewFiles") {
                if let Some(rich_text) = rich_text_from_files(files) {
                    return Some(rich_text);
                }
            }
            map.iter().find_map(|(_, child)| find_rich_text(child))
        }
        Value::Array(items) => items.iter().find_map(find_rich_text),
        _ => None,
    }
}

fn rich_text_from_files(value: &Value) -> Option<String> {
    let Value::Array(files) = value else {
        return None;
    };
    files.iter().find_map(|file| {
        let Value::Object(entry) = file else {
            return None;
        };
        match entry.get("richText") {
            Some(Value::String(rich_text)) if !rich_text.is_empty() => Some(rich_text.clone()),
            _ => None,
        }
    })
}

/// Rewrites root-relative `src="/..."`/`href="/..."` attributes in README
/// HTML to absolute `github.com` URLs. Protocol-relative (`//...`) and
/// empty (`""`) values are left untouched. Operates on the raw string
/// (matching Go's own regex-substitution approach) rather than reparsing
/// and re-serializing the README as a DOM, since the result is stored as
/// opaque HTML text, not walked further here.
fn resolve_relative_urls(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    loop {
        let next_src = lower[pos..].find("src=\"").map(|i| (pos + i, 5));
        let next_href = lower[pos..].find("href=\"").map(|i| (pos + i, 6));
        let candidate = match (next_src, next_href) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some((attr_start, attr_len)) = candidate else {
            out.push_str(&html[pos..]);
            break;
        };
        let quote_pos = attr_start + attr_len;
        out.push_str(&html[pos..quote_pos]);
        let after_quote = &html[quote_pos..];
        let mut chars = after_quote.chars();
        if chars.next() == Some('/') && !matches!(chars.next(), Some('/') | Some('"')) {
            out.push_str(GITHUB_BASE);
        }
        pos = quote_pos;
    }
    out
}

fn extract_issue(document: &Document) -> ExtractOutcome {
    let Some(html_str) = document.html.as_deref() else {
        return ExtractOutcome::Fallback(HisterError::Extraction(
            "document has no HTML".to_string(),
        ));
    };
    let html = Html::parse_document(html_str);

    let mut extracted = document.clone();
    extracted.title = Some(page_title(&html));
    extracted
        .metadata
        .insert("type".to_string(), Value::String("Issue".to_string()));
    if let Some(repo) = get_repo(&document.url) {
        extracted
            .metadata
            .insert("repo".to_string(), Value::String(repo));
    }

    let mut text = String::new();
    let title = html
        .select(&selector(r#"bdi[data-testid="issue-title"]"#))
        .next()
        .map(|el| element_text(&el))
        .unwrap_or_default();
    if !title.is_empty() {
        extracted
            .metadata
            .insert("title".to_string(), Value::String(title.clone()));
        text.push_str(&format!("title: {title}\n\n"));
    }
    if let Some(date) = html
        .select(&selector(r#"[data-testid="issue-body"] relative-time"#))
        .next()
        .and_then(|el| el.value().attr("datetime"))
    {
        if !date.is_empty() {
            extracted
                .metadata
                .insert("date".to_string(), Value::String(date.to_string()));
        }
    }
    let body = html
        .select(&selector("#issue-body-viewer"))
        .next()
        .map(|el| element_text(&el))
        .unwrap_or_default();
    if !body.is_empty() {
        text.push_str(&format!("body: {body}\n\n"));
    }
    let comment_bodies: Vec<String> = html
        .select(&selector(
            r#"[data-testid="issue-viewer-comments-container"] [data-testid="markdown-body"]"#,
        ))
        .map(|el| element_text(&el).trim().to_string())
        .collect();
    if !comment_bodies.is_empty() {
        text.push_str(&format!("comments: {}\n", comment_bodies.join(", ")));
    }

    let text = text.trim().to_string();
    if text.is_empty() && extracted.title.as_deref().unwrap_or("").is_empty() {
        return ExtractOutcome::Fallback(HisterError::Extraction("no content found".to_string()));
    }
    extracted.text = Some(text);
    ExtractOutcome::Extracted(extracted)
}

fn extract_issues(document: &Document) -> ExtractOutcome {
    let Some(html_str) = document.html.as_deref() else {
        return ExtractOutcome::Fallback(HisterError::Extraction(
            "document has no HTML".to_string(),
        ));
    };
    let html = Html::parse_document(html_str);

    let mut extracted = document.clone();
    extracted.title = Some(page_title(&html));
    extracted
        .metadata
        .insert("type".to_string(), Value::String("Issues".to_string()));
    if let Some(repo) = get_repo(&document.url) {
        extracted
            .metadata
            .insert("repo".to_string(), Value::String(repo));
    }

    let mut text = String::new();
    let pinned: Vec<String> = html
        .select(&selector(
            r#"ul[aria-label="Drag and drop pinned issues list."] li"#,
        ))
        .map(|el| element_text(&el).trim().to_string())
        .collect();
    if !pinned.is_empty() {
        text.push_str(&format!("pinned issues: {}\n", pinned.join(", ")));
    }
    let issues: Vec<String> = html
        .select(&selector(r#"ul[data-listview-component="items-list"] li"#))
        .map(|el| element_text(&el).trim().to_string())
        .collect();
    if !issues.is_empty() {
        text.push_str(&format!("regular issues: {}\n", issues.join(", ")));
    }

    let text = text.trim().to_string();
    if text.is_empty() && extracted.title.as_deref().unwrap_or("").is_empty() {
        return ExtractOutcome::Fallback(HisterError::Extraction("no content found".to_string()));
    }
    extracted.text = Some(text);
    ExtractOutcome::Extracted(extracted)
}

fn extract_pull(document: &Document) -> ExtractOutcome {
    let Some(html_str) = document.html.as_deref() else {
        return ExtractOutcome::Fallback(HisterError::Extraction(
            "document has no HTML".to_string(),
        ));
    };
    let html = Html::parse_document(html_str);

    let mut extracted = document.clone();
    extracted.title = Some(page_title(&html));
    extracted
        .metadata
        .insert("type".to_string(), Value::String("PullRequest".to_string()));
    if let Some(repo) = get_repo(&document.url) {
        extracted
            .metadata
            .insert("repo".to_string(), Value::String(repo));
    }

    let mut text = String::new();
    let title = html
        .select(&selector(
            r#"h1[data-component="PH_Title"] .markdown-title"#,
        ))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    if !title.is_empty() {
        extracted
            .metadata
            .insert("title".to_string(), Value::String(title.clone()));
        text.push_str(&format!("title: {title}\n\n"));
    }
    if let Some(date) = html
        .select(&selector(".js-command-palette-pull-body relative-time"))
        .next()
        .and_then(|el| el.value().attr("datetime"))
    {
        if !date.is_empty() {
            extracted
                .metadata
                .insert("date".to_string(), Value::String(date.to_string()));
        }
    }
    let comments: Vec<String> = html
        .select(&selector(".js-comment-container"))
        .map(|el| element_text(&el).trim().to_string())
        .collect();
    if !comments.is_empty() {
        text.push_str(&format!("comments: {}\n", comments.join(", ")));
    }
    let state = html
        .select(&selector("[data-status]"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    if !state.is_empty() {
        extracted
            .metadata
            .insert("state".to_string(), Value::String(state));
    }

    let text = text.trim().to_string();
    if text.is_empty() && extracted.title.as_deref().unwrap_or("").is_empty() {
        return ExtractOutcome::Fallback(HisterError::Extraction("no content found".to_string()));
    }
    extracted.text = Some(text);
    ExtractOutcome::Extracted(extracted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    #[test]
    fn matches_only_known_github_url_shapes() {
        let ext = GitHubExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("https://github.com/asciimoo", false),
            ("https://github.com/asciimoo/hister", true),
            ("https://github.com/asciimoo/hister/", true),
            (
                "https://github.com/asciimoo/hister?tab=readme-ov-file",
                true,
            ),
            ("https://github.com/asciimoo/hister#community", true),
            ("https://github.com/asciimoo/hister/issues", true),
            ("https://github.com/asciimoo/hister/issues/305", true),
            ("https://github.com/asciimoo/hister/issues/305/", true),
            ("https://github.com/asciimoo/hister/pulls", false),
            ("https://github.com/asciimoo/hister/pull/495", true),
            (
                "https://github.com/asciimoo/hister/pull/495#issuecomment-123",
                true,
            ),
            ("https://github.com/asciimoo/hister/settings", false),
            ("https://github.com/topics/react-native", false),
            ("https://stackoverflow.com/questions/1234", false),
            ("https://example.com/wiki/Foo", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&doc(url, "")), *want, "matches({url:?})");
        }
    }

    const REPO_PAGE: &str = r##"
        <html><head><title>acme/widget: A fine widget</title></head><body>
        <p class="f4">A fine widget library</p>
        <span aria-label="42 users starred this repository">42</span>
        <a href="/topics/rust" class="topic-tag-link">rust</a>
        <a href="/topics/widgets" class="topic-tag-link">widgets</a>
        <span class="color-fg-default text-bold mr-1">Rust</span>
        <span class="color-fg-default text-bold mr-1">Other</span>
        <script type="application/json">{"payload":{"overview":{"overviewFiles":[{"richText":"<article><h1>Widget</h1><p>Install it.</p><img src=\"/img/logo.png\"><a href=\"//cdn.example/x\">cdn</a></article>"}]}}}</script>
        </body></html>
    "##;

    #[test]
    fn extract_repo_collects_metadata_and_readme_text() {
        let ext = GitHubExtractor::default();
        match ext.extract(&doc("https://github.com/acme/widget", REPO_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("acme/widget: A fine widget"));
                assert_eq!(
                    result.metadata.get("type"),
                    Some(&Value::String("Repository".to_string()))
                );
                assert_eq!(
                    result.metadata.get("repo"),
                    Some(&Value::String("acme/widget".to_string()))
                );
                assert_eq!(
                    result.metadata.get("topics"),
                    Some(&Value::String("rust, widgets".to_string()))
                );
                assert_eq!(
                    result.metadata.get("languages"),
                    Some(&Value::String("Rust".to_string()))
                );
                let text = result.text.unwrap();
                assert!(text.contains("description: A fine widget library"));
                assert!(text.contains("stars: 42"));
                assert!(text.contains("Widget"));
                assert!(text.contains("Install it."));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_repo_falls_back_when_not_a_repo_overview_page() {
        let ext = GitHubExtractor::default();
        let result = ext.extract(&doc(
            "https://github.com/acme/widget",
            "<html><body>nope</body></html>",
        ));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_resolves_root_relative_readme_urls_but_not_protocol_relative() {
        let ext = GitHubExtractor::default();
        match ext.preview(&doc("https://github.com/acme/widget", REPO_PAGE)) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                assert!(html.contains(r#"src="https://github.com/img/logo.png""#));
                assert!(html.contains(r#"href="//cdn.example/x""#));
                assert!(html.contains("gh-description"));
                assert!(html.contains("42 stars"));
                assert!(html.contains("<code>rust</code>"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    const ISSUE_PAGE: &str = r##"
        <html><head><title>Bug: crashes on start · Issue #7 · acme/widget</title></head><body>
        <bdi data-testid="issue-title">crashes on start</bdi>
        <div data-testid="issue-body"><relative-time datetime="2024-05-01T00:00:00Z"></relative-time></div>
        <div id="issue-body-viewer">It crashes immediately.</div>
        <div data-testid="issue-viewer-comments-container">
            <div data-testid="markdown-body">Can reproduce.</div>
            <div data-testid="markdown-body">Same here.</div>
        </div>
        </body></html>
    "##;

    #[test]
    fn extract_issue_collects_title_body_date_and_comments() {
        let ext = GitHubExtractor::default();
        match ext.extract(&doc("https://github.com/acme/widget/issues/7", ISSUE_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(
                    result.metadata.get("type"),
                    Some(&Value::String("Issue".to_string()))
                );
                assert_eq!(
                    result.metadata.get("date"),
                    Some(&Value::String("2024-05-01T00:00:00Z".to_string()))
                );
                let text = result.text.unwrap();
                assert!(text.contains("title: crashes on start"));
                assert!(text.contains("body: It crashes immediately."));
                assert!(text.contains("comments: Can reproduce., Same here."));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    const ISSUES_PAGE: &str = r##"
        <html><head><title>Issues · acme/widget</title></head><body>
        <ul aria-label="Drag and drop pinned issues list."><li>Pinned one</li></ul>
        <ul data-listview-component="items-list"><li>Regular one</li><li>Regular two</li></ul>
        </body></html>
    "##;

    #[test]
    fn extract_issues_collects_pinned_and_regular_issues() {
        let ext = GitHubExtractor::default();
        match ext.extract(&doc("https://github.com/acme/widget/issues", ISSUES_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                let text = result.text.unwrap();
                assert!(text.contains("pinned issues: Pinned one"));
                assert!(text.contains("regular issues: Regular one, Regular two"));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    const PULL_PAGE: &str = r##"
        <html><head><title>Add feature X · Pull Request #9 · acme/widget</title></head><body>
        <h1 data-component="PH_Title"><span class="markdown-title">Add feature X</span></h1>
        <div class="js-command-palette-pull-body"><relative-time datetime="2024-06-01T00:00:00Z"></relative-time></div>
        <div class="js-comment-container">Looks good.</div>
        <span data-status>Open</span>
        </body></html>
    "##;

    #[test]
    fn extract_pull_collects_title_date_comments_and_state() {
        let ext = GitHubExtractor::default();
        match ext.extract(&doc("https://github.com/acme/widget/pull/9", PULL_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(
                    result.metadata.get("type"),
                    Some(&Value::String("PullRequest".to_string()))
                );
                assert_eq!(
                    result.metadata.get("state"),
                    Some(&Value::String("Open".to_string()))
                );
                assert_eq!(
                    result.metadata.get("date"),
                    Some(&Value::String("2024-06-01T00:00:00Z".to_string()))
                );
                let text = result.text.unwrap();
                assert!(text.contains("title: Add feature X"));
                assert!(text.contains("comments: Looks good."));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_falls_back_for_an_unmatched_url_shape() {
        let ext = GitHubExtractor::default();
        let result = ext.extract(&doc("https://github.com/acme/widget/wiki", "<html></html>"));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }
}
