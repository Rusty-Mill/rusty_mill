//! Microsoft Teams, over the Azure Bot Framework.
//!
//! Inbound: the Bot Framework `POST`s an activity to the bot's messaging
//! endpoint with a bearer JWT it signed (`RS256`, keys at [`KEYS_URL`]).
//! The token is checked for signature, issuer, audience (the app id),
//! validity window and the `serviceurl` claim, which must name the same
//! service the activity says to reply to; a forged activity pointing at
//! another host is refused before its body is read. Only `message`
//! activities are messages to the agent; a Teams conversation id already
//! names the thread (a channel reply carries `;messageid=`), so one Teams
//! thread or chat is one AG-UI thread.
//!
//! Outbound: a reply activity to `{serviceUrl}v3/conversations/{id}/
//! activities/{activityId}`, with a bearer token from the client
//! credentials grant. The token is short-lived, so the channel asks the
//! runner to fetch one through [`Channel::credential`] before a reply.

use std::sync::Mutex;

use rusty_json::Value;
use rusty_oauth::encoding::percent;
use rusty_oauth::jwks::JwkSet;
use rusty_oauth::jwt;

use crate::{Channel, Credential, Error, Headers, Inbound, Outbound, Received};

/// Where the Bot Framework publishes its signing keys (a JWK Set).
pub const KEYS_URL: &str = "https://login.botframework.com/v1/.well-known/keys";
/// Issuer of the tokens on inbound activities.
pub const ISSUER: &str = "https://api.botframework.com";
/// The client credentials endpoint for outbound tokens.
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/botframework.com/oauth2/v2.0/token";
/// The scope of an outbound token.
pub const TOKEN_SCOPE: &str = "https://api.botframework.com/.default";
/// Clock skew tolerated on `exp` and `nbf`, in seconds.
pub const LEEWAY_SECS: u64 = 60 * 5;
/// A token is fetched again this many seconds before it expires.
const TOKEN_MARGIN_SECS: u64 = 60;

/// A Teams app: its id, its password (client secret), the framework's
/// current signing keys, and the outbound token once fetched.
pub struct Teams {
    app_id: String,
    app_password: String,
    keys: JwkSet,
    token_url: String,
    token: Mutex<Option<(String, u64)>>,
}

impl std::fmt::Debug for Teams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Teams")
            .field("app_id", &self.app_id)
            .field("keys", &self.keys.keys.len())
            .finish_non_exhaustive()
    }
}

impl Teams {
    /// An app with this id and password, verifying inbound tokens against
    /// `keys` (the body of [`KEYS_URL`], fetched by the runner).
    pub fn new(app_id: impl Into<String>, app_password: impl Into<String>, keys: JwkSet) -> Self {
        Teams {
            app_id: app_id.into(),
            app_password: app_password.into(),
            keys,
            token_url: TOKEN_URL.into(),
            token: Mutex::new(None),
        }
    }

    /// Fetch outbound tokens from another endpoint (a test server).
    #[must_use]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    /// Replace the signing keys (the framework rotates them).
    pub fn set_keys(&mut self, keys: JwkSet) {
        self.keys = keys;
    }

    /// The claims of a valid inbound token, or why it is not one.
    fn verify(&self, headers: &dyn Headers, now: u64) -> Result<Value, Error> {
        let token = headers
            .get("authorization")
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .ok_or_else(|| Error::Signature("no bearer token".into()))?;
        let kid = jwt::decode_unverified(token)
            .ok()
            .and_then(|d| {
                d.header
                    .get("kid")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .ok_or_else(|| Error::Signature("token has no kid".into()))?;
        let key = self
            .keys
            .rsa_key(&kid)
            .map_err(|e| Error::Signature(format!("no key for kid {kid:?}: {e}")))?;
        let claims =
            jwt::rsa::verify_rs256(token, &key).map_err(|e| Error::Signature(e.to_string()))?;
        if claims.get("iss").and_then(Value::as_str) != Some(ISSUER) {
            return Err(Error::Signature("issuer is not the Bot Framework".into()));
        }
        let audience = claims.get("aud").and_then(Value::as_str);
        if audience != Some(self.app_id.as_str()) {
            return Err(Error::Signature(format!(
                "audience {audience:?} is not this app"
            )));
        }
        let exp = claims
            .get("exp")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Signature("token has no exp".into()))?;
        if exp.max(0) as u64 + LEEWAY_SECS <= now {
            return Err(Error::Signature(format!(
                "token expired at {exp} (now {now})"
            )));
        }
        if let Some(nbf) = claims.get("nbf").and_then(Value::as_i64) {
            if nbf.max(0) as u64 > now + LEEWAY_SECS {
                return Err(Error::Signature(format!(
                    "token not valid before {nbf} (now {now})"
                )));
            }
        }
        if claims.get("serviceurl").and_then(Value::as_str).is_none() {
            return Err(Error::Signature("token has no serviceurl claim".into()));
        }
        Ok(claims)
    }
}

impl Channel for Teams {
    fn name(&self) -> &'static str {
        "teams"
    }

