//! Scans raw template text into a [`Node`] tree: plain text, `{{ output }}`
//! expressions, and `{% tag %}` control flow (`if`/`elif`/`else`/`endif`,
//! `for`/`endfor`, `set`), including `{%-`/`-%}`/`{{-`/`-}}` whitespace
//! trimming.
//!
//! Whitespace trimming across a block boundary needs care: `{%- if ... -%}`
//! trims the text *inside* the `if` body (parsed by a recursive call), not
//! the text after the whole `if`/`endif` — so the trim-right flag of the
//! *opening* tag has to be threaded into the recursive [`parse_block`]
//! call as its starting state, and the trim-right flag of whichever tag
//! *closes* a block (`endif`/`endfor`, or `else`/`elif` continuing to the
//! next branch) has to be threaded back **out** to the caller so it can
//! seed the text that follows. That's what the `bool` on every
//! [`Terminator`] variant carries.

use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{Expr, Node};
use crate::lexer::tokenize;
use crate::parser::Parser;

/// Errors compiling a template.
#[derive(Debug, Clone, PartialEq)]
pub enum JinjaError {
    /// An expression inside `{{ }}`/`{% %}` failed to tokenize or parse.
    Expression(&'static str),
    /// A structural error in the tag stream itself (unclosed/mismatched
    /// `if`/`for`, an unrecognized tag keyword, an unterminated `{{`/`{%`).
    Syntax(&'static str),
}

/// What kind of block a recursive [`parse_block`] call is inside — which
/// closing/continuation tags are legal there.
#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    Top,
    If,
    For,
}

/// Which tag a [`parse_block`] call stopped at. Every variant's `bool` is
/// that tag's own trim-right (`-%}`) flag — see the module doc.
enum Terminator {
    Eof,
    Endif(bool),
    Endfor(bool),
    Else(bool),
    /// Carries the `elif`'s own condition source text and trim-right flag.
    Elif(String, bool),
}

struct Scanner {
    chars: Vec<char>,
    pos: usize,
}

impl Scanner {
    fn new(src: &str) -> Self {
        Scanner {
            chars: src.chars().collect(),
            pos: 0,
        }
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }

    fn at(&self, s: &str) -> bool {
        let pat: Vec<char> = s.chars().collect();
        self.pos + pat.len() <= self.chars.len()
            && self.chars[self.pos..self.pos + pat.len()] == pat[..]
    }

