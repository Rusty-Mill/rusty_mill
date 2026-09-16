//! `Ytdlp` — a Rust port of
//! `server/extractor/extractors/ytdlp/{ytdlp,types,format,vtt}.go`
//! (capability inventory §4.5.17). Extract and preview for video-hosting
//! pages (YouTube, Vimeo, and others), by shelling out to the external
//! `yt-dlp` binary rather than parsing `document.html` at all — the only
//! extractor in this crate that works entirely from `document.url`.
//! **Disabled by default** ([`YtdlpExtractor::default`]), matching Go:
//! this extractor is useless without `yt-dlp` installed, so opting a chain
//! into it is a deliberate administrative choice, not automatic.
//!
//! Three deliberate simplifications from the Go original, each because
//! this crate's `Extractor` trait (and the rest of this cluster) has no
//! equivalent to lean on, rather than an oversight:
//! - **No thumbnail download.** Go fetches the thumbnail image over HTTP
//!   and stores it as a base64 data URI in metadata. This crate has no
//!   general-purpose HTTP *client* to reuse for a one-off image fetch —
//!   `rusty_http` is a sans-IO protocol layer (head parsing/framing, no
//!   client), and standing up a full request/response/TLS stack just for
//!   this would be a large, separate piece of work. The `thumbnail_url`
//!   metadata key holds the original URL instead — deliberately renamed
//!   from Go's `thumbnail` key rather than reusing it for a different
//!   kind of value (a URL rather than embedded image data).
//! - **No per-instance job-slot concurrency limit or cancellation.** Go's
//!   `ExtractContext`/`PreviewContext` take a `context.Context` and cap
//!   concurrent `yt-dlp` subprocesses with a semaphore; this crate's
//!   `Extractor` trait has no cancellation-token concept and no other
//!   extractor models concurrency limits either, so this one doesn't add
//!   the only instance of either.
//! - **Preview renders HTML directly** rather than Go's structured JSON
//!   handed to a `"video"` frontend template — `PreviewResponse` has no
//!   template-hint field, only `html`/`text`/`metadata`, matching every
//!   other extractor in this crate.
//!
//! Everything else — the domain allow-list in [`matches`](Extractor::matches),
//! the `--dump-json` invocation and its argument list, the 10-minute
//! response cache, and (when `fetch_subtitles` is enabled) a second
//! `yt-dlp` invocation per language candidate to fetch and
//! [`vtt::parse_vtt`] a transcript — is a direct port.

mod format;
mod types;
mod vtt;

use format::{format_date, format_duration};
use types::{Chapter, VideoInfo, KNOWN_DOMAINS, KNOWN_HOST_SUBSTRINGS};
use vtt::parse_vtt;

use crate::sanitizer::sanitize_html;
use crate::stackexchange::html_escape;
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::{Map, Number, Value};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(600);
const MAX_CACHE_SIZE: usize = 500;

const KNOWN_OPTIONS: &[&str] = &[
    "binary",
    "timeout",
    "max_concurrent_jobs",
    "fetch_subtitles",
    "sub_language",
    "cookies_file",
    "cookies_from_browser",
    "extra_args",
    "extra_domains",
];

#[derive(Debug, Clone)]
struct CachedInfo {
    info: VideoInfo,
    fetched_at: Instant,
}

#[derive(Debug)]
pub struct YtdlpExtractor {
    config: ExtractorConfig,
    cache: Mutex<HashMap<String, CachedInfo>>,
}

