//! A restricted TOML reader (pack D24): the subset the tool catalog needs, no
//! more. Only `[[name]]` array-of-tables headers and `key = "string"` pairs
//! (basic strings with `\\` and `\"` escapes, or literal `'...'` strings),
//! `#` comments and blank lines. Everything else is an error with its line
//! number, so a catalog never half-parses. Sits behind [`TomlReader`] so the
//! `toml` crate can replace it if Nano approves one (D12).

use std::collections::BTreeMap;
use std::fmt;

/// One `[[name]]` table and its string fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    /// Line of the `[[name]]` header, 1-based.
    pub line: usize,
    pub fields: BTreeMap<String, String>,
}

/// A line the reader refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub msg: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

impl std::error::Error for ParseError {}

/// Reads text into tables. The one seam between the catalog and its parser.
pub trait TomlReader {
    fn read(&self, text: &str) -> Result<Vec<Table>, ParseError>;
}

/// The built-in, zero-dependency reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct Restricted;

fn is_bare(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parses a quoted string at the start of `s`; returns it and the rest.
fn string(s: &str, line: usize) -> Result<(String, &str), ParseError> {
    let err = |msg| ParseError { line, msg };
    let mut chars = s.char_indices();
    let quote = match chars.next() {
        Some((_, q @ ('"' | '\''))) => q,
        _ => return Err(err("value must be a quoted string")),
    };
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            c if c == quote => return Ok((out, &s[i + 1..])),
            '\\' if quote == '"' => match chars.next() {
                Some((_, e @ ('\\' | '"'))) => out.push(e),
                _ => return Err(err("only \\\\ and \\\" escapes are supported")),
            },
            c => out.push(c),
        }
    }
    Err(err("unterminated string"))
}

/// True if `rest` is empty or only a comment.
fn only_comment(rest: &str) -> bool {
    let rest = rest.trim();
    rest.is_empty() || rest.starts_with('#')
}

impl TomlReader for Restricted {
    fn read(&self, text: &str) -> Result<Vec<Table>, ParseError> {
        let mut tables: Vec<Table> = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            let line = idx + 1;
            let err = |msg| ParseError { line, msg };
            let s = raw.trim();
            if s.is_empty() || s.starts_with('#') {
                continue;
            }
            if let Some(head) = s.strip_prefix("[[") {
                let (name, rest) = head.split_once("]]").ok_or(err("unclosed [[header]]"))?;
                let name = name.trim();
                if !is_bare(name) {
                    return Err(err("table name must be [A-Za-z0-9_-]"));
                }
                if !only_comment(rest) {
                    return Err(err("unexpected text after the header"));
                }
                tables.push(Table {
                    name: name.to_owned(),
                    line,
                    fields: BTreeMap::new(),
                });
                continue;
            }
            if s.starts_with('[') {
                return Err(err("only [[array]] tables are supported"));
            }
            let (key, value) = s.split_once('=').ok_or(err("expected key = \"value\""))?;
            let key = key.trim();
            if !is_bare(key) {
                return Err(err("key must be [A-Za-z0-9_-]"));
            }
            let (value, rest) = string(value.trim_start(), line)?;
            if !only_comment(rest) {
                return Err(err("unexpected text after the value"));
            }
            let table = tables.last_mut().ok_or(err("key before any [[table]]"))?;
            if table.fields.insert(key.to_owned(), value).is_some() {
                return Err(err("duplicate key"));
            }
        }
        Ok(tables)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(t: &str) -> Result<Vec<Table>, ParseError> {
        Restricted.read(t)
    }

    #[test]
    fn reads_tables_strings_and_comments() {
        let t = read(
            "# c\n[[tool]] # x\nid = \"a\" # y\npath = 'C:\\x'\n\n[[tool]]\nid = \"b\\\"q\"\n",
        )
        .unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].fields["id"], "a");
        assert_eq!(t[0].fields["path"], "C:\\x");
        assert_eq!(t[1].fields["id"], "b\"q");
        assert_eq!(t[1].line, 6);
    }

    #[test]
    fn empty_and_comment_only_text_is_no_tables() {
        assert!(read("").unwrap().is_empty());
        assert!(read("# nothing\n\n").unwrap().is_empty());
    }

    #[test]
    fn refuses_everything_outside_the_subset() {
        for (text, line) in [
            ("id = \"a\"", 1),                  // key before a table
            ("[[t]]\nn = 3", 2),                // not a string
            ("[[t]]\na = [\"x\"]", 2),          // array value
            ("[t]", 1),                         // plain table
            ("[[t]]\na = \"x", 2),              // unterminated
            ("[[t]]\na = \"x\" b", 2),          // trailing junk
            ("[[t]]\na = \"\\n\"", 2),          // unsupported escape
            ("[[t]]\na = \"1\"\na = \"2\"", 3), // duplicate key
            ("[[t]]\na.b = \"1\"", 2),          // dotted key
            ("[[t] ]", 1),                      // bad header
            ("[[t]] x", 1),                     // junk after header
            ("[[t]]\nnonsense", 2),             // no equals
        ] {
            assert_eq!(read(text).unwrap_err().line, line, "{text:?}");
        }
    }
}
