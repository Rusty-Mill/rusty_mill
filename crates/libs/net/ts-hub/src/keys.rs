//! The keys file: which pre-auth keys the hub admits, one per line.
//!
//! ```text
//! # KEY LOGIN [reusable] [name=SERVER]
//! k-3f9a alice reusable
//! k-77c1 ci name=build1
//! ```
//!
//! The file owns the keys it lists: a key dropped from it stops working at the
//! next load, also after a restart. To change a listed key's settings, drop it,
//! load, then list it again and load.
//!
//! A key is single-use unless `reusable`. `name=` makes it a tagged key that
//! pins a server name (`KeyPolicy::tagged`). Keys are secrets: errors name a
//! line number, never the line.

use crate::KeyPolicy;

/// A keys-file line that could not be read.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("keys line {line}: {reason}")]
pub struct KeysError {
    pub line: usize,
    pub reason: &'static str,
}

/// `(key, login, policy)` for every line of `text`.
pub fn parse_keys(text: &str) -> Result<Vec<(String, String, KeyPolicy)>, KeysError> {
    let mut out: Vec<(String, String, KeyPolicy)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let fail = |reason| KeysError { line, reason };
        let content = raw.split('#').next().unwrap_or_default();
        let mut words = content.split_whitespace();
        let Some(key) = words.next() else { continue };
        let login = words.next().ok_or_else(|| fail("a key needs a login"))?;
        let mut policy = KeyPolicy::default();
        for word in words {
            match word.split_once('=') {
                None if word == "reusable" => policy.reusable = true,
                Some(("name", name)) if !name.is_empty() => policy.tagged = Some(name.into()),
                _ => return Err(fail("expected `reusable` or `name=SERVER`")),
            }
        }
        if out.iter().any(|(k, ..)| k == key) {
            return Err(fail("the key is listed twice"));
        }
        out.push((key.to_string(), login.to_string(), policy));
    }
    Ok(out)
}
