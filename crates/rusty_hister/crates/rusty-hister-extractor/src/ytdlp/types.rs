//! A Rust port of `server/extractor/extractors/ytdlp/types.go`: the subset
//! of yt-dlp's `--dump-json` output this extractor cares about, plus the
//! host lists [`matches`](super::YtdlpExtractor::matches) checks against.
//!
//! Deserialized with `#[derive(serde::Deserialize)]` via
//! `rusty_json::from_str` rather than walking a `rusty_json::Value` by
//! hand field-by-field (the approach `RedditExtractor`/`DiscourseExtractor`
//! use for their JSON-LD/schema.org payloads) — those need that approach
//! because the shape being matched is genuinely open-ended (which
//! `@type` is this, which keys does *this* variant have); yt-dlp's JSON
//! has one fixed shape, so a plain `Deserialize` struct is simpler and
//! exactly as faithful.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SubtitleTrack {
    #[allow(dead_code)]
    #[serde(default)]
    pub ext: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub url: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Chapter {
    #[serde(default)]
    pub start_time: f64,
    #[allow(dead_code)]
    #[serde(default)]
    pub end_time: f64,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct VideoInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub uploader: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub view_count: i64,
    #[allow(dead_code)]
    #[serde(default)]
    pub like_count: i64,
    #[serde(default)]
    pub upload_date: String,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub webpage_url: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub subtitles: std::collections::HashMap<String, Vec<SubtitleTrack>>,
    #[serde(default)]
    pub automatic_captions: std::collections::HashMap<String, Vec<SubtitleTrack>>,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub playlist_title: String,
    #[serde(default)]
    pub playlist_index: Option<i64>,
    #[serde(default)]
    pub playlist_count: Option<i64>,
}

/// Domains yt-dlp commonly supports. `matches` accepts a URL whose host
/// equals or is a subdomain of one of these (e.g. `youtube.com` matches
/// `www.youtube.com`).
pub(super) const KNOWN_DOMAINS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "vimeo.com",
    "dailymotion.com",
    "twitch.tv",
    "bilibili.com",
    "nicovideo.jp",
    "soundcloud.com",
    "bandcamp.com",
    "mixcloud.com",
    "ted.com",
    "media.ccc.de",
    "tiktok.com",
];

/// Hostname fragments matched by substring, for platforms with
/// user-chosen subdomains (e.g. `instance.peertube.live`).
pub(super) const KNOWN_HOST_SUBSTRINGS: &[&str] = &["peertube"];
