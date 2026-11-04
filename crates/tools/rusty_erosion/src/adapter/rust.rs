//! Rust through `tree-sitter-rust`: per-function cyclomatic complexity and a
//! normalized token stream.
//!
//! CC is 1 plus one for each `if` (including `if let` and `else if`),
//! `while`, `for`, match arm after the first, match-arm guard, and `&&` or
//! `||` in a condition. `loop` and `?` add nothing: counting every `?` would
//! score idiomatic error handling as complexity. Each `fn` is its own unit,
//! nested ones included; closures count toward the enclosing `fn`. Code inside
//! macro invocations is tokens only and adds no CC.
//!
//! Tokens drop comments, map identifiers to `ID` and literals to `STR`,
//! `CHAR` or `NUM`, and keep keywords and punctuation. Items marked
//! `#[test]`, `#[cfg(test)]` or `#[cfg(all(test, ..))]` are skipped entirely,
//! and so is a file with `#![cfg(test)]`.

use std::collections::HashMap;

use tree_sitter::Node;

use super::Error;
use crate::core::{FileMeasure, FunctionMetric, Lang, NormTok};

/// A reusable Rust parser.
pub struct Parser {
    inner: tree_sitter::Parser,
}

impl Parser {
    /// Loads the Rust grammar.
    pub fn new() -> Result<Parser, Error> {
        let mut inner = tree_sitter::Parser::new();
        inner
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(Error::Language)?;
        Ok(Parser { inner })
    }

    /// Measures one Rust file.
    pub fn measure(&mut self, path: &str, source: &str) -> Result<FileMeasure, Error> {
        let tree = self
            .inner
            .parse(source, None)
            .ok_or_else(|| Error::NoTree {
                path: path.to_owned(),
            })?;
        let root = tree.root_node();
        let mut walk = Walk::new(path, source);
        if !file_is_test(root, source) {
            walk.children(root, None);
        }
        Ok(walk.finish(root.has_error()))
    }
}

/// Traversal state for one file.
struct Walk<'s> {
    path: &'s str,
    src: &'s str,
    scope: Vec<String>,
    tokens: Vec<NormTok>,
    rows: Vec<bool>,
    functions: Vec<FunctionMetric>,
    seen: HashMap<String, u32>,
}

impl<'s> Walk<'s> {
    fn new(path: &'s str, src: &'s str) -> Walk<'s> {
        let rows = vec![false; src.lines().count() + 1];
        Walk {
            path,
            src,
            scope: Vec::new(),
            tokens: Vec::new(),
            rows,
            functions: Vec::new(),
            seen: HashMap::new(),
        }
    }

    fn finish(self, parse_error: bool) -> FileMeasure {
        FileMeasure {
            path: self.path.to_owned(),
            lang: Lang::Rust,
            functions: self.functions,
            tokens: self.tokens,
            loc: self.rows.iter().filter(|r| **r).count() as u64,
            parse_error,
        }
    }

    fn text(&self, node: Node) -> &'s str {
        &self.src[node.byte_range()]
    }

    /// Visits the children of `node`, dropping any item whose preceding
    /// attributes mark it as test code, along with those attributes.
    fn children(&mut self, node: Node, mut cc: Option<&mut u32>) {
        let mut attrs: Vec<Node> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "attribute_item" => attrs.push(child),
                "line_comment" | "block_comment" => {}
                _ => {
                    if !attrs.iter().any(|a| is_test_attr(self.text(*a))) {
                        for a in attrs.iter() {
                            self.visit(*a, cc.as_deref_mut());
                        }
                        self.visit(child, cc.as_deref_mut());
                    }
                    attrs.clear();
                }
            }
        }
        for a in attrs {
            self.visit(a, cc.as_deref_mut());
        }
    }

    fn visit(&mut self, node: Node, mut cc: Option<&mut u32>) {
        if let Some(class) = literal_class(node.kind()) {
            return self.token(node, class);
        }
        match node.kind() {
            "line_comment" | "block_comment" => return,
            "function_item" => return self.function(node),
            "impl_item" => return self.scoped(node, impl_name(node, self.src), cc),
            "trait_item" | "mod_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_owned());
                return self.scoped(node, name.unwrap_or_default(), cc);
            }
            _ => {}
        }
        if node.child_count() == 0 {
            if !node.is_missing() {
                self.token(node, leaf_class(node.kind()));
            }
            return;
        }
        if let Some(c) = cc.as_deref_mut() {
            *c += decisions(node);
        }
        self.children(node, cc);
    }

    fn token(&mut self, node: Node, class: NormTok) {
        self.tokens.push(class);
        if let Some(r) = self.rows.get_mut(node.start_position().row) {
            *r = true;
        }
    }

    fn scoped(&mut self, node: Node, name: String, cc: Option<&mut u32>) {
        self.scope.push(name);
        self.children(node, cc);
        self.scope.pop();
    }

    fn function(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map_or("", |n| self.text(n))
            .to_owned();
        let mut cc = 1;
        self.scope.push(name);
        self.children(node, Some(&mut cc));
        let key = self.key();
        self.scope.pop();
        let line = node.start_position().row as u32 + 1;
        self.functions.push(FunctionMetric {
            key,
            path: self.path.to_owned(),
            line,
            cc,
        });
    }

    /// `path::scope::name`, with `#n` on the n-th repeat of the same key.
    fn key(&mut self) -> String {
        let base = std::iter::once(self.path)
            .chain(self.scope.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::");
        let n = self.seen.entry(base.clone()).or_insert(0);
        *n += 1;
        match *n {
            1 => base,
            n => format!("{base}#{n}"),
        }
    }
}

