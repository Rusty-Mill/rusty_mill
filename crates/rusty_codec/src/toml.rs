//! A real (subset, honestly-documented) TOML parser.
//!
//! Implemented: comments, bare/dotted keys, tables (`[section]`, including
//! nested `[a.b.c]`), basic strings (`"..."` with `\n`/`\t`/`\\`/`\"`
//! escapes) and literal strings (`'...'`, no escape processing), integers
//! and floats (with `_` digit separators), booleans, arrays (including
//! ones spanning multiple physical lines), and inline tables (`{ a = 1,
//! b = 2 }`).
//!
//! Known, deliberate gaps: no arrays-of-tables (`[[section]]`), no
//! multi-line basic/literal strings (`"""..."""`/`'''...'''`), no
//! quoted/dotted table headers with quoted segments, no native date/time
//! values (parsed as plain strings instead).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A parsed TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    /// A string scalar (from a basic or literal string literal).
    String(String),
    /// An integer scalar.
    Integer(i64),
    /// A floating-point scalar.
    Float(f64),
    /// A boolean scalar.
    Boolean(bool),
    /// An array of values (not necessarily homogeneous, matching TOML's
    /// own permissiveness here).
    Array(Vec<TomlValue>),
    /// A table (from `[section]` headers or inline `{ ... }` tables),
    /// ordered by key for deterministic iteration.
    Table(BTreeMap<String, TomlValue>),
}

