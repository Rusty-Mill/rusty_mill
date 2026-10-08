//! Authenticated `requestState`. The client echoes it back untrusted, so it
//! carries an HMAC-SHA256 over a domain tag, a scope (method and name, so a
//! state is good for the request that issued it and no other) and an expiry.
//!
//! Token: `base64url(expiry_secs_be8 || payload) "." base64url(mac)`.

use rusty_base64::{decode_url_safe, encode_url_safe_no_pad};
use rusty_crypto_key::constant_time_eq;
use rusty_oauth::crypto::hmac::hmac_sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DOMAIN: &[u8] = b"rusty_mcp_server/request-state/v1";

/// The shortest key accepted.
pub const MIN_KEY_BYTES: usize = 32;

/// Why a `requestState` was refused. Deliberately not telling which check
/// failed, beyond expiry, which is the client's cue to start over.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StateError {
    Invalid,
    Expired,
}

pub(crate) struct StateCodec {
    key: Vec<u8>,
    ttl: Duration,
}

fn mac(key: &[u8], scope: &str, body: &[u8]) -> [u8; 32] {
    let mut msg = Vec::with_capacity(DOMAIN.len() + scope.len() + body.len() + 2);
    msg.extend_from_slice(DOMAIN);
    msg.push(0);
    msg.extend_from_slice(scope.as_bytes());
    msg.push(0);
    msg.extend_from_slice(body);
    hmac_sha256(key, &msg)
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl StateCodec {
    pub(crate) fn new(key: Vec<u8>, ttl: Duration) -> Self {
        Self { key, ttl }
    }

    pub(crate) fn seal(&self, scope: &str, payload: &[u8]) -> String {
        self.seal_at(now_secs(), scope, payload)
    }

    pub(crate) fn open(&self, scope: &str, token: &str) -> Result<Vec<u8>, StateError> {
        self.open_at(now_secs(), scope, token)
    }

    fn seal_at(&self, now: u64, scope: &str, payload: &[u8]) -> String {
        let expiry = now.saturating_add(self.ttl.as_secs());
        let mut body = expiry.to_be_bytes().to_vec();
        body.extend_from_slice(payload);
        let tag = mac(&self.key, scope, &body);
        format!(
            "{}.{}",
            encode_url_safe_no_pad(&body),
            encode_url_safe_no_pad(&tag)
        )
    }

    fn open_at(&self, now: u64, scope: &str, token: &str) -> Result<Vec<u8>, StateError> {
        let (body, tag) = token.split_once('.').ok_or(StateError::Invalid)?;
        let body = decode_url_safe(body).map_err(|_| StateError::Invalid)?;
        let tag = decode_url_safe(tag).map_err(|_| StateError::Invalid)?;
        if !constant_time_eq(&tag, &mac(&self.key, scope, &body)) {
            return Err(StateError::Invalid);
        }
        let (expiry, payload) = body.split_first_chunk::<8>().ok_or(StateError::Invalid)?;
        if now > u64::from_be_bytes(*expiry) {
            return Err(StateError::Expired);
        }
        Ok(payload.to_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn codec() -> StateCodec {
        StateCodec::new(vec![7; 32], Duration::from_secs(60))
    }

    #[test]
    fn a_sealed_state_opens_for_its_scope() {
        let c = codec();
        let token = c.seal_at(1000, "tools/call\0ask", b"step 2");
        assert_eq!(
            c.open_at(1010, "tools/call\0ask", &token).unwrap(),
            b"step 2"
        );
        assert_eq!(
            c.open_at(1060, "tools/call\0ask", &token).unwrap(),
            b"step 2"
        );
    }

    #[test]
    fn an_empty_payload_round_trips() {
        let c = codec();
        let token = c.seal_at(0, "s", b"");
        assert_eq!(c.open_at(0, "s", &token).unwrap(), b"");
    }

    #[test]
    fn it_expires() {
        let c = codec();
        let token = c.seal_at(1000, "s", b"x");
        assert_eq!(c.open_at(1061, "s", &token), Err(StateError::Expired));
    }

    #[test]
    fn another_scope_or_key_is_refused() {
        let c = codec();
        let token = c.seal_at(1000, "tools/call\0a", b"x");
        assert_eq!(
            c.open_at(1000, "tools/call\0b", &token),
            Err(StateError::Invalid)
        );
        let other = StateCodec::new(vec![8; 32], Duration::from_secs(60));
        assert_eq!(
            other.open_at(1000, "tools/call\0a", &token),
            Err(StateError::Invalid)
        );
    }

    #[test]
    fn tampering_is_refused() {
        let c = codec();
        let token = c.seal_at(1000, "s", b"price=1");
        let (body, tag) = token.split_once('.').unwrap();
        // Flip a payload byte, keep the old tag.
        let mut raw = decode_url_safe(body).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        let forged = format!("{}.{tag}", encode_url_safe_no_pad(&raw));
        assert_eq!(c.open_at(1000, "s", &forged), Err(StateError::Invalid));
        // Extend the expiry, keep the old tag.
        let mut raw = decode_url_safe(body).unwrap();
        raw[7] = raw[7].wrapping_add(1);
        let forged = format!("{}.{tag}", encode_url_safe_no_pad(&raw));
        assert_eq!(c.open_at(1000, "s", &forged), Err(StateError::Invalid));
    }

    #[test]
    fn garbage_is_refused() {
        let c = codec();
        for junk in ["", "x", "a.b", "...", "!!.!!", "AAAA."] {
            assert_eq!(
                c.open_at(0, "s", junk),
                Err(StateError::Invalid),
                "{junk:?}"
            );
        }
    }
}
