//! Signed upload URLs: a client asks for a URL (authenticated), then `PUT`s the replay to
//! it without credentials. The URL carries the account, the size and SHA-256 the client
//! declared, and an expiry, all signed with a per-process key, and it works once. The server
//! checks the bytes against the declaration before analysing anything, so a truncated or
//! swapped upload is rejected rather than analysed.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::sha256::{ct_eq, hex, hmac, sha256};

/// How long an issued URL may be used (s).
pub const TTL_S: u64 = 10 * 60;

/// What a signed URL authorises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upload {
    pub account: String,
    pub size: usize,
    /// Lower-case hex SHA-256 of the replay bytes.
    pub sha256: String,
    pub name: String,
    /// The analysis options to apply once the bytes are verified.
    pub rank: Option<String>,
    pub session: Option<String>,
    pub inline: bool,
    pub team: Option<String>,
    /// Unix seconds after which the URL is dead.
    pub expires: u64,
    pub nonce: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Malformed, or the signature does not match.
    Invalid,
    Expired,
    /// The URL was already used.
    Used,
    /// The body's length differs from the declared size.
    SizeMismatch,
    /// The body's SHA-256 differs from the declared one.
    HashMismatch,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid upload URL",
            Self::Expired => "the upload URL has expired",
            Self::Used => "the upload URL was already used",
            Self::SizeMismatch => "the upload is not the declared size",
            Self::HashMismatch => "the upload does not match the declared SHA-256",
        })
    }
}

/// Issues and redeems upload URLs.
pub struct Signer {
    key: [u8; 32],
    /// Redeemed nonces with their expiry; entries are dropped once expired (the URL is dead anyway).
    used: Mutex<HashMap<u64, u64>>,
}

impl Signer {
    /// A signer with a fresh random key from the OS. URLs die with the process.
    pub fn new() -> std::io::Result<Self> {
        let mut key = [0u8; 32];
        rusty_rand::fill(&mut key)?;
        Ok(Self::with_key(key))
    }

    pub fn with_key(key: [u8; 32]) -> Self {
        Self {
            key,
            used: Mutex::default(),
        }
    }

    /// The URL token for `upload`: `hex(json).hex(hmac)`.
    pub fn sign(&self, upload: &Upload) -> String {
        let body = serde_json::to_vec(upload).unwrap_or_default();
        format!("{}.{}", hex(&body), hex(&hmac(&self.key, &body)))
    }

    /// Verify `token` and redeem it against `body` at time `now`: the authorisation to
    /// analyse `body`, or why not. A token is spent as soon as its signature and expiry check out.
    pub fn redeem(&self, token: &str, body: &[u8], now: u64) -> Result<Upload, Refusal> {
        let (payload, mac) = token.split_once('.').ok_or(Refusal::Invalid)?;
        let (payload, mac) = (
            unhex(payload).ok_or(Refusal::Invalid)?,
            unhex(mac).ok_or(Refusal::Invalid)?,
        );
        if !ct_eq(&hmac(&self.key, &payload), &mac) {
            return Err(Refusal::Invalid);
        }
        let up: Upload = serde_json::from_slice(&payload).map_err(|_| Refusal::Invalid)?;
        if now > up.expires {
            return Err(Refusal::Expired);
        }
        {
            let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
            used.retain(|_, exp| *exp >= now);
            if used.insert(up.nonce, up.expires).is_some() {
                return Err(Refusal::Used);
            }
        }
        if body.len() != up.size {
            return Err(Refusal::SizeMismatch);
        }
        if !ct_eq(hex(&sha256(body)).as_bytes(), up.sha256.as_bytes()) {
            return Err(Refusal::HashMismatch);
        }
        Ok(up)
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len().is_multiple_of(2) && s.is_ascii())
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn upload(body: &[u8], nonce: u64) -> Upload {
        Upload {
            account: "ann".into(),
            size: body.len(),
            sha256: hex(&sha256(body)),
            name: "m".into(),
            rank: None,
            session: None,
            inline: false,
            team: None,
            expires: NOW + TTL_S,
            nonce,
        }
    }

    #[test]
    fn a_signed_url_admits_exactly_the_declared_bytes_once() {
        let s = Signer::with_key([7; 32]);
        let token = s.sign(&upload(b"replay bytes", 1));
        let up = s.redeem(&token, b"replay bytes", NOW).expect("valid");
        assert_eq!((up.account.as_str(), up.size), ("ann", 12));
        assert_eq!(s.redeem(&token, b"replay bytes", NOW), Err(Refusal::Used));
    }

    #[test]
    fn wrong_size_wrong_hash_expiry_and_tampering_are_refused() {
        let s = Signer::with_key([7; 32]);
        let mk = |n| s.sign(&upload(b"replay bytes", n));
        assert_eq!(
            s.redeem(&mk(1), b"replay byte", NOW),
            Err(Refusal::SizeMismatch)
        );
        assert_eq!(
            s.redeem(&mk(2), b"replay bytez", NOW),
            Err(Refusal::HashMismatch)
        );
        assert_eq!(
            s.redeem(&mk(3), b"replay bytes", NOW + TTL_S + 1),
            Err(Refusal::Expired)
        );
        // Flip a payload byte: the signature no longer matches.
        let token = mk(4);
        let forged = format!(
            "{}{}",
            if token.starts_with('7') { "8" } else { "7" },
            &token[1..]
        );
        assert_eq!(
            s.redeem(&forged, b"replay bytes", NOW),
            Err(Refusal::Invalid)
        );
        // Another process's key cannot mint or accept it.
        assert_eq!(
            Signer::with_key([9; 32]).redeem(&mk(5), b"replay bytes", NOW),
            Err(Refusal::Invalid)
        );
        for junk in ["", ".", "zz.zz", "abc"] {
            assert_eq!(s.redeem(junk, b"", NOW), Err(Refusal::Invalid), "{junk:?}");
        }
    }

    #[test]
    fn spent_nonces_are_forgotten_once_their_url_has_expired() {
        let s = Signer::with_key([7; 32]);
        s.redeem(&s.sign(&upload(b"a", 1)), b"a", NOW).unwrap();
        // A later, unrelated redeem prunes the expired entry.
        let mut late = upload(b"b", 2);
        late.expires = NOW + 10 * TTL_S;
        s.redeem(&s.sign(&late), b"b", NOW + 2 * TTL_S).unwrap();
        assert_eq!(s.used.lock().unwrap().len(), 1);
    }
}