    fn receive(&self, headers: &dyn Headers, body: &[u8], now: u64) -> Result<Received, Error> {
        let claims = self.verify(headers, now)?;
        let text = std::str::from_utf8(body).map_err(|e| Error::Payload(e.to_string()))?;
        let activity = Value::parse(text).map_err(|e| Error::Payload(e.to_string()))?;
        let service_url = with_slash(str_of(&activity, "serviceUrl")?);
        let claimed = with_slash(
            claims
                .get("serviceurl")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        if !service_url.eq_ignore_ascii_case(&claimed) {
            return Err(Error::Signature(format!(
                "activity serviceUrl {service_url:?} is not the token's {claimed:?}"
            )));
        }
        if str_of(&activity, "type")? != "message" {
            return Ok(Received::Ignored("not a message activity"));
        }
        let from = activity
            .get("from")
            .ok_or_else(|| Error::Payload("no from".into()))?;
        if from.get("role").and_then(Value::as_str) == Some("bot") {
            return Ok(Received::Ignored("a bot's message"));
        }
        let conversation = activity
            .get("conversation")
            .map(|c| str_of(c, "id"))
            .ok_or_else(|| Error::Payload("no conversation".into()))??;
        let bot = activity
            .get("recipient")
            .map(|r| str_of(r, "id"))
            .ok_or_else(|| Error::Payload("no recipient".into()))??;
        let mut reply_to = Value::Object(Default::default());
        let _ = reply_to.insert("serviceUrl", Value::String(service_url));
        let _ = reply_to.insert("conversation", Value::String(conversation.into()));
        let _ = reply_to.insert("activity", Value::String(str_of(&activity, "id")?.into()));
        let _ = reply_to.insert("bot", Value::String(bot.into()));
        let _ = reply_to.insert("user", Value::String(str_of(from, "id")?.into()));
        Ok(Received::Message(Inbound {
            conversation: conversation.into(),
            sender: str_of(from, "id")?.into(),
            text: clean_text(activity.get("text").and_then(Value::as_str).unwrap_or("")),
            reply_to,
        }))
    }

    fn credential(&self, now: u64) -> Credential {
        let fresh = self
            .token
            .lock()
            .ok()
            .and_then(|t| {
                t.as_ref()
                    .map(|(_, expires)| *expires > now + TOKEN_MARGIN_SECS)
            })
            .unwrap_or(false);
        if fresh {
            return Credential::Ready;
        }
        Credential::Fetch(Outbound {
            url: self.token_url.clone(),
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: percent::form_urlencode([
                ("grant_type", "client_credentials"),
                ("client_id", self.app_id.as_str()),
                ("client_secret", self.app_password.as_str()),
                ("scope", TOKEN_SCOPE),
            ])
            .into_bytes(),
        })
    }

    fn accept_credential(&self, body: &[u8], now: u64) -> Result<(), Error> {
        let text = std::str::from_utf8(body).map_err(|e| Error::Payload(e.to_string()))?;
        let value = Value::parse(text).map_err(|e| Error::Payload(e.to_string()))?;
        let token = str_of(&value, "access_token")?;
        let expires_in = value
            .get("expires_in")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Payload("no expires_in".into()))?;
        if let Ok(mut slot) = self.token.lock() {
            *slot = Some((token.into(), now + expires_in.max(0) as u64));
        }
        Ok(())
    }

