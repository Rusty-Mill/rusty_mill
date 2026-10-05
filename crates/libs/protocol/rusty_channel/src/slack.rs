//! Slack, over the Events API.
//!
//! Inbound: Slack `POST`s an event envelope, signed with the app's signing
//! secret (`X-Slack-Signature` is `v0=` + HMAC-SHA256 over
//! `v0:<timestamp>:<body>`, `X-Slack-Request-Timestamp` is the clock the
//! signature is bound to). Two event types are messages to the agent: an
//! `app_mention` anywhere, and a `message` in a direct message. A mention
//! starts (or continues) a Slack thread, and the reply goes into it, so
//! one Slack thread is one AG-UI thread. A direct message is one AG-UI
//! thread per person.
//!
//! Outbound: `chat.postMessage` with the bot token.

use rusty_json::Value;
use rusty_oauth::crypto::hmac::{constant_time_eq, hmac_sha256};

use crate::{Channel, Error, Headers, Inbound, Outbound, Received};

/// How far `X-Slack-Request-Timestamp` may be from `now`, in seconds.
/// Slack's own guidance; a replayed request older than this is refused.
pub const MAX_SKEW_SECS: u64 = 60 * 5;

/// A Slack app: its signing secret and the token it posts with.
#[derive(Clone)]
pub struct Slack {
    signing_secret: Vec<u8>,
    bot_token: String,
    api: String,
}

impl std::fmt::Debug for Slack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slack")
            .field("api", &self.api)
            .finish_non_exhaustive()
    }
}

impl Slack {
    /// An app with this signing secret (from "Basic Information") and bot
    /// token (`xoxb-…`, with `chat:write`).
    pub fn new(signing_secret: impl Into<Vec<u8>>, bot_token: impl Into<String>) -> Self {
        Slack {
            signing_secret: signing_secret.into(),
            bot_token: bot_token.into(),
            api: "https://slack.com/api".into(),
        }
    }

    /// Post to another API root (a test server).
    #[must_use]
    pub fn with_api(mut self, api: impl Into<String>) -> Self {
        self.api = api.into();
        self
    }

    /// The signature Slack would put on `body` at `timestamp`.
    pub fn sign(&self, timestamp: &str, body: &[u8]) -> String {
        let mut base = format!("v0:{timestamp}:").into_bytes();
        base.extend_from_slice(body);
        let mac = hmac_sha256(&self.signing_secret, &base);
        let mut out = String::with_capacity(3 + mac.len() * 2);
        out.push_str("v0=");
        for byte in mac.iter() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    fn verify(&self, headers: &dyn Headers, body: &[u8], now: u64) -> Result<(), Error> {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .ok_or_else(|| Error::Signature("no X-Slack-Request-Timestamp".into()))?;
        let signature = headers
            .get("x-slack-signature")
            .ok_or_else(|| Error::Signature("no X-Slack-Signature".into()))?;
        let at: u64 = timestamp
            .parse()
            .map_err(|_| Error::Signature("timestamp is not a number".into()))?;
        if now.abs_diff(at) > MAX_SKEW_SECS {
            return Err(Error::Signature(format!(
                "timestamp {at} is more than {MAX_SKEW_SECS}s from now ({now})"
            )));
        }
        let expected = self.sign(timestamp, body);
        if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
            return Err(Error::Signature("signature does not match".into()));
        }
        Ok(())
    }
}

impl Channel for Slack {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn receive(&self, headers: &dyn Headers, body: &[u8], now: u64) -> Result<Received, Error> {
        self.verify(headers, body, now)?;
        let text = std::str::from_utf8(body).map_err(|e| Error::Payload(e.to_string()))?;
        let envelope = Value::parse(text).map_err(|e| Error::Payload(e.to_string()))?;
        match str_of(&envelope, "type")? {
            "url_verification" => Ok(Received::Challenge(str_of(&envelope, "challenge")?.into())),
            "event_callback" => {
                // Slack retries when it gets no 200 within three seconds.
                // The first delivery was accepted and its run is under way;
                // running it again would answer twice.
                if headers.get("x-slack-retry-num").is_some() {
                    return Ok(Received::Ignored("retry of an accepted event"));
                }
                let event = envelope
                    .get("event")
                    .ok_or_else(|| Error::Payload("no event".into()))?;
                if event.get("bot_id").is_some() || event.get("subtype").is_some() {
                    return Ok(Received::Ignored(
                        "a bot's message or a non-message subtype",
                    ));
                }
                let in_dm = event.get("channel_type").and_then(Value::as_str) == Some("im");
                match str_of(event, "type")? {
                    "app_mention" => {}
                    "message" if in_dm => {}
                    "message" => {
                        return Ok(Received::Ignored("a channel message without a mention"))
                    }
                    _ => return Ok(Received::Ignored("an event this channel does not handle")),
                }
                let team = str_of(&envelope, "team_id")?;
                let channel = str_of(event, "channel")?;
                let ts = str_of(event, "ts")?;
                let thread_ts = event.get("thread_ts").and_then(Value::as_str);
                // A mention threads under itself; a direct message stays
                // flat unless the person is already in a thread.
                let root = if in_dm {
                    thread_ts
                } else {
                    Some(thread_ts.unwrap_or(ts))
                };
                let conversation = match root {
                    Some(root) => format!("{team}:{channel}:{root}"),
                    None => format!("{team}:{channel}"),
                };
                let mut reply_to = Value::Object(Default::default());
                let _ = reply_to.insert("channel", Value::String(channel.into()));
                if let Some(root) = root {
                    let _ = reply_to.insert("thread_ts", Value::String(root.into()));
                }
                Ok(Received::Message(Inbound {
                    conversation,
                    sender: str_of(event, "user")?.into(),
                    text: clean_text(str_of(event, "text")?),
                    reply_to,
                }))
            }
            other => Err(Error::Payload(format!("unknown envelope type {other:?}"))),
        }
    }