impl Default for YtdlpExtractor {
    fn default() -> Self {
        let mut options = Map::new();
        options.insert("binary".to_string(), Value::String("yt-dlp".to_string()));
        options.insert("timeout".to_string(), Value::Number(Number::from(15)));
        options.insert(
            "max_concurrent_jobs".to_string(),
            Value::Number(Number::from(2)),
        );
        options.insert("fetch_subtitles".to_string(), Value::Bool(false));
        options.insert(
            "sub_language".to_string(),
            Value::String("auto".to_string()),
        );
        options.insert("extra_domains".to_string(), Value::Array(Vec::new()));
        YtdlpExtractor {
            config: ExtractorConfig {
                enabled: false,
                options,
            },
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl Extractor for YtdlpExtractor {
    fn name(&self) -> &str {
        "Ytdlp"
    }

    fn description(&self) -> &str {
        "Extracts video metadata (title, description, chapters, subtitles, thumbnail) from video hosting sites using the yt-dlp tool."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        let extra_domains = self.extra_domains();
        let matched = KNOWN_DOMAINS
            .iter()
            .map(|s| (*s).to_string())
            .chain(extra_domains)
            .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
            || KNOWN_HOST_SUBSTRINGS.iter().any(|sub| host.contains(sub));
        if !matched {
            return false;
        }
        // Reject bare homepages -- yt-dlp cannot extract anything from a
        // site root with no path or query parameters.
        let path = url.path().trim_end_matches('/');
        !(path.is_empty() && url.query().unwrap_or("").is_empty())
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let info = match self.get_info(&document.url) {
            Ok(info) => info,
            Err(message) => return ExtractOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut extracted = document.clone();
        if !info.title.is_empty() {
            extracted.title = Some(info.title.clone());
        }

        let mut text = String::new();
        if !info.description.is_empty() {
            text.push_str(&info.description);
        }
        if !info.uploader.is_empty() {
            text.push_str("\n\nUploader: ");
            text.push_str(&info.uploader);
        }
        if !info.tags.is_empty() {
            text.push_str("\nTags: ");
            text.push_str(&info.tags.join(", "));
        }
        if !info.categories.is_empty() {
            text.push_str("\nCategories: ");
            text.push_str(&info.categories.join(", "));
        }
        if !info.chapters.is_empty() {
            text.push_str("\n\nChapters:\n");
            push_chapter_lines(&mut text, &info.chapters);
        }
        if !info.playlist_title.is_empty() {
            text.push_str("\nPlaylist: ");
            text.push_str(&info.playlist_title);
            if let (Some(index), Some(count)) = (info.playlist_index, info.playlist_count) {
                text.push_str(&format!(" ({index}/{count})"));
            }
        }
        if self.fetch_subtitles_enabled() {
            let transcript = self.fetch_subtitle_text(&info);
            if !transcript.is_empty() {
                text.push_str("\n\nTranscript:\n");
                text.push_str(&transcript);
            }
        }
        extracted.text = Some(text.trim().to_string());

        if !info.thumbnail.is_empty() {
            extracted.metadata.insert(
                "thumbnail_url".to_string(),
                Value::String(info.thumbnail.clone()),
            );
        }
        if info.duration > 0.0 {
            let duration = Number::from_f64(info.duration).unwrap_or(Number::from(0));
            extracted
                .metadata
                .insert("duration".to_string(), Value::Number(duration));
        }
        if !info.upload_date.is_empty() {
            extracted.metadata.insert(
                "upload_date".to_string(),
                Value::String(format_date(&info.upload_date)),
            );
        }
        if !info.uploader.is_empty() {
            extracted
                .metadata
                .insert("uploader".to_string(), Value::String(info.uploader.clone()));
        }
        if info.view_count > 0 {
            extracted.metadata.insert(
                "view_count".to_string(),
                Value::Number(Number::from(info.view_count)),
            );
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let info = match self.get_info(&document.url) {
            Ok(info) => info,
            Err(message) => return PreviewOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut out = String::new();
        if !info.thumbnail.is_empty() {
            out.push_str(&format!(
                r#"<img src="{}" alt="" style="max-width: 100%; height: auto">"#,
                html_escape(&info.thumbnail)
            ));
        }
        if !info.title.is_empty() {
            out.push_str(&format!("<h2>{}</h2>", html_escape(&info.title)));
        }
        let mut parts = Vec::new();
        if !info.uploader.is_empty() {
            parts.push(format!("uploaded by {}", html_escape(&info.uploader)));
        }
        if info.duration > 0.0 {
            parts.push(html_escape(&format_duration(info.duration)));
        }
        if !info.upload_date.is_empty() {
            parts.push(html_escape(&format_date(&info.upload_date)));
        }
        if info.view_count > 0 {
            parts.push(format!("{} views", info.view_count));
        }
        if !parts.is_empty() {
            out.push_str(&format!("<p>{}</p>", parts.join(" &middot; ")));
        }
        if !info.categories.is_empty() {
            out.push_str(&format!(
                "<p>categories: {}</p>",
                html_escape(&info.categories.join(", "))
            ));
        }
        if !info.tags.is_empty() {
            out.push_str(&format!(
                "<p>tags: {}</p>",
                html_escape(&info.tags.join(", "))
            ));
        }
        if !info.description.is_empty() {
            out.push_str(&format!(
                "<p>{}</p>",
                html_escape(&info.description).replace('\n', "<br>")
            ));
        }
        if !info.chapters.is_empty() {
            out.push_str("<h3>Chapters</h3><ul>");
            for chapter in &info.chapters {
                out.push_str(&format!(
                    "<li>{} {}</li>",
                    html_escape(&format_duration(chapter.start_time)),
                    html_escape(&chapter.title)
                ));
            }
            out.push_str("</ul>");
        }
        if !info.playlist_title.is_empty() {
            out.push_str(&format!(
                "<p>Playlist: {}",
                html_escape(&info.playlist_title)
            ));
            if let (Some(index), Some(count)) = (info.playlist_index, info.playlist_count) {
                out.push_str(&format!(" ({index}/{count})"));
            }
            out.push_str("</p>");
        }
        if self.fetch_subtitles_enabled() {
            let transcript = self.fetch_subtitle_text(&info);
            if !transcript.is_empty() {
                out.push_str(&format!(
                    "<h3>Transcript</h3><p>{}</p>",
                    html_escape(&transcript)
                ));
            }
        }

        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitize_html(&out)),
            text: None,
            metadata: Default::default(),
        })
    }

    fn config(&self) -> &ExtractorConfig {
        &self.config
    }

    fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
        for key in config.options.iter().map(|(k, _)| k.as_str()) {
            if !KNOWN_OPTIONS.contains(&key) {
                return Err(HisterError::InvalidConfig(format!(
                    "unknown option {key:?}"
                )));
            }
        }
        self.config = config;
        Ok(())
    }
}

fn push_chapter_lines(text: &mut String, chapters: &[Chapter]) {
    for chapter in chapters {
        text.push_str(&format!(
            "  {} {}\n",
            format_duration(chapter.start_time),
            chapter.title
        ));
    }
}

impl YtdlpExtractor {
    fn binary(&self) -> String {
        self.config
            .options
            .get("binary")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("yt-dlp")
            .to_string()
    }

    fn timeout(&self) -> Duration {
        match self.config.options.get("timeout") {
            Some(Value::Number(n)) => {
                let secs = n
                    .as_i64()
                    .or_else(|| n.as_u64().map(|u| u as i64))
                    .unwrap_or(15);
                Duration::from_secs(secs.max(0) as u64)
            }
            _ => Duration::from_secs(15),
        }
    }

    fn fetch_subtitles_enabled(&self) -> bool {
        matches!(
            self.config.options.get("fetch_subtitles"),
            Some(Value::Bool(true))
        )
    }

    /// Go's own internal fallback (used when the option is missing or not
    /// a string) is `"en"`, distinct from the `"auto"` default
    /// [`YtdlpExtractor::default`] populates — a real, if slightly odd,
    /// discrepancy in the Go source (the fallback only ever matters if a
    /// caller explicitly sets a non-string value), preserved here rather
    /// than "corrected".
    fn sub_language(&self) -> String {
        self.config
            .options
            .get("sub_language")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("en")
            .to_string()
    }

    fn cookie_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(f) = self
            .config
            .options
            .get("cookies_file")
            .and_then(Value::as_str)
        {
            if !f.is_empty() {
                args.push("--cookies".to_string());
                args.push(f.to_string());
            }
        }
        if let Some(b) = self
            .config
            .options
            .get("cookies_from_browser")
            .and_then(Value::as_str)
        {
            if !b.is_empty() {
                args.push("--cookies-from-browser".to_string());
                args.push(b.to_string());
            }
        }
        args
    }

    fn extra_args(&self) -> Vec<String> {
        match self.config.options.get("extra_args") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn extra_domains(&self) -> Vec<String> {
        match self.config.options.get("extra_domains") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn get_info(&self, video_url: &str) -> Result<VideoInfo, String> {
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cached) = cache.get(video_url) {
                if cached.fetched_at.elapsed() < CACHE_TTL {
                    return Ok(cached.info.clone());
                }
                cache.remove(video_url);
            }
        }
        let info = self.fetch_info(video_url)?;
        self.prune_cache();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            video_url.to_string(),
            CachedInfo {
                info: info.clone(),
                fetched_at: Instant::now(),
            },
        );
        Ok(info)
    }