    fn reply(&self, to: &Inbound, text: &str) -> Outbound {
        let field = |key: &str| to.reply_to.get(key).and_then(Value::as_str).unwrap_or("");
        let token = self
            .token
            .lock()
            .ok()
            .and_then(|t| t.as_ref().map(|(token, _)| token.clone()))
            .unwrap_or_default();
        let mut body = Value::Object(Default::default());
        let _ = body.insert("type", Value::String("message".into()));
        let _ = body.insert("text", Value::String(text.into()));
        let _ = body.insert("replyToId", Value::String(field("activity").into()));
        let mut from = Value::Object(Default::default());
        let _ = from.insert("id", Value::String(field("bot").into()));
        let _ = body.insert("from", from);
        let mut recipient = Value::Object(Default::default());
        let _ = recipient.insert("id", Value::String(field("user").into()));
        let _ = body.insert("recipient", recipient);
        let mut conversation = Value::Object(Default::default());
        let _ = conversation.insert("id", Value::String(field("conversation").into()));
        let _ = body.insert("conversation", conversation);
        Outbound {
            url: format!(
                "{}v3/conversations/{}/activities/{}",
                field("serviceUrl"),
                percent::encode(field("conversation")),
                percent::encode(field("activity"))
            ),
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
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

fn with_slash(url: &str) -> String {
    let mut url = url.to_string();
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// The person's words: `<at>…</at>` mentions removed, whitespace trimmed.
pub fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<at>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</at>") {
            Some(end) => rest = &rest[start + end + "</at>".len()..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // An RSA key generated with `openssl genrsa 2048` and tokens signed with
    // `openssl dgst -sha256 -sign`: produced outside this crate. The claims
    // are iss, aud "app-id", exp 1700001000, nbf 1699999000, serviceurl.
    const JWKS: &str = r#"{"keys":[{"kty":"RSA","use":"sig","kid":"k1","n":"1dZzrIkXcyC1fPuLMYTLsgeLxIaZPAdf-nzWqd_uhGwCXyrbsgMNsooiyVRJ0kPJWQ7qhPWxk4nM_AVD_T9jpK6FaIP9kjUFXk6Z7jcWgLlg-pWI_srVKfd_IumtfE8wYR0Oe98bXBt6ioRttBQBvLTDfAjDn2k-NVmZIDK2OeQksn9MwDxJ0zY6My3HtN-QrLkIq82Rw979m7Kv8GviCceTC1iqxiaLmVQ-yy9dBTM0NzxgWp1giM7fvtVO07ndgbT4u37nEbxoNNsp66USiVBpHVE_1YYAZSe3tJGR5XBFRSugJ8DiEP8cshzqKhNcgnoFcOtd122A-aVSRU81Dw","e":"AQAB"}]}"#;
    const GOOD: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImsxIn0.eyJpc3MiOiJodHRwczovL2FwaS5ib3RmcmFtZXdvcmsuY29tIiwiYXVkIjoiYXBwLWlkIiwiZXhwIjoxNzAwMDAxMDAwLCJuYmYiOjE2OTk5OTkwMDAsInNlcnZpY2V1cmwiOiJodHRwczovL3NtYmEudHJhZmZpY21hbmFnZXIubmV0L2VtZWEvIn0.TyQZ7ziMq3NKWRpzTZRyiFCHZUN90pHqbjH1jlawqc5gmzaCxPWBOshoJ_T1wqtjmZzoChal4QDU-uIBJ8UElgbWg_eJp7wB6yG29exZL4ZJx_lnKxjzYR2z8cxv9ejqGx9GtlWH61xCZG8FuPzGmgY5TT38WcWVhJfWsOXPpzqmsJSd5cGL3PnP6mT7UF8SdGCxvnq_UrLua684-D85xfogCLo3wlNXm1-H5LI0BTSUz4QVT-oc7Y4no8G-SoLGaS67hq_KEBFjAfPrwKDuUrHZ5PeSV6qNYws0L3-7b0VvgJh_M048yRkSQgRCAJ4L7W4sHsCw9u6aR8Ty8OsS4A";
    const BAD_AUD: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImsxIn0.eyJpc3MiOiJodHRwczovL2FwaS5ib3RmcmFtZXdvcmsuY29tIiwiYXVkIjoib3RoZXItYXBwIiwiZXhwIjoxNzAwMDAxMDAwLCJuYmYiOjE2OTk5OTkwMDAsInNlcnZpY2V1cmwiOiJodHRwczovL3NtYmEudHJhZmZpY21hbmFnZXIubmV0L2VtZWEvIn0.Drf0iK8MTdzmhLR6CD511soS6NegvdWFY07gJUayf5J0xwEK7FiqasMotdLrauQpcKigG1fJKB4qc1KEGy2b4_mSp3aw3JGOB04FR2_x8JWKdPh3q2DmidsCPjP7jyHcce39K4i7eyIsfS553Y3wkfLL6VHDVVGWKM0SJJuQMMuiHu4BinIq7wV8uk7FAasxXm_HMl_7F_HSxWejpyiBDLchVr0Vga63iTIAZjxd6Hm_XkqN58DVtdDZGm5x8jJvSsXW0OSYcFA4b_cwnbDRiWqniPzVRK8pI9PYghp0vXmBwJeHvz35dSICcw8UIpUW5_EzslSeX_vp2xm6RRh6Rg";
    const NO_URL: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImsxIn0.eyJpc3MiOiJodHRwczovL2FwaS5ib3RmcmFtZXdvcmsuY29tIiwiYXVkIjoiYXBwLWlkIiwiZXhwIjoxNzAwMDAxMDAwLCJuYmYiOjE2OTk5OTkwMDB9.Bb7lGudZVwCv0_3iY50MnYZo6PxnLuL8fViZGnYbMebRoJwMMuTG6o4BjW_nHTVW7MH8b6uR7swZBPpmP6fNg6ShI1ifga5H59NxhsxtfaeHl_CEh6NiktUYDB2K5yeyLcTw25olXpg5NG-H_K6E-gXKuZmlWLJruvqnaIUA5wvrbu-GjhtdkJp_N8TyerXi3MbfVYujya0E_WHWR3nOXJs9GVOKl7KQv8xphitnu4W6KU51jgnAsu0g0g0tc83lwvMT10SJ7LGNZafBHiQEg5NXKJldW29DK_eQV-jXNtI2uXMZ4ORjlcVVDVPz_AXZSc4RyO80v4j0_efp6zvhzg";
    const NOW: u64 = 1_700_000_000;
    const SERVICE: &str = "https://smba.trafficmanager.net/emea";

    fn teams() -> Teams {
        Teams::new("app-id", "secret", JwkSet::parse(JWKS).expect("jwks"))
    }

    fn bearer(token: &str) -> [(&'static str, String); 1] {
        [("Authorization", format!("Bearer {token}"))]
    }

    impl Headers for [(&str, String); 1] {
        fn get(&self, name: &str) -> Option<&str> {
            self.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    fn message(service_url: &str, text: &str) -> String {
        format!(
            r#"{{"type":"message","id":"act1","serviceUrl":"{service_url}","channelId":"msteams","text":"{text}","from":{{"id":"29:user","name":"Pat"}},"recipient":{{"id":"28:bot","name":"Bot"}},"conversation":{{"id":"19:chan@thread.tacv2;messageid=42","conversationType":"channel"}}}}"#
        )
    }

    #[test]
    fn a_signed_message_is_accepted_and_threaded_by_conversation() {
        let app = teams();
        let body = message(SERVICE, "<at>Bot</at> what&#39;s   due?");
        let Ok(Received::Message(inbound)) = app.receive(&bearer(GOOD), body.as_bytes(), NOW)
        else {
            panic!("a signed message is a message");
        };
        assert_eq!(inbound.conversation, "19:chan@thread.tacv2;messageid=42");
        assert_eq!(inbound.sender, "29:user");
        assert_eq!(inbound.text, "what&#39;s due?");
        assert_eq!(
            inbound.reply_to.get("serviceUrl").and_then(Value::as_str),
            Some("https://smba.trafficmanager.net/emea/"),
            "the service URL is normalised to a trailing slash"
        );
    }

    #[test]
    fn a_bad_token_a_wrong_audience_a_stale_clock_and_a_foreign_service_are_refused() {
        let app = teams();
        let body = message(SERVICE, "hi");
        let none: [(&str, &str); 0] = [];
        assert!(matches!(
            app.receive(&none, body.as_bytes(), NOW),
            Err(Error::Signature(_))
        ));
        // The payload altered by one character: the signature no longer holds.
        let (head, rest) = GOOD.split_once('.').expect("three parts");
        let (payload, signature) = rest.split_once('.').expect("three parts");
        let mut altered = payload.to_string();
        altered.replace_range(..1, if payload.starts_with('e') { "f" } else { "e" });
        let tampered = format!("{head}.{altered}.{signature}");
        assert!(matches!(
            app.receive(&bearer(&tampered), body.as_bytes(), NOW),
            Err(Error::Signature(_))
        ));
        assert!(matches!(
            app.receive(&bearer(BAD_AUD), body.as_bytes(), NOW),
            Err(Error::Signature(why)) if why.contains("audience")
        ));
        assert!(matches!(
            app.receive(&bearer(GOOD), body.as_bytes(), 1_700_001_000 + LEEWAY_SECS),
            Err(Error::Signature(why)) if why.contains("expired")
        ));
        assert!(matches!(
            app.receive(&bearer(GOOD), body.as_bytes(), 1_699_999_000 - LEEWAY_SECS - 1),
            Err(Error::Signature(why)) if why.contains("not valid before")
        ));
        assert!(matches!(
            app.receive(&bearer(NO_URL), body.as_bytes(), NOW),
            Err(Error::Signature(why)) if why.contains("serviceurl")
        ));
        let elsewhere = message("https://evil.example/", "hi");
        assert!(matches!(
            app.receive(&bearer(GOOD), elsewhere.as_bytes(), NOW),
            Err(Error::Signature(why)) if why.contains("is not the token's")
        ));
        // A token signed by an unknown key.
        let mut other = teams();
        other.set_keys(JwkSet::parse(r#"{"keys":[]}"#).expect("jwks"));
        assert!(matches!(
            other.receive(&bearer(GOOD), body.as_bytes(), NOW),
            Err(Error::Signature(why)) if why.contains("no key")
        ));
    }

    #[test]
    fn other_activities_and_bots_are_ignored() {
        let app = teams();
        let update = r#"{"type":"conversationUpdate","id":"a","serviceUrl":"https://smba.trafficmanager.net/emea/","from":{"id":"x"},"recipient":{"id":"28:bot"},"conversation":{"id":"c"}}"#;
        assert_eq!(
            app.receive(&bearer(GOOD), update.as_bytes(), NOW),
            Ok(Received::Ignored("not a message activity"))
        );
        let bot = message(SERVICE, "me").replace(r#""name":"Pat""#, r#""role":"bot""#);
        assert_eq!(
            app.receive(&bearer(GOOD), bot.as_bytes(), NOW),
            Ok(Received::Ignored("a bot's message"))
        );
    }

    #[test]
    fn the_reply_goes_back_to_the_activity_with_the_fetched_token() {
        let app = teams().with_token_url("https://login.test/token");
        let body = message(SERVICE, "hi");
        let Ok(Received::Message(inbound)) = app.receive(&bearer(GOOD), body.as_bytes(), NOW)
        else {
            panic!("a signed message is a message");
        };

        let Credential::Fetch(fetch) = app.credential(NOW) else {
            panic!("no token yet: fetch one");
        };
        assert_eq!(fetch.url, "https://login.test/token");
        let form = String::from_utf8(fetch.body).expect("utf8");
        assert!(
            form.contains("grant_type=client_credentials") && form.contains("client_id=app-id")
        );
        assert!(form.contains("client_secret=secret"));
        app.accept_credential(
            br#"{"token_type":"Bearer","expires_in":3600,"access_token":"tok"}"#,
            NOW,
        )
        .expect("a token response");
        assert_eq!(app.credential(NOW), Credential::Ready);
        assert!(
            matches!(app.credential(NOW + 3600), Credential::Fetch(_)),
            "fetched again near expiry"
        );

        let out = app.reply(&inbound, "Hello");
        assert_eq!(
            out.url,
            "https://smba.trafficmanager.net/emea/v3/conversations/19%3Achan%40thread.tacv2%3Bmessageid%3D42/activities/act1"
        );
        assert_eq!(out.headers[0].1, "Bearer tok");
        let posted = Value::parse(std::str::from_utf8(&out.body).expect("utf8")).expect("json");
        assert_eq!(posted.get("text").and_then(Value::as_str), Some("Hello"));
        assert_eq!(
            posted.get("replyToId").and_then(Value::as_str),
            Some("act1")
        );
        assert_eq!(
            posted
                .get("from")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str),
            Some("28:bot")
        );
        assert_eq!(
            posted
                .get("recipient")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str),
            Some("29:user")
        );

        assert!(matches!(
            app.accept_credential(b"{}", NOW),
            Err(Error::Payload(_))
        ));
    }

    #[test]
    fn mentions_are_stripped() {
        assert_eq!(clean_text("<at>Bot</at> hello <at>Someone</at> "), "hello");
        assert_eq!(clean_text("<at>Bot"), "<at>Bot");
    }
}
