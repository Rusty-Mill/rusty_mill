//! RFC 6901 JSON Pointer: parsed once into unescaped reference tokens.

use crate::{Error, Result};
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt;
use rusty_json::Value;

/// A parsed JSON Pointer. The empty pointer names the whole document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pointer {
    tokens: Vec<String>,
}

/// Where a pointer lands when resolved against a container, for writes.
pub(crate) enum Slot<'a> {
    /// A key in an object, present or not.
    Key(&'a mut rusty_json::Map, String),
    /// An index in an array, `len` meaning append (`-` or `len`).
    Index(&'a mut Vec<Value>, usize),
}

impl Pointer {
    /// Parses RFC 6901 text. `""` is the root; otherwise it must start
    /// with `/`, and `~` may only appear as `~0` or `~1`.
    pub fn parse(text: &str) -> Result<Self> {
        if text.is_empty() {
            return Ok(Self::default());
        }
        let Some(rest) = text.strip_prefix('/') else {
            return Err(Error::InvalidPointer(text.into()));
        };
        let tokens = rest
            .split('/')
            .map(|token| unescape(token).ok_or_else(|| Error::InvalidPointer(text.into())))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { tokens })
    }

    /// A pointer from already-unescaped tokens.
    pub fn from_tokens(tokens: Vec<String>) -> Self {
        Self { tokens }
    }

    /// The unescaped reference tokens.
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    /// True for the root pointer.
    pub fn is_root(&self) -> bool {
        self.tokens.is_empty()
    }

    /// This pointer extended by one more token.
    pub fn child(&self, token: impl Into<String>) -> Self {
        let mut tokens = self.tokens.clone();
        tokens.push(token.into());
        Self { tokens }
    }

    /// True when `self` is a proper prefix of `other`: `other` lies
    /// strictly inside the value `self` names.
    pub fn is_proper_prefix_of(&self, other: &Pointer) -> bool {
        self.tokens.len() < other.tokens.len() && other.tokens.starts_with(&self.tokens)
    }

    /// The value this pointer names in `doc`, if any.
    pub fn resolve<'a>(&self, doc: &'a Value) -> Option<&'a Value> {
        self.tokens
            .iter()
            .try_fold(doc, |cur, token| step(cur, token))
    }

    /// Mutable counterpart to [`Pointer::resolve`].
    pub fn resolve_mut<'a>(&self, doc: &'a mut Value) -> Option<&'a mut Value> {
        self.tokens
            .iter()
            .try_fold(doc, |cur, token| step_mut(cur, token))
    }

    /// Resolves the parent container and the final token as a write
    /// slot. The root has no parent and is handled by callers.
    pub(crate) fn slot<'a>(&self, doc: &'a mut Value, allow_append: bool) -> Result<Slot<'a>> {
        let Some((last, parents)) = self.tokens.split_last() else {
            return Err(Error::PathNotFound(self.to_string()));
        };
        let path = self.to_string();
        let parent = parents
            .iter()
            .try_fold(doc, |cur, token| step_mut(cur, token))
            .ok_or_else(|| Error::PathNotFound(path.clone()))?;
        match parent {
            Value::Object(map) => Ok(Slot::Key(map, last.clone())),
            Value::Array(items) => {
                let index = if last == "-" && allow_append {
                    items.len()
                } else {
                    let limit = if allow_append {
                        items.len()
                    } else {
                        items.len().saturating_sub(1)
                    };
                    let index =
                        parse_index(last).ok_or_else(|| Error::InvalidIndex(path.clone()))?;
                    if index > limit || (items.is_empty() && !allow_append) {
                        return Err(Error::InvalidIndex(path));
                    }
                    index
                };
                Ok(Slot::Index(items, index))
            }
            _ => Err(Error::PathNotFound(path)),
        }
    }
}

fn step<'a>(cur: &'a Value, token: &str) -> Option<&'a Value> {
    match cur {
        Value::Object(map) => map.get(token),
        Value::Array(items) => items.get(parse_index(token)?),
        _ => None,
    }
}

fn step_mut<'a>(cur: &'a mut Value, token: &str) -> Option<&'a mut Value> {
    match cur {
        Value::Object(map) => map.get_mut(token),
        Value::Array(items) => items.get_mut(parse_index(token)?),
        _ => None,
    }
}

/// RFC 6901 §4: digits only, no leading zero unless the index is `0`.
fn parse_index(token: &str) -> Option<usize> {
    if token.is_empty() || (token.len() > 1 && token.starts_with('0')) {
        return None;
    }
    if !token.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// `~1` → `/`, `~0` → `~`; any other `~` is an error.
fn unescape(token: &str) -> Option<String> {
    if !token.contains('~') {
        return Some(token.into());
    }
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            _ => return None,
        }
    }
    Some(out)
}

fn escape_into(token: &str, out: &mut String) {
    for c in token.chars() {
        match c {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            c => out.push(c),
        }
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        for token in &self.tokens {
            text.push('/');
            escape_into(token, &mut text);
        }
        f.write_str(&text)
    }
}

impl core::str::FromStr for Pointer {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_json::json;

    /// RFC 6901 §5's example document and the pointers it lists.
    #[test]
    fn rfc_6901_examples_resolve() {
        let doc = json!({
            "foo": ["bar", "baz"],
            "": 0,
            "a/b": 1,
            "c%d": 2,
            "e^f": 3,
            "g|h": 4,
            "i\\j": 5,
            "k\"l": 6,
            " ": 7,
            "m~n": 8
        });
        let cases: &[(&str, Value)] = &[
            ("", doc.clone()),
            ("/foo", json!(["bar", "baz"])),
            ("/foo/0", json!("bar")),
            ("/", json!(0)),
            ("/a~1b", json!(1)),
            ("/c%d", json!(2)),
            ("/e^f", json!(3)),
            ("/g|h", json!(4)),
            ("/i\\j", json!(5)),
            ("/k\"l", json!(6)),
            ("/ ", json!(7)),
            ("/m~0n", json!(8)),
        ];
        for (text, expected) in cases {
            let pointer = Pointer::parse(text).unwrap();
            assert_eq!(pointer.resolve(&doc), Some(expected), "{text}");
            assert_eq!(pointer.to_string(), *text, "round trip");
        }
    }

    #[test]
    fn rejects_malformed_pointers() {
        for bad in ["foo", "/a~", "/a~2", "~0"] {
            assert!(
                matches!(Pointer::parse(bad), Err(Error::InvalidPointer(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn index_rules_follow_the_rfc() {
        assert_eq!(parse_index("0"), Some(0));
        assert_eq!(parse_index("10"), Some(10));
        assert_eq!(parse_index("01"), None);
        assert_eq!(parse_index(""), None);
        assert_eq!(parse_index("-"), None);
        assert_eq!(parse_index("1a"), None);
    }

    #[test]
    fn prefix_and_child() {
        let a = Pointer::parse("/a").unwrap();
        let ab = a.child("b");
        assert!(a.is_proper_prefix_of(&ab));
        assert!(!ab.is_proper_prefix_of(&a));
        assert!(!a.is_proper_prefix_of(&a));
        assert_eq!(ab.to_string(), "/a/b");
    }
}