impl TomlValue {
    /// Borrows this value as a table, if it is one.
    pub fn as_table(&self) -> Option<&BTreeMap<String, TomlValue>> {
        match self {
            TomlValue::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Looks up `key` in this value if it's a table, else `None`.
    pub fn get(&self, key: &str) -> Option<&TomlValue> {
        self.as_table()?.get(key)
    }

    /// Borrows this value as a `&str`, if it's a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            TomlValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// This value as an `i64`, if it's an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            TomlValue::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// This value as an `f64`, if it's a float.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TomlValue::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// This value as a `bool`, if it's a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            TomlValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Borrows this value as an array, if it is one.
    pub fn as_array(&self) -> Option<&[TomlValue]> {
        match self {
            TomlValue::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Parses a TOML document into its root table.
    pub fn parse_str(input: &str) -> Result<Self, &'static str> {
        Parser::new(input).parse_document()
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

type PResult<T> = Result<T, &'static str>;

impl Parser {
    fn new(input: &str) -> Self {
        Parser {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Skips spaces/tabs, comments, and newlines — every kind of
    /// insignificant whitespace between top-level document items.
    fn skip_insignificant(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c == ' ' || c == '\t' || c == '\n' || c == '\r' => {
                    self.advance();
                }
                Some('#') => {
                    while matches!(self.peek(), Some(c) if c != '\n') {
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    /// Skips only same-line whitespace and a trailing comment, stopping at
    /// (not consuming) a newline or EOF — used between tokens on one
    /// logical line, where a newline that isn't inside brackets ends
    /// the entry.
    fn skip_line_whitespace(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') => {
                    self.advance();
                }
                Some('#') => {
                    while matches!(self.peek(), Some(c) if c != '\n') {
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> PResult<()> {
        if self.peek() == Some(expected) {
            self.advance();
            Ok(())
        } else {
            Err("unexpected character")
        }
    }

    fn parse_document(&mut self) -> PResult<TomlValue> {
        let mut root: BTreeMap<String, TomlValue> = BTreeMap::new();
        let mut current_path: Vec<String> = Vec::new();

        loop {
            self.skip_insignificant();
            match self.peek() {
                None => break,
                Some('[') => {
                    self.advance();
                    let path = self.parse_key_path()?;
                    self.expect_char(']')?;
                    self.skip_line_whitespace();
                    current_path = path;
                    Self::ensure_table_path(&mut root, &current_path)?;
                }
                _ => {
                    let key_path = self.parse_key_path()?;
                    self.skip_line_whitespace();
                    self.expect_char('=')?;
                    self.skip_line_whitespace();
                    let value = self.parse_value()?;
                    self.skip_line_whitespace();

                    let mut full_path = current_path.clone();
                    full_path.extend(key_path);
                    Self::insert_at_path(&mut root, &full_path, value)?;
                }
            }
        }

        Ok(TomlValue::Table(root))
    }

    fn ensure_table_path(root: &mut BTreeMap<String, TomlValue>, path: &[String]) -> PResult<()> {
        let mut table = root;
        for segment in path {
            let entry = table
                .entry(segment.clone())
                .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
            match entry {
                TomlValue::Table(inner) => table = inner,
                _ => return Err("key already defined as a non-table value"),
            }
        }
        Ok(())
    }

    fn insert_at_path(
        root: &mut BTreeMap<String, TomlValue>,
        path: &[String],
        value: TomlValue,
    ) -> PResult<()> {
        match path.split_last() {
            None => Err("empty key"),
            Some((last, parents)) => {
                let mut table = root;
                for segment in parents {
                    let entry = table
                        .entry(segment.clone())
                        .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
                    match entry {
                        TomlValue::Table(inner) => table = inner,
                        _ => return Err("key already defined as a non-table value"),
                    }
                }
                table.insert(last.clone(), value);
                Ok(())
            }
        }
    }

    /// Parses a bare or dotted key path (`a`, `a.b.c`) — also reused for
    /// table headers.
    fn parse_key_path(&mut self) -> PResult<Vec<String>> {
        let mut parts = Vec::new();
        loop {
            self.skip_line_whitespace();
            parts.push(self.parse_key_segment()?);
            self.skip_line_whitespace();
            if self.peek() == Some('.') {
                self.advance();
            } else {
                break;
            }
        }
        Ok(parts)
    }

    fn parse_key_segment(&mut self) -> PResult<String> {
        match self.peek() {
            Some('"') => self.parse_basic_string(),
            Some('\'') => self.parse_literal_string(),
            _ => {
                let mut s = String::new();
                while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_' || c == '-')
                {
                    s.push(self.advance().unwrap());
                }
                if s.is_empty() {
                    Err("expected a key")
                } else {
                    Ok(s)
                }
            }
        }
    }

    fn parse_value(&mut self) -> PResult<TomlValue> {
        match self.peek() {
            Some('"') => Ok(TomlValue::String(self.parse_basic_string()?)),
            Some('\'') => Ok(TomlValue::String(self.parse_literal_string()?)),
            Some('[') => self.parse_array(),
            Some('{') => self.parse_inline_table(),
            Some('t') if self.matches_keyword("true") => {
                self.advance_by(4);
                Ok(TomlValue::Boolean(true))
            }
            Some('f') if self.matches_keyword("false") => {
                self.advance_by(5);
                Ok(TomlValue::Boolean(false))
            }
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' => self.parse_number(),
            _ => Err("expected a value"),
        }
    }

    fn matches_keyword(&self, kw: &str) -> bool {
        kw.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn advance_by(&mut self, n: usize) {
        self.pos += n;
    }

    fn parse_basic_string(&mut self) -> PResult<String> {
        self.expect_char('"')?;
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated string"),
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(other) => s.push(other),
                    None => return Err("unterminated escape"),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn parse_literal_string(&mut self) -> PResult<String> {
        self.expect_char('\'')?;
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated string"),
                Some('\'') => break,
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn parse_number(&mut self) -> PResult<TomlValue> {
        let mut raw = String::new();
        if matches!(self.peek(), Some('-') | Some('+')) {
            raw.push(self.advance().unwrap());
        }
        let mut seen_dot = false;
        let mut seen_exp = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    raw.push(c);
                }
                self.advance();
            } else if c == '.' && !seen_dot && !seen_exp {
                seen_dot = true;
                raw.push(c);
                self.advance();
            } else if (c == 'e' || c == 'E') && !seen_exp {
                seen_exp = true;
                raw.push(c);
                self.advance();
                if matches!(self.peek(), Some('-') | Some('+')) {
                    raw.push(self.advance().unwrap());
                }
            } else {
                break;
            }
        }
        if seen_dot || seen_exp {
            raw.parse::<f64>()
                .map(TomlValue::Float)
                .map_err(|_| "invalid float")
        } else {
            raw.parse::<i64>()
                .map(TomlValue::Integer)
                .map_err(|_| "invalid integer")
        }
    }

    fn parse_array(&mut self) -> PResult<TomlValue> {
        self.expect_char('[')?;
        let mut items = Vec::new();
        loop {
            self.skip_insignificant();
            if self.peek() == Some(']') {
                self.advance();
                break;
            }
            items.push(self.parse_value()?);
            self.skip_insignificant();
            if self.peek() == Some(',') {
                self.advance();
            }
        }
        Ok(TomlValue::Array(items))
    }

    fn parse_inline_table(&mut self) -> PResult<TomlValue> {
        self.expect_char('{')?;
        let mut table = BTreeMap::new();
        loop {
            self.skip_line_whitespace();
            if self.peek() == Some('}') {
                self.advance();
                break;
            }
            let key_path = self.parse_key_path()?;
            self.skip_line_whitespace();
            self.expect_char('=')?;
            self.skip_line_whitespace();
            let value = self.parse_value()?;
            Self::insert_at_path(&mut table, &key_path, value)?;
            self.skip_line_whitespace();
            if self.peek() == Some(',') {
                self.advance();
            }
        }
        Ok(TomlValue::Table(table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_scalars() {
        let doc = TomlValue::parse_str(
            r#"
            name = "rusty_codec"
            count = 42
            ratio = 2.5
            enabled = true
            "#,
        )
        .unwrap();
        assert_eq!(doc.get("name").unwrap().as_str(), Some("rusty_codec"));
        assert_eq!(doc.get("count").unwrap().as_i64(), Some(42));
        assert_eq!(doc.get("ratio").unwrap().as_f64(), Some(2.5));
        assert_eq!(doc.get("enabled").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn parses_negative_and_underscored_numbers() {
        let doc = TomlValue::parse_str("a = -42\nb = 1_000_000\nc = -3.5e2\n").unwrap();
        assert_eq!(doc.get("a").unwrap().as_i64(), Some(-42));
        assert_eq!(doc.get("b").unwrap().as_i64(), Some(1_000_000));
        assert_eq!(doc.get("c").unwrap().as_f64(), Some(-350.0));
    }

    #[test]
    fn comments_are_ignored() {
        let doc = TomlValue::parse_str("# a comment\nx = 1 # trailing comment\n").unwrap();
        assert_eq!(doc.get("x").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn literal_strings_do_not_process_escapes() {
        let doc = TomlValue::parse_str(r"path = 'C:\Users\name'").unwrap();
        assert_eq!(doc.get("path").unwrap().as_str(), Some(r"C:\Users\name"));
    }

    #[test]
    fn basic_string_escapes_are_decoded() {
        let doc = TomlValue::parse_str(r#"greeting = "hi\nthere""#).unwrap();
        assert_eq!(doc.get("greeting").unwrap().as_str(), Some("hi\nthere"));
    }

    #[test]
    fn table_headers_nest_correctly() {
        let doc = TomlValue::parse_str(
            r#"
            [package]
            name = "rusty_codec"

            [package.metadata]
            docs = true
            "#,
        )
        .unwrap();
        let package = doc.get("package").unwrap();
        assert_eq!(package.get("name").unwrap().as_str(), Some("rusty_codec"));
        assert_eq!(
            package
                .get("metadata")
                .unwrap()
                .get("docs")
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn dotted_keys_build_nested_tables() {
        let doc = TomlValue::parse_str("a.b.c = 1\n").unwrap();
        assert_eq!(
            doc.get("a")
                .unwrap()
                .get("b")
                .unwrap()
                .get("c")
                .unwrap()
                .as_i64(),
            Some(1)
        );
    }

    #[test]
    fn single_line_arrays() {
        let doc = TomlValue::parse_str(r#"xs = [1, 2, 3]"#).unwrap();
        let arr = doc.get("xs").unwrap().as_array().unwrap();
        assert_eq!(
            arr,
            &[
                TomlValue::Integer(1),
                TomlValue::Integer(2),
                TomlValue::Integer(3)
            ]
        );
    }

    #[test]
    fn multi_line_arrays_with_trailing_comma_and_comments() {
        let doc = TomlValue::parse_str("xs = [\n  1, # one\n  2,\n  3,\n]\n").unwrap();
        let arr = doc.get("xs").unwrap().as_array().unwrap();
        assert_eq!(
            arr,
            &[
                TomlValue::Integer(1),
                TomlValue::Integer(2),
                TomlValue::Integer(3)
            ]
        );
    }

    #[test]
    fn inline_tables() {
        let doc = TomlValue::parse_str(r#"point = { x = 1, y = 2 }"#).unwrap();
        let point = doc.get("point").unwrap();
        assert_eq!(point.get("x").unwrap().as_i64(), Some(1));
        assert_eq!(point.get("y").unwrap().as_i64(), Some(2));
    }

    #[test]
    fn array_of_strings() {
        let doc = TomlValue::parse_str(r#"names = ["a", "b", "c"]"#).unwrap();
        let arr = doc.get("names").unwrap().as_array().unwrap();
        assert_eq!(
            arr,
            &[
                TomlValue::String("a".into()),
                TomlValue::String("b".into()),
                TomlValue::String("c".into())
            ]
        );
    }

    #[test]
    fn redefining_a_scalar_key_as_a_table_errors() {
        let result = TomlValue::parse_str("a = 1\n[a]\nb = 2\n");
        assert!(result.is_err());
    }

    #[test]
    fn a_realistic_cargo_toml_like_document_parses() {
        let doc = TomlValue::parse_str(
            r#"
            [package]
            name = "rusty_codec"
            version = "0.1.0"
            edition = "2024"

            [dependencies]
            rusty_wire = { path = "../rusty_wire" }
            rusty_std = { path = "../rusty_std" }
            "#,
        )
        .unwrap();
        assert_eq!(
            doc.get("package").unwrap().get("version").unwrap().as_str(),
            Some("0.1.0")
        );
        let deps = doc.get("dependencies").unwrap();
        assert_eq!(
            deps.get("rusty_wire")
                .unwrap()
                .get("path")
                .unwrap()
                .as_str(),
            Some("../rusty_wire")
        );
    }
}

#[cfg(test)]
mod real_world_tests {
    use super::*;

    #[test]
    fn parses_real_rusty_codec_cargo_toml() {
        let text = include_str!("../Cargo.toml");
        let doc =
            TomlValue::parse_str(text).expect("should parse this crate's own real Cargo.toml");
        assert_eq!(
            doc.get("package").unwrap().get("name").unwrap().as_str(),
            Some("rusty_codec")
        );
        assert!(doc.get("dependencies").unwrap().get("rusty_wire").is_some());
    }

    #[test]
    fn parses_an_inline_table_with_a_feature_array_and_hyphenated_keys() {
        // Lifted verbatim from rusty_tls's real Cargo.toml.
        let text = concat!(
            "[dependencies]\n",
            "rustls = { version = \"0.23\", default-features = false, features = [\"std\", \"tls12\", \"ring\"] }\n",
        );
        let doc = TomlValue::parse_str(text).expect("should parse a real rustls dependency line");
        let rustls = doc.get("dependencies").unwrap().get("rustls").unwrap();
        assert_eq!(rustls.get("version").unwrap().as_str(), Some("0.23"));
        assert_eq!(
            rustls.get("default-features").unwrap().as_bool(),
            Some(false)
        );
        let features = rustls.get("features").unwrap().as_array().unwrap();
        assert_eq!(
            features,
            &[
                TomlValue::String("std".into()),
                TomlValue::String("tls12".into()),
                TomlValue::String("ring".into())
            ]
        );
    }
}