    fn prune_cache(&self) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        cache.retain(|_, v| now.duration_since(v.fetched_at) < CACHE_TTL);
        if cache.len() >= MAX_CACHE_SIZE {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.fetched_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }
    }

    fn fetch_info(&self, video_url: &str) -> Result<VideoInfo, String> {
        let mut args = vec![
            "--dump-json".to_string(),
            "--no-download".to_string(),
            "--no-playlist".to_string(),
            "--no-warnings".to_string(),
        ];
        args.extend(self.cookie_args());
        args.extend(self.extra_args());
        args.push(video_url.to_string());

        let output = self.run_ytdlp(&args)?;
        rusty_json::from_str::<VideoInfo>(&output)
            .map_err(|e| format!("failed to parse yt-dlp output: {e}"))
    }

    fn fetch_subtitle_text(&self, info: &VideoInfo) -> String {
        let lang_spec = self.sub_language();
        if lang_spec == "auto" {
            if info.language.is_empty() {
                return String::new();
            }
            return self.download_subtitle_for_lang(info, &info.language);
        }
        for lang in lang_spec.split(',').map(str::trim) {
            if lang.is_empty() {
                continue;
            }
            let text = self.download_subtitle_for_lang(info, lang);
            if !text.is_empty() {
                return text;
            }
        }
        String::new()
    }

    fn download_subtitle_for_lang(&self, info: &VideoInfo, lang: &str) -> String {
        let has_manual = info.subtitles.get(lang).is_some_and(|v| !v.is_empty());
        let has_auto = info
            .automatic_captions
            .get(lang)
            .is_some_and(|v| !v.is_empty());
        if !has_manual && !has_auto {
            return String::new();
        }
        let Ok(dir) = make_temp_dir() else {
            return String::new();
        };
        let text = (|| {
            let out_template = dir.join("sub");
            let mut args = vec![
                "--skip-download".to_string(),
                "--no-playlist".to_string(),
                "--no-warnings".to_string(),
                "--sub-lang".to_string(),
                lang.to_string(),
                "--convert-subs".to_string(),
                "vtt".to_string(),
                "-o".to_string(),
                out_template.display().to_string(),
            ];
            args.push(
                if has_manual {
                    "--write-sub"
                } else {
                    "--write-auto-sub"
                }
                .to_string(),
            );
            args.extend(self.cookie_args());
            args.push(info.webpage_url.clone());

            self.run_ytdlp(&args).ok()?;

            let entry = std::fs::read_dir(&dir)
                .ok()?
                .filter_map(Result::ok)
                .find(|entry| entry.path().extension().is_some_and(|ext| ext == "vtt"))?;
            let data = std::fs::read_to_string(entry.path()).ok()?;
            Some(parse_vtt(&data))
        })();
        let _ = std::fs::remove_dir_all(&dir);
        text.unwrap_or_default()
    }

    /// Runs `yt-dlp` with `args`, enforcing [`Self::timeout`] by polling
    /// rather than blocking indefinitely (`std::process::Child` has no
    /// built-in wait-with-timeout). `stdout`/`stderr` are drained on their
    /// own threads throughout, not just after the process exits, so a
    /// chatty child can't deadlock by filling a pipe buffer no one is
    /// reading yet.
    fn run_ytdlp(&self, args: &[String]) -> Result<String, String> {
        let mut command = Command::new(self.binary());
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("yt-dlp failed to start: {e}"))?;

        let Some(mut stdout_pipe) = child.stdout.take() else {
            return Err("yt-dlp: missing stdout pipe".to_string());
        };
        let Some(mut stderr_pipe) = child.stderr.take() else {
            return Err("yt-dlp: missing stderr pipe".to_string());
        };
        let stdout_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut buf);
            buf
        });

        let deadline = Instant::now() + self.timeout();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break None,
            }
        };

        let stdout = stdout_thread.join().unwrap_or_default();
        let stderr = stderr_thread.join().unwrap_or_default();

        let Some(status) = status else {
            return Err("yt-dlp timed out".to_string());
        };
        if !status.success() {
            let stderr_text = String::from_utf8_lossy(&stderr).trim().to_string();
            return if stderr_text.is_empty() {
                Err(format!("yt-dlp failed: {status}"))
            } else {
                Err(format!("yt-dlp failed: {stderr_text}"))
            };
        }
        Ok(String::from_utf8_lossy(&stdout).to_string())
    }
}

