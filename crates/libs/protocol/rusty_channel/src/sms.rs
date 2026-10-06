//! SMS, over Twilio.
//!
//! Inbound: Twilio `POST`s a form (`From`, `To`, `Body`, `MessageSid`, …)
//! to the webhook, signed in `X-Twilio-Signature`: base64 of HMAC-SHA1
//! over the webhook's public URL followed by every form field, sorted by
//! name, as `name` then `value` with nothing between. The channel must
//! therefore know its own public URL, exactly as Twilio was given it.
//! There is no timestamp, so a replay is only as detectable as a repeated
//! `MessageSid`, which the runner does not track yet.
//!
//! SMS has no threads: one conversation per pair of numbers. Twilio
//! expects TwiML back, so the acknowledgement is an empty `<Response/>`
//! and the reply itself goes through the REST API (`Messages.json`, basic
//! auth with the account SID and auth token) once the agent has answered.

use rusty_oauth::crypto::hmac::constant_time_eq;
use rusty_oauth::encoding::percent;
use rusty_sha1::Sha1;

use crate::{Ack, Channel, Error, Headers, Inbound, Outbound, Received};

/// Twilio concatenates up to this many characters into one message and
/// refuses anything longer; a reply is cut here.
pub const MAX_BODY_CHARS: usize = 1600;

/// A Twilio account and the webhook it signs for.
#[derive(Clone)]
pub struct Twilio {
    account_sid: String,
    auth_token: String,
    webhook_url: String,
    api: String,
}

impl std::fmt::Debug for Twilio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Twilio")
            .field("account_sid", &self.account_sid)
            .field("webhook_url", &self.webhook_url)
            .finish_non_exhaustive()
    }
}

impl Twilio {
    /// An account (`AC…`) with its auth token, receiving at `webhook_url`:
    /// the public URL the number's messaging webhook is set to, query
    /// string included, since the signature covers it.
    pub fn new(
        account_sid: impl Into<String>,
        auth_token: impl Into<String>,
        webhook_url: impl Into<String>,
    ) -> Self {
        Twilio {
            account_sid: account_sid.into(),
            auth_token: auth_token.into(),
            webhook_url: webhook_url.into(),
            api: "https://api.twilio.com".into(),
        }
    }

    /// Send through another API root (a test server).
    #[must_use]
    pub fn with_api(mut self, api: impl Into<String>) -> Self {
        self.api = api.into();
        self
    }

    /// The signature Twilio would put on `fields` posted to the webhook.
    pub fn sign(&self, fields: &[(String, String)]) -> String {
        let mut sorted: Vec<&(String, String)> = fields.iter().collect();
        sorted.sort();
        let mut base = self.webhook_url.clone();
        for (name, value) in sorted {
            base.push_str(name);
            base.push_str(value);
        }
        rusty_base64::encode_standard(&hmac_sha1(self.auth_token.as_bytes(), base.as_bytes()))
    }
}

