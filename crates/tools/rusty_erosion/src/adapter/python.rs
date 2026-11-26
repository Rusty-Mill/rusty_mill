//! Python through `tree-sitter-python`: per-function cyclomatic complexity and
//! a normalized token stream (card #705).
//!
//! CC follows radon's table, the tool SlopCodeBench uses for its CC (D41
//! thresholds stay the same): 1 plus one for each `if`, `elif`, conditional
//! expression, `for`, `while`, `else` on a loop, `except`, `with`, `assert`,
//! comprehension `for` and `if`, `and`/`or` operator, and `case` after the
//! first. `else` on an `if` or `try` and `finally` add nothing. Each `def` is its
//! own unit, nested ones included; lambdas count toward the enclosing `def`.
//! Module-level code adds no CC but its tokens count.
//!
//! Tokens drop comments, map identifiers to `ID` and literals to `STR` (one per
//! string, f-string interpolations included), `NUM`, `BOOL` or `NONE`, and keep
//! keywords and punctuation.

use std::collections::HashMap;

use tree_sitter::Node;

use super::Error;
use crate::core::{FileMeasure, FunctionMetric, Lang, NormTok};

/// A reusable Python parser.
pub struct Parser {
    inner: tree_sitter::Parser,
}

impl Parser {
    /// Loads the Python grammar.
    pub fn new() -> Result<Parser, Error> {
        let mut inner = tree_sitter::Parser::new();
        inner
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(Error::Language)?;
        Ok(Parser { inner })
    }

    /// Measures one Python file.
    pub fn measure(&mut self, path: &str, source: &str) -> Result<FileMeasure, Error> {
        let tree = self
            .inner
            .parse(source, None)
            .ok_or_else(|| Error::NoTree {
                path: path.to_owned(),
            })?;
        let root = tree.root_node();
        let mut walk = Walk::new(path, source);
        walk.children(root, None);
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
    /// Whitespace-only lines, which a multi-line string does not count.
    blank: Vec<bool>,
    functions: Vec<FunctionMetric>,
    seen: HashMap<String, u32>,
}

impl<'s> Walk<'s> {
    fn new(path: &'s str, src: &'s str) -> Walk<'s> {
        Walk {
            path,
            src,
            scope: Vec::new(),
            tokens: Vec::new(),
            rows: vec![false; src.lines().count() + 1],
            blank: src.lines().map(|l| l.trim().is_empty()).collect(),
            functions: Vec::new(),
            seen: HashMap::new(),
        }
    }

    fn finish(self, parse_error: bool) -> FileMeasure {
        FileMeasure {
            path: self.path.to_owned(),
            lang: Lang::Python,
            functions: self.functions,
            tokens: self.tokens,
            loc: self.rows.iter().filter(|r| **r).count() as u64,
            parse_error,
        }
    }

    fn children(&mut self, node: Node, mut cc: Option<&mut u32>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child, cc.as_deref_mut());
        }
    }

    fn visit(&mut self, node: Node, mut cc: Option<&mut u32>) {
        if let Some(class) = literal_class(node.kind()) {
            return self.token(node, class);
        }
        match node.kind() {
            "comment" => return,
            "function_definition" => return self.function(node),
            "class_definition" => {
                let name = self.name(node);
                self.scope.push(name);
                self.children(node, cc);
                self.scope.pop();
                return;
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
        for row in node.start_position().row..=node.end_position().row {
            let blank = self.blank.get(row).copied().unwrap_or(true);
            if let Some(r) = self.rows.get_mut(row) {
                *r |= !blank;
            }
        }
    }

    fn name(&self, node: Node) -> String {
        node.child_by_field_name("name")
            .map_or("", |n| &self.src[n.byte_range()])
            .to_owned()
    }

    fn function(&mut self, node: Node) {
        let mut cc = 1;
        self.scope.push(self.name(node));
        self.children(node, Some(&mut cc));
        let key = self.key();
        self.scope.pop();
        self.functions.push(FunctionMetric {
            key,
            path: self.path.to_owned(),
            line: node.start_position().row as u32 + 1,
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
        "if_statement"
        | "elif_clause"
        | "conditional_expression"
        | "for_statement"
        | "while_statement"
        | "except_clause"
        | "except_group_clause"
        | "with_statement"
        | "assert_statement"
        | "for_in_clause"
        | "if_clause"
        | "boolean_operator" => 1,
        "else_clause" => {
            let parent = node.parent().map(|p| p.kind());
            u32::from(matches!(parent, Some("for_statement" | "while_statement")))
        }
        "match_statement" => {
            let Some(body) = node.child_by_field_name("body") else {
                return 0;
            };
            let mut cursor = body.walk();
            let cases = body
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "case_clause")
                .count() as u32;
            cases.saturating_sub(1)
        }
        _ => 0,
    }
}

fn literal_class(kind: &str) -> Option<NormTok> {
    match kind {
        "string" => Some("STR"),
        "integer" | "float" => Some("NUM"),
        "true" | "false" => Some("BOOL"),
        "none" => Some("NONE"),
        _ => None,
    }
}

/// Identifiers become `ID`; any other leaf keeps its kind, which is the token
/// text for keywords and punctuation.
fn leaf_class(kind: &'static str) -> NormTok {
    match kind {
        "identifier" => "ID",
        k => k,
    }
}