fn make_temp_dir() -> std::io::Result<std::path::PathBuf> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("hister-subs-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str) -> Document {
        Document::new(url)
    }

    #[test]
    fn is_disabled_by_default() {
        let ext = YtdlpExtractor::default();
        assert!(!ext.config().enabled);
    }

    #[test]
    fn matches_known_video_hosts_but_rejects_bare_homepages() {
        let ext = YtdlpExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("https://www.youtube.com/watch?v=abc123", true),
            ("https://youtu.be/abc123", true),
            ("https://vimeo.com/12345", true),
            ("https://my-instance.peertube.live/w/abc", true),
            ("https://www.youtube.com/", false),
            ("https://www.youtube.com", false),
            ("https://example.com/watch?v=abc123", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&doc(url)), *want, "url = {url}");
        }
    }

    #[test]
    fn matches_extra_configured_domains() {
        let mut ext = YtdlpExtractor::default();
        let mut options = ext.config().options.clone();
        options.insert(
            "extra_domains".to_string(),
            Value::Array(vec![Value::String("example-video.test".to_string())]),
        );
        ext.set_config(ExtractorConfig {
            enabled: true,
            options,
        })
        .unwrap();
        assert!(ext.matches(&doc("https://example-video.test/v/1")));
        assert!(!ext.matches(&doc("https://unrelated.test/v/1")));
    }

    #[test]
    fn set_config_rejects_unknown_options() {
        let mut ext = YtdlpExtractor::default();
        let mut options = Map::new();
        options.insert("nonsense".to_string(), Value::Bool(true));
        let err = ext
            .set_config(ExtractorConfig {
                enabled: true,
                options,
            })
            .unwrap_err();
        assert!(matches!(err, HisterError::InvalidConfig(_)));
    }

    #[test]
    fn extract_falls_back_when_the_binary_does_not_exist() {
        let mut ext = YtdlpExtractor::default();
        let mut options = ext.config().options.clone();
        options.insert(
            "binary".to_string(),
            Value::String("definitely-not-a-real-binary-xyz".to_string()),
        );
        ext.set_config(ExtractorConfig {
            enabled: true,
            options,
        })
        .unwrap();
        let d = doc("https://www.youtube.com/watch?v=abc123");
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn deserializes_a_realistic_yt_dlp_json_payload() {
        let json = r#"{
            "title": "A Talk About Rust",
            "description": "An introductory talk.",
            "uploader": "Rust Channel",
            "duration": 754.5,
            "view_count": 1200,
            "like_count": 42,
            "upload_date": "20260115",
            "thumbnail": "https://example.com/thumb.jpg",
            "webpage_url": "https://www.youtube.com/watch?v=abc123",
            "categories": ["Education"],
            "tags": ["rust", "programming"],
            "chapters": [{"start_time": 0.0, "end_time": 30.0, "title": "Intro"}],
            "subtitles": {"en": [{"ext": "vtt", "url": "https://example.com/en.vtt", "name": "English"}]},
            "automatic_captions": {},
            "language": "en",
            "playlist_title": "",
            "playlist_index": null,
            "playlist_count": null
        }"#;
        let info: VideoInfo = rusty_json::from_str(json).expect("valid yt-dlp JSON");
        assert_eq!(info.title, "A Talk About Rust");
        assert_eq!(info.duration, 754.5);
        assert_eq!(info.chapters.len(), 1);
        assert_eq!(info.chapters[0].title, "Intro");
        assert!(info.subtitles.contains_key("en"));
    }

    #[test]
    fn tolerates_missing_optional_fields() {
        let info: VideoInfo =
            rusty_json::from_str(r#"{"title": "Minimal"}"#).expect("minimal JSON parses");
        assert_eq!(info.title, "Minimal");
        assert_eq!(info.description, "");
        assert!(info.chapters.is_empty());
        assert!(info.playlist_index.is_none());
    }
}