impl Channel for Twilio {
    fn name(&self) -> &'static str {
        "sms"
    }

    fn receive(&self, headers: &dyn Headers, body: &[u8], _now: u64) -> Result<Received, Error> {
        let signature = headers
            .get("x-twilio-signature")
            .ok_or_else(|| Error::Signature("no X-Twilio-Signature".into()))?;
        let text = std::str::from_utf8(body).map_err(|e| Error::Signature(e.to_string()))?;
        let fields = percent::form_urldecode(text).map_err(|e| Error::Signature(e.to_string()))?;
        if !constant_time_eq(self.sign(&fields).as_bytes(), signature.as_bytes()) {
            return Err(Error::Signature("signature does not match".into()));
        }
        let field = |name: &str| {
            fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        let from = field("From").ok_or_else(|| Error::Payload("no From".into()))?;
        let to = field("To").ok_or_else(|| Error::Payload("no To".into()))?;
        let text = field("Body").map(str::trim).unwrap_or("");
        if text.is_empty() {
            return Ok(Received::Ignored("a message without text"));
        }
        let mut reply_to = rusty_json::Value::Object(Default::default());
        let _ = reply_to.insert("to", rusty_json::Value::String(from.into()));
        let _ = reply_to.insert("from", rusty_json::Value::String(to.into()));
        Ok(Received::Message(Inbound {
            conversation: format!("{to}:{from}"),
            sender: from.into(),
            text: text.into(),
            reply_to,
        }))
    }

    fn ack(&self) -> Ack {
        Ack {
            content_type: "text/xml",
            body: b"<Response/>",
        }
    }

    fn reply(&self, to: &Inbound, text: &str) -> Outbound {
        let field = |key: &str| {
            to.reply_to
                .get(key)
                .and_then(rusty_json::Value::as_str)
                .unwrap_or("")
        };
        let body: String = match text.char_indices().nth(MAX_BODY_CHARS - 1) {
            Some((cut, _)) if text[cut..].chars().count() > 1 => format!("{}…", &text[..cut]),
            _ => text.into(),
        };
        let credentials = rusty_base64::encode_standard(
            format!("{}:{}", self.account_sid, self.auth_token).as_bytes(),
        );
        Outbound {
            url: format!(
                "{}/2010-04-01/Accounts/{}/Messages.json",
                self.api,
                percent::encode(&self.account_sid)
            ),
            headers: vec![
                ("Authorization".into(), format!("Basic {credentials}")),
                (
                    "Content-Type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
            ],
            body: percent::form_urlencode([
                ("To", field("to")),
                ("From", field("from")),
                ("Body", body.as_str()),
            ])
            .into_bytes(),
        }
    }
}

/// HMAC-SHA1 (RFC 2104), as Twilio signs with.
fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK: usize = 64;
    let mut block_key = [0u8; BLOCK];
    if key.len() > BLOCK {
        block_key[..20].copy_from_slice(&rusty_sha1::sha1(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha1::new();
    inner.update(&block_key.map(|b| b ^ 0x36));
    inner.update(message);
    let inner = inner.finish();
    let mut outer = Sha1::new();
    outer.update(&block_key.map(|b| b ^ 0x5c));
    outer.update(&inner);
    outer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn twilio() -> Twilio {
        Twilio::new(
            "AC123",
            "12345",
            "https://mycompany.com/myapp.php?foo=1&bar=2",
        )
    }

    fn form(fields: &[(&str, &str)]) -> (String, Vec<(String, String)>) {
        let owned: Vec<(String, String)> = fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        (percent::form_urlencode(fields.iter().copied()), owned)
    }

    #[test]
    fn the_signature_matches_an_independent_hmac_sha1() {
        // Python: base64(hmac.new(b"12345", (url + "".join(k + v for k, v in
        // sorted(params.items()))).encode(), sha1).digest())
        let (_, fields) = form(&[
            ("CallSid", "CA1234567890ABCDE"),
            ("Caller", "+12349013030"),
            ("Digits", "1234"),
            ("From", "+12349013030"),
            ("To", "+18005551212"),
        ]);
        assert_eq!(twilio().sign(&fields), "0/KCTR6DLpKmkAf8muzZqo1nDgQ=");
    }

    #[test]
    fn hmac_sha1_matches_rfc_2202() {
        let mac = hmac_sha1(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            rusty_sha1::hex(&mac),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        let long_key = [0xaa; 80];
        let mac = hmac_sha1(
            &long_key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            rusty_sha1::hex(&mac),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }

    #[test]
    fn a_signed_text_is_one_conversation_per_pair_of_numbers() {
        let app = twilio();
        let (body, fields) = form(&[
            ("MessageSid", "SM1"),
            ("From", "+15551234567"),
            ("To", "+15557654321"),
            ("Body", " what's due? "),
        ]);
        let headers = [("X-Twilio-Signature", app.sign(&fields))];
        let Ok(Received::Message(inbound)) = app.receive(&headers, body.as_bytes(), 0) else {
            panic!("a signed text is a message");
        };
        assert_eq!(inbound.conversation, "+15557654321:+15551234567");
        assert_eq!(inbound.sender, "+15551234567");
        assert_eq!(inbound.text, "what's due?");
        assert_eq!(
            app.ack(),
            Ack {
                content_type: "text/xml",
                body: b"<Response/>"
            }
        );

        let out = app.reply(&inbound, "Two tasks & a note");
        assert_eq!(
            out.url,
            "https://api.twilio.com/2010-04-01/Accounts/AC123/Messages.json"
        );
        assert_eq!(
            out.headers[0].1,
            format!("Basic {}", rusty_base64::encode_standard(b"AC123:12345"))
        );
        let posted =
            percent::form_urldecode(std::str::from_utf8(&out.body).expect("utf8")).expect("form");
        assert_eq!(
            posted,
            vec![
                ("To".to_string(), "+15551234567".to_string()),
                ("From".to_string(), "+15557654321".to_string()),
                ("Body".to_string(), "Two tasks & a note".to_string()),
            ]
        );
    }

    #[test]
    fn a_bad_signature_a_missing_header_and_a_changed_body_are_refused() {
        let app = twilio();
        let (body, fields) = form(&[("From", "+1"), ("To", "+2"), ("Body", "hi")]);
        let good = app.sign(&fields);
        let none: [(&str, &str); 0] = [];
        assert!(matches!(
            app.receive(&none, body.as_bytes(), 0),
            Err(Error::Signature(_))
        ));
        assert!(matches!(
            app.receive(&[("X-Twilio-Signature", "nope")], body.as_bytes(), 0),
            Err(Error::Signature(_))
        ));
        let changed = body.replace("hi", "ho");
        assert!(matches!(
            app.receive(
                &[("X-Twilio-Signature", good.clone())],
                changed.as_bytes(),
                0
            ),
            Err(Error::Signature(_))
        ));
        // The same fields signed for another URL: a webhook moved without
        // telling the channel is refused rather than trusted.
        let elsewhere = Twilio::new("AC123", "12345", "https://other.example/hook");
        assert!(matches!(
            elsewhere.receive(&[("X-Twilio-Signature", good)], body.as_bytes(), 0),
            Err(Error::Signature(_))
        ));
    }

    #[test]
    fn a_text_without_words_is_ignored_and_long_replies_are_cut() {
        let app = twilio();
        let (body, fields) = form(&[("From", "+1"), ("To", "+2"), ("NumMedia", "1")]);
        assert_eq!(
            app.receive(
                &[("X-Twilio-Signature", app.sign(&fields))],
                body.as_bytes(),
                0
            ),
            Ok(Received::Ignored("a message without text"))
        );
        let inbound = Inbound {
            conversation: "+2:+1".into(),
            sender: "+1".into(),
            text: "x".into(),
            reply_to: rusty_json::Value::Null,
        };
        let long = "é".repeat(MAX_BODY_CHARS + 5);
        let out = app.reply(&inbound, &long);
        let posted =
            percent::form_urldecode(std::str::from_utf8(&out.body).expect("utf8")).expect("form");
        let sent = &posted[2].1;
        assert_eq!(sent.chars().count(), MAX_BODY_CHARS);
        assert!(sent.ends_with('…'));
        let exact = "y".repeat(MAX_BODY_CHARS);
        let out = app.reply(&inbound, &exact);
        assert!(
            String::from_utf8_lossy(&out.body).ends_with(&exact),
            "an exact fit is not cut"
        );
    }
}
