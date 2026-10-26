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
/// it. A report is discarded whole when any pair, repeated, unknown or
/// final, breaks a size limit, or its text is bad base64 or holds control
/// characters; that holds for a clear too, so nothing invalid touches a
/// record. Malformed pairs are skipped before a repeated key keeps its last
/// value, and unknown keys are ignored.
pub fn parse(body: &str) -> Option<Report> {
    if body.len() > MAX_BODY {
        return None;
    }
    let mut fields = Fields::default();
    for (key, value) in body.split(':').filter_map(pair) {
        fields.set(key, value)?;
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
        title: fields.title,
        msg: fields.msg,
    };
    Some(Report::Set(id, record))
}

/// The last value of each key this crate knows, text already decoded.
#[derive(Default)]
struct Fields<'a> {
    state: Option<&'a str>,
    id: Option<&'a str>,
    kind: Option<&'a str>,
    progress: Option<&'a str>,
    app: Option<&'a str>,
    title: Option<String>,
    msg: Option<String>,
}

impl<'a> Fields<'a> {
    /// Checks one well-formed pair against the limits and keeps its value;
    /// `None` discards the report.
    fn set(&mut self, key: &str, value: &'a str) -> Option<()> {
        if key.len() > MAX_KEY {
            return None;
        }
        match key {
            "state" => self.state = Some(value),
            "id" if !within_id_limits(value) => return None,
            "id" => self.id = Some(value),
            "kind" => self.kind = Some(value),
            "progress" => self.progress = Some(value),
            "app" if value.len() > MAX_APP => return None,
            "app" => self.app = Some(value),
            "title" => self.title = Some(text(value, TITLE_LIMITS)?),
            "msg" => self.msg = Some(text(value, MSG_LIMITS)?),
            _ => {}
        }
        Some(())
    }
}

/// A well-formed `key=value`, trimmed: a key of lowercase letters and a
/// value from the protocol's value set. `None` skips it.
fn pair(text: &str) -> Option<(&str, &str)> {
    let (key, value) = text.split_once('=')?;
    let (key, value) = (key.trim(), value.trim());
    let key_ok = !key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase());
    let value_ok = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b));
    (key_ok && value_ok).then_some((key, value))
}

/// The id size limits, which discard a report; the rest of the id grammar
/// is [`valid_id`]'s, which ignores one.
fn within_id_limits(text: &str) -> bool {
    text.len() <= MAX_ID
        && text.split('/').count() <= MAX_DEPTH
        && text.split('/').all(|s| s.len() <= MAX_SEGMENT)
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

/// An id within the limits that matches the grammar: non-empty segments of
/// the id characters.
fn valid_id(text: &str) -> Option<Id> {
    let ok = text.split('/').all(|s| is_segment(s, MAX_SEGMENT));
    ok.then(|| Id(text.to_owned()))
}

/// 1 to `max` characters from letters, digits and `_ . + -`.
fn is_segment(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
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
