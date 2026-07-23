//! A minimal, case-insensitive multi-map for HTTP headers. Order of
//! insertion is preserved (useful for predictable wire output and
//! tests); lookups are case-insensitive per RFC 7230 §3.2.
//!
//! Ported near-verbatim from `rusty_request`'s `header.rs` -- shared by
//! both request and response heads here, not just responses.

use crate::error::{Error, Result};

/// A case-insensitive, order-preserving multi-map of HTTP header names to
/// values.
///
/// Entries marked sensitive (see [`Self::insert_sensitive`]/
/// [`Self::append_sensitive`]) have their value masked in [`Debug`](std::fmt::Debug)
/// output -- the sensitivity flag has no other effect (`get`/`iter`/etc.
/// still return the real value; it's `{:?}` that's masked, so an
/// `Authorization` or `Cookie` value doesn't leak in full into a debug log
/// or panic message).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HeaderMap {
    entries: Vec<(String, String, bool)>,
}

impl HeaderMap {
    /// An empty header map.
    pub fn new() -> Self {
        HeaderMap::default()
    }

    /// Sets `name` to `value`, replacing any existing entries with the
    /// same name (case-insensitive).
    pub fn insert(&mut self, name: &str, value: &str) -> Result<()> {
        self.insert_impl(name, value, false)
    }

    /// Like [`Self::insert`], but marks the value sensitive -- masked in
    /// `Debug` output. Use for credential-bearing headers (`Authorization`,
    /// `Cookie`, ...).
    pub fn insert_sensitive(&mut self, name: &str, value: &str) -> Result<()> {
        self.insert_impl(name, value, true)
    }

    fn insert_impl(&mut self, name: &str, value: &str, sensitive: bool) -> Result<()> {
        validate_name(name)?;
        validate_value(value)?;
        self.entries
            .retain(|(k, _, _)| !k.eq_ignore_ascii_case(name));
        self.entries
            .push((name.to_string(), value.to_string(), sensitive));
        Ok(())
    }

    /// Adds `name: value` without removing any existing entries for the
    /// same name -- for headers that legitimately repeat.
    pub fn append(&mut self, name: &str, value: &str) -> Result<()> {
        self.append_impl(name, value, false)
    }

    /// Like [`Self::append`], but marks the value sensitive -- masked in
    /// `Debug` output. Use for credential-bearing headers (`Authorization`,
    /// `Cookie`, ...).
    pub fn append_sensitive(&mut self, name: &str, value: &str) -> Result<()> {
        self.append_impl(name, value, true)
    }

    fn append_impl(&mut self, name: &str, value: &str, sensitive: bool) -> Result<()> {
        validate_name(name)?;
        validate_value(value)?;
        self.entries
            .push((name.to_string(), value.to_string(), sensitive));
        Ok(())
    }

    /// The first value for `name`, case-insensitive.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v, _)| v.as_str())
    }

    /// Whether any entry matches `name`, case-insensitive.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Every value for `name`, case-insensitive, in insertion order --
    /// for headers that legitimately repeat (e.g. `Set-Cookie`), where
    /// [`Self::get`] only ever returns the first.
    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.entries
            .iter()
            .filter(move |(k, _, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v, _)| v.as_str())
    }

    /// Removes every entry for `name` (case-insensitive), returning the
    /// first removed value, if any.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        let mut removed = None;
        self.entries.retain(|(k, v, _)| {
            if k.eq_ignore_ascii_case(name) {
                if removed.is_none() {
                    removed = Some(v.clone());
                }
                false
            } else {
                true
            }
        });
        removed
    }

    /// Iterates entries in insertion order, `(name, value)`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(k, v, _)| (k.as_str(), v.as_str()))
    }

    /// The number of entries (repeated names each count separately).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Prints as the bare word `Sensitive`, masking a redacted header value in
/// `Debug` output (mirroring the `http` crate's `HeaderValue` masking).
struct Redacted;

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sensitive")
    }
}

impl std::fmt::Debug for HeaderMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (k, v, sensitive) in &self.entries {
            if *sensitive {
                map.entry(k, &Redacted);
            } else {
                map.entry(k, v);
            }
        }
        map.finish()
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b':' && b != b'\r' && b != b'\n')
    {
        return Err(Error::InvalidHeader(format!(
            "invalid header name `{name}`"
        )));
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<()> {
    // Reject bare CR/LF -- allowing them would let a caller smuggle
    // extra headers or a second request into the stream.
    if value.bytes().any(|b| b == b'\r' || b == b'\n') {
        return Err(Error::InvalidHeader(format!(
            "header value must not contain CR/LF: `{value}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_is_case_insensitive_and_replaces() {
        let mut h = HeaderMap::new();
        h.insert("Content-Type", "text/plain").unwrap();
        h.insert("content-type", "application/json").unwrap();
        assert_eq!(h.get("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn append_keeps_both() {
        let mut h = HeaderMap::new();
        h.append("X-A", "1").unwrap();
        h.append("X-A", "2").unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn rejects_crlf_injection_in_value() {
        let mut h = HeaderMap::new();
        assert!(h.insert("X-A", "evil\r\nX-Injected: yes").is_err());
    }

    #[test]
    fn remove_drops_all_case_insensitive_matches() {
        let mut h = HeaderMap::new();
        h.append("X-A", "1").unwrap();
        h.append("x-a", "2").unwrap();
        h.append("X-B", "3").unwrap();
        assert_eq!(h.remove("x-A"), Some("1".to_string()));
        assert!(!h.contains("x-a"));
        assert_eq!(h.len(), 1);
        assert_eq!(h.remove("missing"), None);
    }

    #[test]
    fn rejects_colon_in_name() {
        let mut h = HeaderMap::new();
        assert!(h.insert("X-A:", "1").is_err());
    }

    #[test]
    fn sensitive_values_are_still_readable_normally() {
        let mut h = HeaderMap::new();
        h.insert_sensitive("Authorization", "Bearer secret")
            .unwrap();
        assert_eq!(h.get("authorization"), Some("Bearer secret"));
    }

    #[test]
    fn sensitive_values_are_masked_in_debug_output() {
        let mut h = HeaderMap::new();
        h.insert_sensitive("Authorization", "Bearer secret")
            .unwrap();
        h.insert("Content-Type", "text/plain").unwrap();
        let debug = format!("{h:?}");
        assert!(!debug.contains("secret"));
        assert!(debug.contains("Sensitive"));
        assert!(debug.contains("text/plain"));
    }

    #[test]
    fn append_sensitive_keeps_value_readable_and_masked() {
        let mut h = HeaderMap::new();
        h.append_sensitive("Set-Cookie", "session=abc123").unwrap();
        assert_eq!(h.get("set-cookie"), Some("session=abc123"));
        assert!(!format!("{h:?}").contains("abc123"));
    }

    #[test]
    fn get_all_returns_every_value_case_insensitive() {
        let mut h = HeaderMap::new();
        h.append("Set-Cookie", "a=1").unwrap();
        h.append("set-cookie", "b=2").unwrap();
        h.append("X-Other", "irrelevant").unwrap();
        let values: Vec<&str> = h.get_all("SET-COOKIE").collect();
        assert_eq!(values, vec!["a=1", "b=2"]);
    }

    #[test]
    fn get_all_on_missing_name_is_empty() {
        let h = HeaderMap::new();
        assert_eq!(h.get_all("missing").count(), 0);
    }
}
