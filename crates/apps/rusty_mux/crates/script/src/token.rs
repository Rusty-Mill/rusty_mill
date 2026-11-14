//! Access tokens for the TLS listener (#506, D237).
//!
//! A token is 32 random bytes, shown once as 64 hex digits when it is made.
//! The server keeps only its SHA-256, in a token file of one `NAME HASH` per
//! line (`#` starts a comment). A connection that presents a token is the
//! principal `token:NAME`, which the shared-mode allowlist names like any
//! other principal (one access table, not two). The file lives here, with
//! the other files ctl and the server both read.
//!
//! `create`, `rotate` and `revoke` are pure edits of the file's text; the
//! caller reads and writes the file (checked as the allowlist is).

use std::collections::BTreeMap;

use rusty_crypto_key::constant_time_eq;
use rusty_sha2::{Hash, Sha256};

/// A token's length in bytes.
pub const LEN: usize = 32;
/// The longest token name.
const MAX_NAME: usize = 64;

/// The token names and hashes a token file holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tokens(BTreeMap<String, [u8; 32]>);

/// A token's SHA-256.
fn hash(token: &[u8]) -> [u8; 32] {
    let mut sha = Sha256::new();
    sha.update(token);
    sha.finalize()
}

/// Whether `name` is a usable token name: letters, digits, `.`, `_` and `-`.
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if ok {
        return Ok(());
    }
    Err(format!(
        "{name:?} is not a token name (1 to {MAX_NAME} of letters, digits, `.`, `_`, `-`)"
    ))
}

impl Tokens {
    /// The tokens `text` lists. A bad line, a bad name, a bad hash or a
    /// repeated name refuses the whole file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut all = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let at = |why: String| format!("line {}: {why}", n + 1);
            let (name, digest) = line
                .split_once(char::is_whitespace)
                .ok_or_else(|| at("expected `NAME HASH`".into()))?;
            check_name(name).map_err(&at)?;
            let digest = rusty_hex::decode_array::<32>(digest.trim())
                .map_err(|_| at(format!("the hash of {name} is not 64 hex digits")))?;
            if all.insert(name.to_string(), digest).is_some() {
                return Err(at(format!("{name} is listed already")));
            }
        }
        Ok(Self(all))
    }

    /// The file's text.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .fold(String::new(), |mut out, (name, digest)| {
                out.push_str(&format!("{name} {}\n", rusty_hex::encode(digest)));
                out
            })
    }

    /// The name of the token `presented` is, if it is one. Every entry is
    /// compared in constant time, so the answer's timing says nothing of
    /// which names exist.
    pub fn verify(&self, presented: &[u8]) -> Option<&str> {
        let digest = hash(presented);
        let mut found = None;
        for (name, kept) in &self.0 {
            if constant_time_eq(kept, &digest) {
                found = Some(name.as_str());
            }
        }
        found
    }

    /// Whether `name` has a token.
    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The names whose token differs in `fresh`: changed or gone. A reload
    /// closes the connections of exactly these.
    pub fn changed<'a>(&'a self, fresh: &Tokens) -> Vec<&'a str> {
        self.0
            .iter()
            .filter(|(name, digest)| fresh.0.get(*name) != Some(*digest))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Adds the token `name`, made of `bytes`; a name in use is refused
    /// (use [`Tokens::rotate`]).
    pub fn create(&mut self, name: &str, bytes: &[u8; LEN]) -> Result<(), String> {
        check_name(name)?;
        if self.has(name) {
            return Err(format!("token {name} exists; use `token rotate {name}`"));
        }
        self.0.insert(name.to_string(), hash(bytes));
        Ok(())
    }

    /// Replaces the token `name`, which must exist.
    pub fn rotate(&mut self, name: &str, bytes: &[u8; LEN]) -> Result<(), String> {
        let slot = self
            .0
            .get_mut(name)
            .ok_or_else(|| format!("no token {name}"))?;
        *slot = hash(bytes);
        Ok(())
    }

    /// Removes the token `name`, which must exist.
    pub fn revoke(&mut self, name: &str) -> Result<(), String> {
        self.0
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| format!("no token {name}"))
    }
}

/// A fresh random token, and its 64 hex digits for the user to keep.
pub fn generate() -> Result<([u8; LEN], String), String> {
    let mut bytes = [0u8; LEN];
    rusty_rand::fill(&mut bytes).map_err(|e| format!("no random bytes: {e}"))?;
    let shown = rusty_hex::encode(&bytes);
    Ok((bytes, shown))
}

/// The bytes of a token its holder typed as hex, or `None` if it is not
/// exactly [`LEN`] bytes of hex.
pub fn from_hex(text: &str) -> Option<[u8; LEN]> {
    rusty_hex::decode_array::<LEN>(text.trim()).ok()
}

#[cfg(test)]
mod tests;