/// CC added by `node` itself, not counting its descendants.
fn decisions(node: Node) -> u32 {
    match node.kind() {
        "if_expression" | "while_expression" | "for_expression" => 1,
        "match_block" => {
            let mut cursor = node.walk();
            let arms = node
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "match_arm")
                .count() as u32;
            arms.saturating_sub(1)
        }
        "match_pattern" => u32::from(node.child_by_field_name("condition").is_some()),
        "binary_expression" => match node.child_by_field_name("operator").map(|o| o.kind()) {
            Some("&&" | "||") => 1,
            _ => 0,
        },
        "let_chain" => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|c| c.kind() == "&&")
                .count() as u32
        }
        _ => 0,
    }
}

fn literal_class(kind: &str) -> Option<NormTok> {
    match kind {
        "string_literal" | "raw_string_literal" => Some("STR"),
        "char_literal" => Some("CHAR"),
        "integer_literal" | "float_literal" => Some("NUM"),
        _ => None,
    }
}

/// Identifiers become `ID`; any other leaf keeps its kind, which is the token
/// text for keywords and punctuation.
fn leaf_class(kind: &'static str) -> NormTok {
    match kind {
        "identifier" | "type_identifier" | "field_identifier" | "shorthand_field_identifier" => {
            "ID"
        }
        k => k,
    }
}

/// `Type` or `<Type as Trait>`, without generics, for an `impl` block.
fn impl_name(node: Node, src: &str) -> String {
    let part = |field: &str| {
        node.child_by_field_name(field)
            .map(|n| strip_generics(&src[n.byte_range()]))
    };
    match (part("type"), part("trait")) {
        (Some(ty), Some(tr)) => format!("<{ty} as {tr}>"),
        (Some(ty), None) => ty,
        _ => String::new(),
    }
}

fn strip_generics(s: &str) -> String {
    s.split('<')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

/// Whether an outer attribute marks its item as test-only.
fn is_test_attr(text: &str) -> bool {
    let t: String = text.split_whitespace().collect();
    let Some(inner) = t.strip_prefix("#[").and_then(|t| t.strip_suffix(']')) else {
        return false;
    };
    inner == "test" || is_test_cfg(inner)
}

/// `cfg(test)` or `cfg(all(.., test, ..))`. `any` and `not` keep the item.
fn is_test_cfg(attr: &str) -> bool {
    let Some(pred) = attr.strip_prefix("cfg(").and_then(|t| t.strip_suffix(')')) else {
        return false;
    };
    if pred == "test" {
        return true;
    }
    let Some(args) = pred.strip_prefix("all(").and_then(|t| t.strip_suffix(')')) else {
        return false;
    };
    top_level_args(args).any(|a| a == "test")
}

/// Comma-separated arguments at nesting depth 0.
fn top_level_args(s: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter()
}

/// Whether the file opens with `#![cfg(test)]`.
fn file_is_test(root: Node, src: &str) -> bool {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|c| c.kind() == "inner_attribute_item")
        .any(|c| {
            let t: String = src[c.byte_range()].split_whitespace().collect();
            t.strip_prefix("#![")
                .and_then(|t| t.strip_suffix(']'))
                .is_some_and(is_test_cfg)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attributes() {
        assert!(is_test_attr("#[test]"));
        assert!(is_test_attr("#[cfg(test)]"));
        assert!(is_test_attr("#[cfg( all(unix, test) )]"));
        assert!(!is_test_attr("#[cfg(any(test, feature = \"x\"))]"));
        assert!(!is_test_attr("#[cfg(not(test))]"));
        assert!(!is_test_attr("#[cfg(all(unix, not(test)))]"));
        assert!(!is_test_attr("#[derive(Debug)]"));
    }
}