    fn reply(&self, to: &Inbound, text: &str) -> Outbound {
        let mut body = Value::Object(Default::default());
        if let Some(channel) = to.reply_to.get("channel") {
            let _ = body.insert("channel", channel.clone());
        }
        if let Some(thread_ts) = to.reply_to.get("thread_ts") {
            let _ = body.insert("thread_ts", thread_ts.clone());
        }
        let _ = body.insert("text", Value::String(escape(text)));
        Outbound {
            url: format!("{}/chat.postMessage", self.api),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", self.bot_token)),
                (
                    "Content-Type".into(),
                    "application/json; charset=utf-8".into(),
                ),
            ],
            body: body.to_json_string().into_bytes(),
        }
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Payload(format!("no string {key:?}")))
}

/// The person's words: mentions (`<@U123>`) removed, Slack's three HTML
/// escapes undone, whitespace trimmed.
pub fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text as Slack wants it posted: `&`, `<` and `>` escaped so the reply
/// cannot form links or mentions.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn slack() -> Slack {
        Slack::new("8f742231b10e8888abcd99yyyzzz85a5", "xoxb-token")
    }

    fn signed(app: &Slack, body: &str) -> Vec<(String, String)> {
        let ts = NOW.to_string();
        vec![
            ("X-Slack-Request-Timestamp".into(), ts.clone()),
            ("X-Slack-Signature".into(), app.sign(&ts, body.as_bytes())),
        ]
    }

    fn mention(thread_ts: Option<&str>) -> String {
        let thread = thread_ts
            .map(|t| format!(r#","thread_ts":"{t}""#))
            .unwrap_or_default();
        format!(
            r#"{{"type":"event_callback","team_id":"T1","event":{{"type":"app_mention","user":"U2","text":"<@U1> what&amp;s due &lt;today&gt;?","ts":"1700.0002","channel":"C9"{thread}}}}}"#
        )
    }

    #[test]
    fn the_signature_matches_an_independent_hmac() {
        // Python: hmac.new(secret, b"v0:1531420618:" + body, sha256).hexdigest()
        let app = Slack::new("8f742231b10e8888abcd99yyyzzz85a5", "");
        let body = br#"{"type":"url_verification","challenge":"abc"}"#;
        assert_eq!(
            app.sign("1531420618", body),
            "v0=ae6e383ee58dd0c92dd248df269b7153bdd89895087b58885a6703496ce22369"
        );
    }

    #[test]
    fn a_bad_signature_a_stale_clock_and_missing_headers_are_refused() {
        let app = slack();
        let body = mention(None);
        let mut headers = signed(&app, &body);
        assert!(matches!(
            app.receive(&headers, body.as_bytes(), NOW + 10),
            Ok(Received::Message(_))
        ));

        assert!(matches!(
            app.receive(&headers, body.as_bytes(), NOW + MAX_SKEW_SECS + 1),
            Err(Error::Signature(why)) if why.contains("from now")
        ));
        headers[1].1 = "v0=0000".into();
        assert!(matches!(
            app.receive(&headers, body.as_bytes(), NOW),
            Err(Error::Signature(_))
        ));
        let none: [(&str, &str); 0] = [];
        assert!(matches!(
            app.receive(&none, body.as_bytes(), NOW),
            Err(Error::Signature(_))
        ));
        // The body is read only after the signature holds.
        let tampered = body.replace("U2", "U3");
        assert!(matches!(
            app.receive(&signed(&app, &body), tampered.as_bytes(), NOW),
            Err(Error::Signature(_))
        ));
    }

    #[test]
    fn url_verification_answers_the_challenge() {
        let app = slack();
        let body = r#"{"type":"url_verification","token":"x","challenge":"3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO7tYXAYM8P"}"#;
        assert_eq!(
            app.receive(&signed(&app, body), body.as_bytes(), NOW),
            Ok(Received::Challenge(
                "3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO7tYXAYM8P".into()
            ))
        );
    }

    #[test]
    fn a_mention_is_a_threaded_conversation_with_clean_text() {
        let app = slack();
        let body = mention(None);
        let Ok(Received::Message(inbound)) =
            app.receive(&signed(&app, &body), body.as_bytes(), NOW)
        else {
            panic!("a mention is a message");
        };
        assert_eq!(
            inbound.conversation, "T1:C9:1700.0002",
            "a fresh mention roots its own thread"
        );
        assert_eq!(inbound.sender, "U2");
        assert_eq!(inbound.text, "what&s due <today>?");
        assert_eq!(
            inbound.reply_to.get("thread_ts").and_then(Value::as_str),
            Some("1700.0002")
        );

        let body = mention(Some("1699.0001"));
        let Ok(Received::Message(inbound)) =
            app.receive(&signed(&app, &body), body.as_bytes(), NOW)
        else {
            panic!("a mention is a message");
        };
        assert_eq!(
            inbound.conversation, "T1:C9:1699.0001",
            "a mention in a thread continues it"
        );
    }

    #[test]
    fn a_direct_message_is_one_conversation_per_person() {
        let app = slack();
        let body = r#"{"type":"event_callback","team_id":"T1","event":{"type":"message","channel_type":"im","user":"U2","text":"hi","ts":"1700.0003","channel":"D5"}}"#;
        let Ok(Received::Message(inbound)) = app.receive(&signed(&app, body), body.as_bytes(), NOW)
        else {
            panic!("a direct message is a message");
        };
        assert_eq!(inbound.conversation, "T1:D5");
        assert_eq!(
            inbound.reply_to.get("thread_ts"),
            None,
            "a flat DM gets a flat reply"
        );

        let out = app.reply(&inbound, "a <b> & c");
        assert_eq!(out.url, "https://slack.com/api/chat.postMessage");
        assert_eq!(out.headers[0].1, "Bearer xoxb-token");
        let posted = Value::parse(std::str::from_utf8(&out.body).expect("utf8")).expect("json");
        assert_eq!(posted.get("channel").and_then(Value::as_str), Some("D5"));
        assert_eq!(
            posted.get("text").and_then(Value::as_str),
            Some("a &lt;b&gt; &amp; c")
        );
        assert_eq!(posted.get("thread_ts"), None);
    }

    #[test]
    fn bots_retries_and_plain_channel_messages_are_ignored() {
        let app = slack();
        let cases = [
            (
                r#"{"type":"event_callback","team_id":"T1","event":{"type":"message","channel_type":"im","bot_id":"B1","text":"me","ts":"1","channel":"D5"}}"#,
                "bot",
            ),
            (
                r#"{"type":"event_callback","team_id":"T1","event":{"type":"message","channel_type":"im","subtype":"message_changed","ts":"1","channel":"D5"}}"#,
                "subtype",
            ),
            (
                r#"{"type":"event_callback","team_id":"T1","event":{"type":"message","channel_type":"channel","user":"U2","text":"hi","ts":"1","channel":"C9"}}"#,
                "mention",
            ),
            (
                r#"{"type":"event_callback","team_id":"T1","event":{"type":"reaction_added","user":"U2"}}"#,
                "handle",
            ),
        ];
        for (body, word) in cases {
            match app.receive(&signed(&app, body), body.as_bytes(), NOW) {
                Ok(Received::Ignored(why)) => {
                    assert!(why.contains(word), "{why} should mention {word}")
                }
                other => panic!("{body}: {other:?}"),
            }
        }
        let body = mention(None);
        let mut headers = signed(&app, &body);
        headers.push(("X-Slack-Retry-Num".into(), "1".into()));
        assert_eq!(
            app.receive(&headers, body.as_bytes(), NOW),
            Ok(Received::Ignored("retry of an accepted event"))
        );

        let body = r#"{"type":"something_else"}"#;
        assert!(matches!(
            app.receive(&signed(&app, body), body.as_bytes(), NOW),
            Err(Error::Payload(_))
        ));
    }

    #[test]
    fn text_is_cleaned_and_escaped() {
        assert_eq!(clean_text("  <@U1>   hello <@U2>  "), "hello");
        assert_eq!(clean_text("a &amp;&lt; b"), "a &< b");
        assert_eq!(
            clean_text("<@U1"),
            "<@U1",
            "an unterminated mention is left alone"
        );
        assert_eq!(escape("<&>"), "&lt;&amp;&gt;");
    }
}