    /// Reads a `{{ ... }}`/`{% ... %}` tag starting at the current
    /// position (the opening delimiter must already be at `self.pos`),
    /// returning `(trim_left, trim_right, inner_text)` with the cursor
    /// left just past the closing delimiter.
    fn read_tag(&mut self, open: &str, close: &str) -> Result<(bool, bool, String), JinjaError> {
        self.pos += open.chars().count();
        let trim_left = self.chars.get(self.pos) == Some(&'-');
        if trim_left {
            self.pos += 1;
        }

        let start = self.pos;
        let close_pat: Vec<char> = close.chars().collect();
        let mut end = None;
        let mut trim_right = false;
        let mut i = self.pos;
        while i < self.chars.len() {
            if i + close_pat.len() <= self.chars.len()
                && self.chars[i..i + close_pat.len()] == close_pat[..]
            {
                end = Some(i);
                break;
            }
            if self.chars[i] == '-'
                && i + 1 + close_pat.len() <= self.chars.len()
                && self.chars[i + 1..i + 1 + close_pat.len()] == close_pat[..]
            {
                end = Some(i);
                trim_right = true;
                break;
            }
            i += 1;
        }
        let end = end.ok_or(JinjaError::Syntax("unterminated tag"))?;
        let inner = self.slice(start, end);
        self.pos = end + usize::from(trim_right) + close_pat.len();
        Ok((trim_left, trim_right, inner))
    }
}

fn parse_expr_src(src: &str) -> Result<Expr, JinjaError> {
    Parser::new(tokenize(src).map_err(JinjaError::Expression)?)
        .parse_expr_to_eof()
        .map_err(JinjaError::Expression)
}

/// The recognized `{% ... %}` tag keywords, with their rest-of-tag text.
enum RawTag<'a> {
    If(&'a str),
    Elif(&'a str),
    Else,
    Endif,
    For(&'a str),
    Endfor,
    Set(&'a str),
}

fn classify_tag(inner: &str) -> Result<RawTag<'_>, JinjaError> {
    if let Some(rest) = inner.strip_prefix("if ") {
        Ok(RawTag::If(rest))
    } else if let Some(rest) = inner.strip_prefix("elif ") {
        Ok(RawTag::Elif(rest))
    } else if inner == "else" {
        Ok(RawTag::Else)
    } else if inner == "endif" {
        Ok(RawTag::Endif)
    } else if let Some(rest) = inner.strip_prefix("for ") {
        Ok(RawTag::For(rest))
    } else if inner == "endfor" {
        Ok(RawTag::Endfor)
    } else if let Some(rest) = inner.strip_prefix("set ") {
        Ok(RawTag::Set(rest))
    } else {
        Err(JinjaError::Syntax("unrecognized tag"))
    }
}

/// Parses a sequence of nodes until EOF (`BlockKind::Top`) or a closing/
/// continuation tag legal for `kind` (`elif`/`else`/`endif` for `If`,
/// `endfor` for `For`). `initial_trim` seeds whether the *first* text node
/// should have its leading whitespace trimmed — the trim-right flag of
/// whichever tag opened this block (see the module doc).
fn parse_block(
    scanner: &mut Scanner,
    kind: BlockKind,
    initial_trim: bool,
) -> Result<(Vec<Node>, Terminator), JinjaError> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut trim_next_text_start = initial_trim;

    loop {
        let text_start = scanner.pos;
        while scanner.pos < scanner.chars.len() && !scanner.at("{{") && !scanner.at("{%") {
            scanner.pos += 1;
        }
        let mut text = scanner.slice(text_start, scanner.pos);
        if trim_next_text_start {
            text = text.trim_start().into();
        }

        if scanner.pos >= scanner.chars.len() {
            if !text.is_empty() {
                nodes.push(Node::Text(text));
            }
            return if kind == BlockKind::Top {
                Ok((nodes, Terminator::Eof))
            } else {
                Err(JinjaError::Syntax(
                    "unexpected end of template inside a block",
                ))
            };
        }

        if scanner.at("{{") {
            let (trim_left, trim_right, inner) = scanner.read_tag("{{", "}}")?;
            if trim_left {
                text = text.trim_end().into();
            }
            if !text.is_empty() {
                nodes.push(Node::Text(text));
            }
            nodes.push(Node::Output(parse_expr_src(&inner)?));
            trim_next_text_start = trim_right;
            continue;
        }

        let (trim_left, trim_right, inner) = scanner.read_tag("{%", "%}")?;
        if trim_left {
            text = text.trim_end().into();
        }
        if !text.is_empty() {
            nodes.push(Node::Text(text));
        }

        let inner_trimmed = inner.trim();
        match classify_tag(inner_trimmed)? {
            RawTag::If(cond) => {
                let (node, tail_trim) = parse_if_chain(scanner, cond, trim_right)?;
                nodes.push(node);
                trim_next_text_start = tail_trim;
            }
            RawTag::For(spec) => {
                let (node, tail_trim) = parse_for_chain(scanner, spec, trim_right)?;
                nodes.push(node);
                trim_next_text_start = tail_trim;
            }
            RawTag::Set(spec) => {
                let (var, value_src) = spec
                    .split_once('=')
                    .ok_or(JinjaError::Syntax("expected 'set x = y'"))?;
                let value = parse_expr_src(value_src.trim())?;
                nodes.push(Node::Set {
                    var: var.trim().into(),
                    value,
                });
                trim_next_text_start = trim_right;
            }
            RawTag::Endif if kind == BlockKind::If => {
                return Ok((nodes, Terminator::Endif(trim_right)));
            }
            RawTag::Else if kind == BlockKind::If => {
                return Ok((nodes, Terminator::Else(trim_right)));
            }
            RawTag::Elif(cond) if kind == BlockKind::If => {
                return Ok((nodes, Terminator::Elif(cond.trim().into(), trim_right)));
            }
            RawTag::Endfor if kind == BlockKind::For => {
                return Ok((nodes, Terminator::Endfor(trim_right)));
            }
            RawTag::Endif | RawTag::Else | RawTag::Elif(_) | RawTag::Endfor => {
                return Err(JinjaError::Syntax(
                    "closing tag doesn't match the enclosing block",
                ));
            }
        }
    }
}

/// Parses an `if` statement's full chain (`if` → any `elif`s → optional
/// `else` → `endif`), given the opening `{% if ... %}` tag's condition
/// source and its own trim-right flag (seeding the first branch's body).
/// Returns the built node plus the closing `endif`'s trim-right flag, for
/// the caller to seed whatever text follows.
fn parse_if_chain(
    scanner: &mut Scanner,
    first_cond_src: &str,
    first_trim: bool,
) -> Result<(Node, bool), JinjaError> {
    let mut branches = Vec::new();
    let mut cond_src: String = first_cond_src.trim().into();
    let mut body_trim = first_trim;
    loop {
        let cond = parse_expr_src(&cond_src)?;
        let (body, terminator) = parse_block(scanner, BlockKind::If, body_trim)?;
        branches.push((cond, body));
        match terminator {
            Terminator::Elif(next_cond, trim) => {
                cond_src = next_cond;
                body_trim = trim;
            }
            Terminator::Else(else_trim) => {
                let (else_body, terminator2) = parse_block(scanner, BlockKind::If, else_trim)?;
                match terminator2 {
                    Terminator::Endif(tail_trim) => {
                        return Ok((
                            Node::If {
                                branches,
                                else_branch: Some(else_body),
                            },
                            tail_trim,
                        ));
                    }
                    _ => return Err(JinjaError::Syntax("expected 'endif' after 'else'")),
                }
            }
            Terminator::Endif(tail_trim) => {
                return Ok((
                    Node::If {
                        branches,
                        else_branch: None,
                    },
                    tail_trim,
                ));
            }
            Terminator::Eof | Terminator::Endfor(_) => {
                unreachable!("parse_block(BlockKind::If) only ever returns Elif/Else/Endif")
            }
        }
    }
}

/// Parses a `for` statement (`for ... in ...` → `endfor`), given the
/// opening tag's spec text and trim-right flag. Returns the built node
/// plus the closing `endfor`'s trim-right flag.
fn parse_for_chain(
    scanner: &mut Scanner,
    spec: &str,
    first_trim: bool,
) -> Result<(Node, bool), JinjaError> {
    let (var, iterable_src) = spec
        .split_once(" in ")
        .ok_or(JinjaError::Syntax("expected 'for x in y'"))?;
    let iterable = parse_expr_src(iterable_src.trim())?;
    let (body, terminator) = parse_block(scanner, BlockKind::For, first_trim)?;
    match terminator {
        Terminator::Endfor(tail_trim) => Ok((
            Node::For {
                var: var.trim().into(),
                iterable,
                body,
            },
            tail_trim,
        )),
        _ => Err(JinjaError::Syntax("expected 'endfor'")),
    }
}

/// Compiles `src` into a node tree.
pub fn compile(src: &str) -> Result<Vec<Node>, JinjaError> {
    let mut scanner = Scanner::new(src);
    let (nodes, terminator) = parse_block(&mut scanner, BlockKind::Top, false)?;
    match terminator {
        Terminator::Eof => Ok(nodes),
        _ => Err(JinjaError::Syntax("unexpected closing tag at top level")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinOp;

    #[test]
    fn plain_text_with_no_tags() {
        let nodes = compile("hello world").unwrap();
        assert_eq!(nodes, alloc::vec![Node::Text("hello world".into())]);
    }

    #[test]
    fn output_expression() {
        let nodes = compile("hi {{ name }}!").unwrap();
        assert_eq!(
            nodes,
            alloc::vec![
                Node::Text("hi ".into()),
                Node::Output(Expr::Var("name".into())),
                Node::Text("!".into()),
            ]
        );
    }

    #[test]
    fn if_else_endif() {
        let nodes = compile("{% if x %}A{% else %}B{% endif %}").unwrap();
        match &nodes[0] {
            Node::If {
                branches,
                else_branch,
            } => {
                assert_eq!(branches.len(), 1);
                assert_eq!(branches[0].1, alloc::vec![Node::Text("A".into())]);
                assert_eq!(else_branch, &Some(alloc::vec![Node::Text("B".into())]));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn if_elif_else_endif() {
        let nodes = compile("{% if a %}A{% elif b %}B{% else %}C{% endif %}").unwrap();
        match &nodes[0] {
            Node::If {
                branches,
                else_branch,
            } => {
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].1, alloc::vec![Node::Text("A".into())]);
                assert_eq!(branches[1].1, alloc::vec![Node::Text("B".into())]);
                assert_eq!(else_branch, &Some(alloc::vec![Node::Text("C".into())]));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn for_loop() {
        let nodes = compile("{% for x in items %}{{ x }}{% endfor %}").unwrap();
        match &nodes[0] {
            Node::For {
                var,
                iterable,
                body,
            } => {
                assert_eq!(var, "x");
                assert_eq!(*iterable, Expr::Var("items".into()));
                assert_eq!(*body, alloc::vec![Node::Output(Expr::Var("x".into()))]);
            }
            other => panic!("expected For, got {other:?}"),
        }
    }

    #[test]
    fn set_statement() {
        let nodes = compile("{% set x = 1 + 2 %}").unwrap();
        assert_eq!(
            nodes,
            alloc::vec![Node::Set {
                var: "x".into(),
                value: Expr::BinOp(
                    BinOp::Add,
                    alloc::boxed::Box::new(Expr::Num(1.0)),
                    alloc::boxed::Box::new(Expr::Num(2.0))
                ),
            }]
        );
    }

    #[test]
    fn whitespace_trim_control() {
        // `{%-` trims trailing whitespace of the preceding text; `-%}`
        // trims leading whitespace of the *body* (threaded into the
        // recursive parse for the if-block).
        let nodes = compile("A   \n  {%- if true -%}  \n   B  {% endif %}").unwrap();
        assert_eq!(nodes[0], Node::Text("A".into()));
        match &nodes[1] {
            Node::If { branches, .. } => {
                assert_eq!(branches[0].1, alloc::vec![Node::Text("B  ".into())]);
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn trim_right_on_endif_trims_the_text_that_follows_the_whole_if_statement() {
        let nodes = compile("{% if true %}A{% endif -%}   B").unwrap();
        // The trailing "B" text should have its leading whitespace
        // stripped by endif's `-%}`, proving the tail-trim flag threads
        // back out to the enclosing block, not just into the body.
        assert_eq!(nodes.last(), Some(&Node::Text("B".into())));
    }

    #[test]
    fn mismatched_endfor_on_an_if_block_is_an_error() {
        assert!(compile("{% if x %}A{% endfor %}").is_err());
    }

    #[test]
    fn unterminated_tag_is_an_error() {
        assert!(compile("{{ x").is_err());
        assert!(compile("{% if x %}").is_err());
    }
}
