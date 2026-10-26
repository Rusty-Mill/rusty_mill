//! OSC 7501 report bodies: `key=value` pairs joined by `:`.

use crate::{Id, Kind, Record, Report, State};

/// The longest body: the protocol's 4096-byte sequence limit less the
/// longest framing, `ESC ] 7501 ;` and `ESC \`.
const MAX_BODY: usize = 4096 - 9;
const MAX_KEY: usize = 16;
const MAX_SEGMENT: usize = 32;
const MAX_DEPTH: usize = 8;
const MAX_ID: usize = 128;
const MAX_APP: usize = 32;
/// (encoded, decoded) byte limits.
const TITLE_LIMITS: (usize, usize) = (256, 192);
const MSG_LIMITS: (usize, usize) = (2732, 2048);

/// The report in `body` (the text after `7501;`), or `None` when the
/// protocol says to ignore it (no or unknown `state`, a bad `id`) or discard
/// it (over a size limit, bad base64, control characters in text).
/// Malformed pairs and unknown keys are skipped, and a repeated key keeps its
/// last value.
pub fn parse(body: &str) -> Option<Report> {
    if body.len() > MAX_BODY {
        return None;
    }
    let mut fields = Fields::default();
    for (key, value) in body.split(':').filter_map(pair) {
        fields.set(key, value);
    }
    let id = match fields.id {
        Some(id) => valid_id(id)?,
        None => Id::default(),
    };
    let state = match fields.state? {
        "clear" => return Some(Report::Clear(id)),
        other => state(other)?,
    };
    let record = Record {
        state,
        kind: fields
            .kind
            .filter(|_| state == State::Blocked)
            .and_then(kind),
        progress: fields
            .progress
            .filter(|_| matches!(state, State::Working | State::Blocked))
            .and_then(progress),
        app: fields
            .app
            .filter(|a| is_segment(a, MAX_APP))
            .map(str::to_owned),
        title: optional_text(fields.title, TITLE_LIMITS)?,
        msg: optional_text(fields.msg, MSG_LIMITS)?,
    };
    Some(Report::Set(id, record))
}

/// The last value of each key this crate knows.
#[derive(Default)]
struct Fields<'a> {
    state: Option<&'a str>,
    id: Option<&'a str>,
    kind: Option<&'a str>,
    progress: Option<&'a str>,
    app: Option<&'a str>,
    title: Option<&'a str>,
    msg: Option<&'a str>,
}

impl<'a> Fields<'a> {
    fn set(&mut self, key: &str, value: &'a str) {
        let slot = match key {
            "state" => &mut self.state,
            "id" => &mut self.id,
            "kind" => &mut self.kind,
            "progress" => &mut self.progress,
            "app" => &mut self.app,
            "title" => &mut self.title,
            "msg" => &mut self.msg,
            _ => return,
        };
        *slot = Some(value);
    }
}

/// A `key=value` with a well-formed key, trimmed; `None` skips it. Each
/// field checks its own value.
fn pair(text: &str) -> Option<(&str, &str)> {
    let (key, value) = text.split_once('=')?;
    let (key, value) = (key.trim(), value.trim());
    let key_ok =
        !key.is_empty() && key.len() <= MAX_KEY && key.bytes().all(|b| b.is_ascii_lowercase());
    key_ok.then_some((key, value))
}

fn state(text: &str) -> Option<State> {
    Some(match text {
        "idle" => State::Idle,
        "working" => State::Working,
        "blocked" => State::Blocked,
        "done" => State::Done,
        "error" => State::Error,
        _ => return None,
    })
}

fn kind(text: &str) -> Option<Kind> {
    Some(match text {
        "permission" => Kind::Permission,
        "question" => Kind::Question,
        "auth" => Kind::Auth,
        _ => return None,
    })
}

fn progress(text: &str) -> Option<u8> {
    text.parse::<u8>().ok().filter(|p| *p <= 100)
}

fn valid_id(text: &str) -> Option<Id> {
    let ok = text.len() <= MAX_ID
        && text.split('/').count() <= MAX_DEPTH
        && text.split('/').all(|s| is_segment(s, MAX_SEGMENT));
    ok.then(|| Id(text.to_owned()))
}

/// 1 to `max` characters from letters, digits and `_ . + -`.
fn is_segment(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
}

/// `Some(None)` when absent, `None` when present but invalid.
fn optional_text(encoded: Option<&str>, limits: (usize, usize)) -> Option<Option<String>> {
    match encoded {
        Some(encoded) => text(encoded, limits).map(Some),
        None => Some(None),
    }
}

/// Base64 UTF-8 within `(encoded, decoded)` limits, single line and free of
/// control characters; `None` discards the report.
fn text(encoded: &str, (max_encoded, max_decoded): (usize, usize)) -> Option<String> {
    if encoded.len() > max_encoded {
        return None;
    }
    let bytes = rusty_base64::decode_standard(encoded).ok()?;
    if bytes.len() > max_decoded {
        return None;
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|t| !t.chars().any(char::is_control))
}

#[cfg(test)]
mod tests;
