//! Parsed program structure: patterns, actions, statements, expressions.

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
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
pub enum LValue {
    Var(String),
    Field(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    /// `$expr` — a field reference (`$0` is the whole record).
    Field(Box<Expr>),
    Var(String),
    Concat(Box<Expr>, Box<Expr>),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    /// `expr ~ /re/` (`negate = true` for `!~`).
    Match { expr: Box<Expr>, pattern: String, negate: bool },
    Assign(LValue, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Print(Vec<Expr>),
    Expr(Expr),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Begin,
    End,
    Always,
    /// A bare `/regex/` pattern parses as `Expr::Match` against `$0` — no
    /// separate variant needed.
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub pattern: Pattern,
    /// `None` means the default action: `print $0`.
    pub action: Option<Vec<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub rules: Vec<Rule>,
}
