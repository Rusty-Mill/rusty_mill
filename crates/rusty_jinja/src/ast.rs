//! Parsed template structure: text/output/control-flow nodes, and the
//! expression AST used inside `{{ }}`/`{% %}`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    /// `+` — numeric addition if both sides are numbers, string
    /// concatenation if both are strings (matches real Jinja/Python
    /// overloading of `+`).
    Add,
    Sub,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Str(String),
    Num(f64),
    Bool(bool),
    None,
    Var(String),
    /// `a.b` or `a['b']` — the JSON model doesn't distinguish attribute
    /// vs. index access on an object, so both parse to this.
    Attr(Box<Expr>, String),
    /// `a[expr]` — a computed index (numeric, into an array).
    Index(Box<Expr>, Box<Expr>),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// `~` — always-string concatenation (Jinja's dedicated operator).
    Concat(Box<Expr>, Box<Expr>),
    /// `expr | name(args)` or `expr.name(args)` — both a filter and a
    /// known method call resolve here identically.
    Filter(Box<Expr>, String, Vec<Expr>),
    /// `expr is name` (`negate` for `is not`).
    Test(Box<Expr>, String, bool),
    /// `expr in expr` (`negate` for `not in`).
    In(Box<Expr>, Box<Expr>, bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Text(String),
    Output(Expr),
    If {
        /// `(condition, body)` pairs — the first `if`, then each `elif`.
        branches: Vec<(Expr, Vec<Node>)>,
        else_branch: Option<Vec<Node>>,
    },
    For {
        var: String,
        iterable: Expr,
        body: Vec<Node>,
    },
    Set {
        var: String,
        value: Expr,
    },
}
