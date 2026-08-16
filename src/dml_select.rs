//! SQL parser: single-table `SELECT` (DML subset, foundation-tier `A4b`).
//! No joins, aggregates, or subqueries. Parses `WHERE` into an [`Expr`]
//! tree but does not evaluate it — evaluation is `A6`.
//! Grammar reference: <https://www.sqlite.org/lang_select.html>.

use crate::ddl::ParseError;
use crate::token::Token;
use crate::value::Value;

/// The result-column list of a `SELECT`.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumns {
    /// `SELECT * FROM ...`
    All,
    /// `SELECT a, b FROM ...`
    Named(Vec<String>),
    /// A select list that's entirely aggregate-function calls, e.g.
    /// `SELECT COUNT(*), SUM(a) FROM t`. Evaluated over every row matching
    /// `filter` into a single output row — this crate has no `GROUP BY`
    /// yet, so grouped aggregation (multiple output rows, one per group)
    /// isn't supported; only whole-table aggregation is.
    Aggregates(Vec<AggregateCall>),
    /// A select list that's entirely window-function calls, e.g.
    /// `SELECT SUM(a) OVER (PARTITION BY b) FROM t`. See [`WindowCall`]'s
    /// doc comment for this crate's "whole partition, no frame" scope.
    Window(Vec<WindowCall>),
}

/// One window-function call in a window select list, e.g. the
/// `SUM(a) OVER (PARTITION BY b)` in
/// `SELECT SUM(a) OVER (PARTITION BY b) FROM t`.
///
/// **Scope, stated plainly:** real SQLite window functions support an
/// `ORDER BY` inside `OVER (...)` plus an explicit frame clause
/// (`ROWS`/`RANGE BETWEEN ...`), which together let a window function's
/// result differ row-by-row within a partition (a running total, a rank,
/// `LAG`/`LEAD`, ...). Building that needs real per-partition ordering
/// and frame-boundary machinery — a comparable amount of new grammar and
/// execution logic to the vtab epic (#38), not a small addition. This
/// crate only supports `PARTITION BY` with no `ORDER BY`/frame: every
/// row in a partition gets the same whole-partition aggregate value
/// (built on the same [`crate::Aggregate`] used for whole-table
/// aggregation — see [`crate::Connection::create_window_function`]).
/// `ROW_NUMBER`/`RANK`/`DENSE_RANK`/`NTILE`/`LAG`/`LEAD` (which are
/// inherently row-position-dependent, not whole-partition aggregates)
/// aren't supported for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowCall {
    pub name: String,
    pub arg: AggregateArg,
    pub partition_by: Vec<String>,
}

/// One aggregate-function call in an aggregate select list, e.g. the
/// `COUNT(*)` in `SELECT COUNT(*) FROM t`.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateCall {
    pub name: String,
    pub arg: AggregateArg,
}

/// An aggregate call's argument.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateArg {
    /// `*`, as in `COUNT(*)`. The parser only accepts this for `COUNT` —
    /// real SQLite doesn't allow `SUM(*)`/`MIN(*)`/etc. either.
    Star,
    Expr(Box<Expr>),
}

/// A parsed single-table `SELECT` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub columns: SelectColumns,
    pub table_name: String,
    pub filter: Option<Expr>,
    /// Whether `DISTINCT` followed `SELECT` (issue #116) — the engine
    /// dedups the final output rows, preserving first-occurrence order.
    pub distinct: bool,
}

/// A minimal expression tree — enough to represent `WHERE` filters.
/// Evaluated by `A6`, not here.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(String),
    Literal(Value),
    BinaryOp {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `left AND right` (issue #112). Three-valued: `eval.rs` combines
    /// per SQLite's own `NULL`-propagation rule (`FALSE AND NULL` is
    /// `FALSE`, `TRUE AND NULL` is `NULL`), not plain two-valued boolean
    /// `&&`.
    And(Box<Expr>, Box<Expr>),
    /// `left OR right` (issue #112). Three-valued, mirroring [`Expr::And`]
    /// (`TRUE OR NULL` is `TRUE`, `FALSE OR NULL` is `NULL`).
    Or(Box<Expr>, Box<Expr>),
    /// `NOT expr` (issue #112). `NOT NULL` is `NULL`.
    Not(Box<Expr>),
    /// `left LIKE pattern [ESCAPE escape]` (`NOT LIKE` if `negate`) —
    /// issue #113. `%` matches any run of characters, `_` matches one,
    /// ASCII-case-insensitive.
    Like {
        left: Box<Expr>,
        pattern: Box<Expr>,
        escape: Option<Box<Expr>>,
        negate: bool,
    },
    /// `left GLOB pattern` (`NOT GLOB` if `negate`) — issue #113. Unix
    /// glob syntax (`*`/`?`/`[...]`), case-sensitive, no escape.
    Glob {
        left: Box<Expr>,
        pattern: Box<Expr>,
        negate: bool,
    },
    /// `expr BETWEEN low AND high` (`NOT BETWEEN` if `negate`) — issue
    /// #113. Sugar for `expr >= low AND expr <= high`.
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negate: bool,
    },
    /// `CASE [operand] WHEN cond1 THEN result1 ... [ELSE else_result] END`
    /// — issue #115. `operand` is `Some` for the simple form (`CASE
    /// operand WHEN val THEN ...`, matched by equality) and `None` for
    /// the searched form (`CASE WHEN cond THEN ...`, each `cond`
    /// evaluated as a boolean). The first matching branch's result wins;
    /// `else_result` (or `NULL` if absent) if none match.
    Case {
        operand: Option<Box<Expr>>,
        branches: Vec<(Expr, Expr)>,
        else_result: Option<Box<Expr>>,
    },
    /// `expr IN (v1, v2, ...)` (`NOT IN` if `negate`) — issue #114,
    /// literal-list form only. `IN (SELECT ...)` (subquery form) needs
    /// nested query execution, which this crate's `Expr`/`eval.rs`
    /// layering doesn't support yet — tracked separately as a
    /// new-subsystem gap alongside subqueries in general (issue #131).
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negate: bool,
    },
    /// A scalar function call, e.g. `UPPER(name)`. Evaluated only by
    /// `eval::evaluate_with_functions` — plain `evaluate`/`evaluate_bool`
    /// (which predate function-call support) error on this variant rather
    /// than silently treating it as something else.
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
    /// A `?`/`?N`/`:name`/`@name`/`$name` bound-parameter marker — see
    /// `docs/adr/0002-parameter-markers.md`. `crate::Statement` resolves
    /// these to a bound value (or `Value::Null`, for an unbound one)
    /// before evaluation; the plain `eval`/`engine` functions (given no
    /// bindings to consult) also treat every `Parameter` as `Value::Null`,
    /// matching real SQLite's own unbound-parameter default.
    Parameter(ParamMarker),
}

/// A `?`/`?N`/`:name`/`@name`/`$name`-style bound-parameter marker, as
/// parsed. Not yet resolved to a concrete index — see
/// `docs/adr/0002-parameter-markers.md`'s index-resolution rule, applied
/// by `crate::Statement` at prepare time.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamMarker {
    /// Bare `?` — positional, auto-numbered by left-to-right occurrence
    /// order starting at 1 (SQLite's convention).
    Anonymous,
    /// `?N` — an explicit 1-based positional index.
    Numbered(usize),
    /// `:name`/`@name`/`$name`, sigil included — SQLite treats these as
    /// distinct namespaces even for matching name text (`:foo` and
    /// `@foo` are different parameters).
    Named(String),
}

/// Parses a `Token::Param`'s stored spec text into a [`ParamMarker`].
pub(crate) fn parse_param_marker(spec: &str) -> ParamMarker {
    if spec.is_empty() {
        ParamMarker::Anonymous
    } else if let Ok(n) = spec.parse::<usize>() {
        ParamMarker::Numbered(n)
    } else {
        ParamMarker::Named(spec.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// Parses a single-table `SELECT` statement from a token stream (as
/// produced by [`crate::tokenize`]).
pub fn parse_select(tokens: &[Token]) -> Result<Select, ParseError> {
    let mut p = SelectParser { tokens, pos: 0 };
    p.expect_ident("SELECT")?;

    let distinct = if p.peek_ident("DISTINCT") {
        p.advance();
        true
    } else {
        false
    };

    let columns = if p.peek_punct("*") {
        p.advance();
        SelectColumns::All
    } else if p.starts_aggregate_call() {
        let first = p.parse_aggregate_call()?;
        if p.peek_ident("OVER") {
            let mut windows = vec![p.parse_window_call(first)?];
            while p.peek_punct(",") {
                p.advance();
                let call = p.parse_aggregate_call()?;
                windows.push(p.parse_window_call(call)?);
            }
            SelectColumns::Window(windows)
        } else {
            let mut calls = vec![first];
            while p.peek_punct(",") {
                p.advance();
                calls.push(p.parse_aggregate_call()?);
            }
            SelectColumns::Aggregates(calls)
        }
    } else {
        let mut cols = vec![p.expect_any_ident()?];
        while p.peek_punct(",") {
            p.advance();
            cols.push(p.expect_any_ident()?);
        }
        SelectColumns::Named(cols)
    };

    p.expect_ident("FROM")?;
    let table_name = p.expect_any_ident()?;

    let filter = if p.peek_ident("WHERE") {
        p.advance();
        Some(p.parse_or_expr()?)
    } else {
        None
    };

    Ok(Select {
        columns,
        table_name,
        filter,
        distinct,
    })
}

/// Parses a single expression (the full boolean/comparison precedence
/// chain used by `WHERE`) starting at `tokens[pos]`. Returns the parsed
/// [`Expr`] and the index of the first token past it.
///
/// Reused by `ddl.rs` for `CHECK`/`DEFAULT` column constraints (issue
/// #117), which need the same expression grammar without a full `SELECT`
/// wrapped around it — rather than duplicating `parse_or_expr` and its
/// whole precedence chain in a second parser.
pub(crate) fn parse_expr_at(tokens: &[Token], pos: usize) -> Result<(Expr, usize), ParseError> {
    let mut p = SelectParser { tokens, pos };
    let expr = p.parse_or_expr()?;
    Ok((expr, p.pos))
}

/// Parses a single primary expression (a literal, column, function call,
/// `CASE`, or parameter — no comparison operator required) starting at
/// `tokens[pos]`. Returns the parsed [`Expr`] and the index of the first
/// token past it.
///
/// Reused by `ddl.rs` for `DEFAULT` column-constraint values (issue
/// #117): [`parse_expr_at`] goes through `parse_comparison`, which
/// *requires* a binary operator after its left operand (it models
/// `WHERE`, where a bare value isn't itself a filter) — wrong for
/// `DEFAULT 1`, a bare value with no operator at all.
pub(crate) fn parse_operand_at(tokens: &[Token], pos: usize) -> Result<(Expr, usize), ParseError> {
    let mut p = SelectParser { tokens, pos };
    let expr = p.parse_operand()?;
    Ok((expr, p.pos))
}

struct SelectParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> SelectParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    /// Whether the select list starting at the current position looks like
    /// `IDENT(` — an aggregate call — rather than a plain column name.
    fn starts_aggregate_call(&self) -> bool {
        matches!(self.peek(), Some(Token::Ident(_)))
            && matches!(self.peek_at(1), Some(Token::Punct(s)) if *s == "(")
    }

    fn parse_aggregate_call(&mut self) -> Result<AggregateCall, ParseError> {
        let name = self.expect_any_ident()?;
        self.expect_punct("(")?;
        let arg = if self.peek_punct("*") {
            if !name.eq_ignore_ascii_case("COUNT") {
                return Err(ParseError::UnexpectedToken(
                    "'*' is only a valid argument to COUNT".to_string(),
                ));
            }
            self.advance();
            AggregateArg::Star
        } else {
            AggregateArg::Expr(Box::new(self.parse_operand()?))
        };
        self.expect_punct(")")?;
        Ok(AggregateCall { name, arg })
    }

    /// Parses `OVER (PARTITION BY col1, col2, ...)` (or `OVER ()`, no
    /// partitioning) following an already-parsed [`AggregateCall`], into
    /// a [`WindowCall`] — see that type's doc comment for the "no
    /// `ORDER BY`/frame clause" scope limit.
    fn parse_window_call(&mut self, call: AggregateCall) -> Result<WindowCall, ParseError> {
        self.expect_ident("OVER")?;
        self.expect_punct("(")?;
        let mut partition_by = Vec::new();
        if self.peek_ident("PARTITION") {
            self.advance();
            self.expect_ident("BY")?;
            partition_by.push(self.expect_any_ident()?);
            while self.peek_punct(",") {
                self.advance();
                partition_by.push(self.expect_any_ident()?);
            }
        }
        self.expect_punct(")")?;
        Ok(WindowCall {
            name: call.name,
            arg: call.arg,
            partition_by,
        })
    }

    fn peek_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Token::Punct(s)) if *s == p)
    }

    fn peek_ident(&self, keyword: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos);
        self.pos += 1;
        tok
    }

    fn expect_ident(&mut self, keyword: &str) -> Result<(), ParseError> {
        match self.advance() {
            Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword) => Ok(()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    fn expect_any_ident(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<(), ParseError> {
        match self.advance() {
            Some(Token::Punct(s)) if *s == p => Ok(()),
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    /// `expr (OR expr)*`, left-associative — the lowest-precedence level
    /// of the `WHERE` boolean grammar (issue #112).
    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.peek_ident("OR") {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `expr (AND expr)*`, left-associative — binds tighter than `OR`,
    /// looser than `NOT`, matching SQLite's own precedence.
    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not_expr()?;
        while self.peek_ident("AND") {
            self.advance();
            let right = self.parse_not_expr()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `NOT expr` (right-recursive, so `NOT NOT x` parses) or a plain
    /// boolean primary.
    fn parse_not_expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek_ident("NOT") {
            self.advance();
            let inner = self.parse_not_expr()?;
            Ok(Expr::Not(Box::new(inner)))
        } else {
            self.parse_bool_primary()
        }
    }

    /// A parenthesized boolean sub-expression (`(a = 1 OR b = 2)`) or a
    /// single comparison. Grouping parens are scoped to the boolean
    /// grammar only here — this crate has no general arithmetic/operand
    /// grouping to extend (`parse_operand` never accepted `(` either).
    fn parse_bool_primary(&mut self) -> Result<Expr, ParseError> {
        if self.peek_punct("(") {
            self.advance();
            let inner = self.parse_or_expr()?;
            self.expect_punct(")")?;
            Ok(inner)
        } else {
            self.parse_comparison()
        }
    }

    fn peek_ident_at(&self, offset: usize, keyword: &str) -> bool {
        matches!(self.peek_at(offset), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_operand()?;

        // `NOT LIKE`/`NOT GLOB`/`NOT BETWEEN`/`NOT IN` -- infix negation,
        // distinct from the prefix `NOT` `parse_not_expr` already handles
        // (that one wraps a whole boolean primary; this one only fires
        // mid-comparison, so there's no ambiguity between the two).
        let negate = self.peek_ident("NOT")
            && (self.peek_ident_at(1, "LIKE")
                || self.peek_ident_at(1, "GLOB")
                || self.peek_ident_at(1, "BETWEEN")
                || self.peek_ident_at(1, "IN"));
        if negate {
            self.advance();
        }

        if self.peek_ident("LIKE") {
            self.advance();
            let pattern = self.parse_operand()?;
            let escape = if self.peek_ident("ESCAPE") {
                self.advance();
                Some(Box::new(self.parse_operand()?))
            } else {
                None
            };
            return Ok(Expr::Like {
                left: Box::new(left),
                pattern: Box::new(pattern),
                escape,
                negate,
            });
        }
        if self.peek_ident("GLOB") {
            self.advance();
            let pattern = self.parse_operand()?;
            return Ok(Expr::Glob {
                left: Box::new(left),
                pattern: Box::new(pattern),
                negate,
            });
        }
        if self.peek_ident("BETWEEN") {
            self.advance();
            let low = self.parse_operand()?;
            self.expect_ident("AND")?;
            let high = self.parse_operand()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
                negate,
            });
        }
        if self.peek_ident("IN") {
            self.advance();
            self.expect_punct("(")?;
            let mut list = Vec::new();
            if !self.peek_punct(")") {
                loop {
                    list.push(self.parse_operand()?);
                    if self.peek_punct(",") {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            self.expect_punct(")")?;
            return Ok(Expr::InList {
                expr: Box::new(left),
                list,
                negate,
            });
        }

        let op = match self.advance() {
            Some(Token::Punct("=")) => BinaryOp::Eq,
            Some(Token::Punct("<>")) | Some(Token::Punct("!=")) => BinaryOp::NotEq,
            Some(Token::Punct("<")) => BinaryOp::Lt,
            Some(Token::Punct("<=")) => BinaryOp::LtEq,
            Some(Token::Punct(">")) => BinaryOp::Gt,
            Some(Token::Punct(">=")) => BinaryOp::GtEq,
            Some(Token::Eof) | None => return Err(ParseError::UnexpectedEof),
            Some(other) => return Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        };
        let right = self.parse_operand()?;
        Ok(Expr::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_operand(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::Ident(name)) if name.eq_ignore_ascii_case("CASE") => {
                self.advance();
                self.parse_case_expr()
            }
            Some(Token::Ident(name)) => {
                self.advance();
                if self.peek_punct("(") {
                    self.advance();
                    let mut args = Vec::new();
                    if !self.peek_punct(")") {
                        loop {
                            args.push(self.parse_operand()?);
                            if self.peek_punct(",") {
                                self.advance();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_punct(")")?;
                    Ok(Expr::FunctionCall { name, args })
                } else {
                    Ok(Expr::Column(name))
                }
            }
            Some(Token::Integer(n)) => {
                self.advance();
                Ok(Expr::Literal(Value::Integer(n)))
            }
            Some(Token::Real(f)) => {
                self.advance();
                Ok(Expr::Literal(Value::Real(f)))
            }
            Some(Token::String(s)) => {
                self.advance();
                Ok(Expr::Literal(Value::Text(s)))
            }
            Some(Token::Blob(b)) => {
                self.advance();
                Ok(Expr::Literal(Value::Blob(b)))
            }
            Some(Token::Param(spec)) => {
                self.advance();
                Ok(Expr::Parameter(parse_param_marker(&spec)))
            }
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    /// Parses a `CASE` expression's body — the leading `CASE` keyword is
    /// already consumed. Simple form (`CASE operand WHEN val THEN
    /// ...`) if the next token isn't `WHEN`; searched form (`CASE WHEN
    /// cond THEN ...`, `cond` a full boolean expression) otherwise.
    fn parse_case_expr(&mut self) -> Result<Expr, ParseError> {
        let operand = if self.peek_ident("WHEN") {
            None
        } else {
            Some(Box::new(self.parse_operand()?))
        };

        let mut branches = Vec::new();
        while self.peek_ident("WHEN") {
            self.advance();
            let cond = if operand.is_some() {
                self.parse_operand()?
            } else {
                self.parse_or_expr()?
            };
            self.expect_ident("THEN")?;
            let result = self.parse_operand()?;
            branches.push((cond, result));
        }
        if branches.is_empty() {
            return Err(ParseError::UnexpectedToken(
                "CASE needs at least one WHEN branch".to_string(),
            ));
        }

        let else_result = if self.peek_ident("ELSE") {
            self.advance();
            Some(Box::new(self.parse_operand()?))
        } else {
            None
        };
        self.expect_ident("END")?;

        Ok(Expr::Case {
            operand,
            branches,
            else_result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    #[test]
    fn parses_select_star() {
        let tokens = tokenize("SELECT * FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.columns, SelectColumns::All);
        assert_eq!(select.table_name, "t");
        assert_eq!(select.filter, None);
        assert!(!select.distinct);
    }

    #[test]
    fn parses_select_distinct_star() {
        let tokens = tokenize("SELECT DISTINCT * FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.distinct);
        assert_eq!(select.columns, SelectColumns::All);
    }

    #[test]
    fn parses_select_distinct_named_columns() {
        let tokens = tokenize("SELECT DISTINCT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert!(select.distinct);
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_named_columns() {
        let tokens = tokenize("SELECT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_where_clause() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Column("a".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_not_eq_operator() {
        let tokens = tokenize("SELECT * FROM t WHERE a <> 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::NotEq,
                left: Box::new(Expr::Column("a".into())),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn missing_from_keyword_is_an_error() {
        let tokens = tokenize("SELECT a t").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn parses_function_call_in_where() {
        let tokens = tokenize("SELECT * FROM t WHERE UPPER(name) = 'X'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::FunctionCall {
                    name: "UPPER".into(),
                    args: vec![Expr::Column("name".into())],
                }),
                right: Box::new(Expr::Literal(Value::Text("X".into()))),
            })
        );
    }

    #[test]
    fn parses_function_call_with_multiple_args() {
        let tokens = tokenize("SELECT * FROM t WHERE ADD(a, 1) = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *left,
            Expr::FunctionCall {
                name: "ADD".into(),
                args: vec![Expr::Column("a".into()), Expr::Literal(Value::Integer(1))],
            }
        );
    }

    #[test]
    fn parses_function_call_with_no_args() {
        let tokens = tokenize("SELECT * FROM t WHERE RANDOM() = 0").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        assert_eq!(
            *left,
            Expr::FunctionCall {
                name: "RANDOM".into(),
                args: vec![],
            }
        );
    }

    #[test]
    fn parses_count_star() {
        let tokens = tokenize("SELECT COUNT(*) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Aggregates(vec![AggregateCall {
                name: "COUNT".into(),
                arg: AggregateArg::Star,
            }])
        );
    }

    #[test]
    fn parses_multiple_aggregate_calls() {
        let tokens = tokenize("SELECT SUM(a), MAX(b) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Aggregates(vec![
                AggregateCall {
                    name: "SUM".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                },
                AggregateCall {
                    name: "MAX".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("b".into()))),
                },
            ])
        );
    }

    #[test]
    fn star_arg_only_valid_for_count() {
        let tokens = tokenize("SELECT SUM(*) FROM t").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn plain_column_list_still_parses_as_named() {
        let tokens = tokenize("SELECT a, b FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Named(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn parses_window_call_with_partition_by() {
        let tokens = tokenize("SELECT SUM(a) OVER (PARTITION BY b) FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![WindowCall {
                name: "SUM".into(),
                arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                partition_by: vec!["b".into()],
            }])
        );
    }

    #[test]
    fn parses_window_call_with_no_partition() {
        let tokens = tokenize("SELECT COUNT(*) OVER () FROM t").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![WindowCall {
                name: "COUNT".into(),
                arg: AggregateArg::Star,
                partition_by: vec![],
            }])
        );
    }

    #[test]
    fn parses_multiple_window_calls_and_multi_column_partition() {
        let tokens = tokenize(
            "SELECT SUM(a) OVER (PARTITION BY b, c), COUNT(*) OVER (PARTITION BY b) FROM t",
        )
        .unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.columns,
            SelectColumns::Window(vec![
                WindowCall {
                    name: "SUM".into(),
                    arg: AggregateArg::Expr(Box::new(Expr::Column("a".into()))),
                    partition_by: vec!["b".into(), "c".into()],
                },
                WindowCall {
                    name: "COUNT".into(),
                    arg: AggregateArg::Star,
                    partition_by: vec!["b".into()],
                },
            ])
        );
    }

    fn eq(col: &str, n: i64) -> Expr {
        Expr::BinaryOp {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Column(col.into())),
            right: Box::new(Expr::Literal(Value::Integer(n))),
        }
    }

    #[test]
    fn parses_and() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 AND b = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(Box::new(eq("a", 1)), Box::new(eq("b", 2))))
        );
    }

    #[test]
    fn parses_or() {
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 OR b = 2").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Or(Box::new(eq("a", 1)), Box::new(eq("b", 2))))
        );
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // a=1 OR (b=2 AND c=3), not (a=1 OR b=2) AND c=3.
        let tokens = tokenize("SELECT * FROM t WHERE a = 1 OR b = 2 AND c = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Or(
                Box::new(eq("a", 1)),
                Box::new(Expr::And(Box::new(eq("b", 2)), Box::new(eq("c", 3)))),
            ))
        );
    }

    #[test]
    fn parses_not() {
        let tokens = tokenize("SELECT * FROM t WHERE NOT a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.filter, Some(Expr::Not(Box::new(eq("a", 1)))));
    }

    #[test]
    fn parses_double_not() {
        let tokens = tokenize("SELECT * FROM t WHERE NOT NOT a = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Not(Box::new(Expr::Not(Box::new(eq("a", 1))))))
        );
    }

    #[test]
    fn parses_parenthesized_grouping() {
        // (a=1 OR b=2) AND c=3 -- parens override AND-before-OR precedence.
        let tokens = tokenize("SELECT * FROM t WHERE (a = 1 OR b = 2) AND c = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::Or(Box::new(eq("a", 1)), Box::new(eq("b", 2)))),
                Box::new(eq("c", 3)),
            ))
        );
    }

    #[test]
    fn parses_nested_parens() {
        let tokens = tokenize("SELECT * FROM t WHERE ((a = 1))").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(select.filter, Some(eq("a", 1)));
    }

    #[test]
    fn missing_closing_paren_is_an_error() {
        let tokens = tokenize("SELECT * FROM t WHERE (a = 1").unwrap();
        assert!(matches!(
            parse_select(&tokens),
            Err(ParseError::UnexpectedEof)
        ));
    }

    #[test]
    fn parses_like() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: false,
            })
        );
    }

    #[test]
    fn parses_like_with_escape() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a\\%' ESCAPE '\\'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a\\%".into()))),
                escape: Some(Box::new(Expr::Literal(Value::Text("\\".into())))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_like() {
        let tokens = tokenize("SELECT * FROM t WHERE name NOT LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: true,
            })
        );
    }

    #[test]
    fn parses_glob() {
        let tokens = tokenize("SELECT * FROM t WHERE name GLOB 'a*'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Glob {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a*".into()))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_glob() {
        let tokens = tokenize("SELECT * FROM t WHERE name NOT GLOB 'a*'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Glob {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a*".into()))),
                negate: true,
            })
        );
    }

    #[test]
    fn parses_between() {
        let tokens = tokenize("SELECT * FROM t WHERE a BETWEEN 1 AND 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Between {
                expr: Box::new(Expr::Column("a".into())),
                low: Box::new(Expr::Literal(Value::Integer(1))),
                high: Box::new(Expr::Literal(Value::Integer(10))),
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_between() {
        let tokens = tokenize("SELECT * FROM t WHERE a NOT BETWEEN 1 AND 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Between {
                expr: Box::new(Expr::Column("a".into())),
                low: Box::new(Expr::Literal(Value::Integer(1))),
                high: Box::new(Expr::Literal(Value::Integer(10))),
                negate: true,
            })
        );
    }

    #[test]
    fn like_combines_with_and() {
        let tokens = tokenize("SELECT * FROM t WHERE name LIKE 'a%' AND b = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::Like {
                    left: Box::new(Expr::Column("name".into())),
                    pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                    escape: None,
                    negate: false,
                }),
                Box::new(eq("b", 1)),
            ))
        );
    }

    #[test]
    fn prefix_not_still_wraps_a_like_comparison() {
        // `WHERE NOT name LIKE 'a%'` -- prefix NOT (boolean-level),
        // distinct from `NOT LIKE` (infix, per-operator negation).
        let tokens = tokenize("SELECT * FROM t WHERE NOT name LIKE 'a%'").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::Not(Box::new(Expr::Like {
                left: Box::new(Expr::Column("name".into())),
                pattern: Box::new(Expr::Literal(Value::Text("a%".into()))),
                escape: None,
                negate: false,
            })))
        );
    }

    #[test]
    fn parses_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN (1, 2, 3)").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![
                    Expr::Literal(Value::Integer(1)),
                    Expr::Literal(Value::Integer(2)),
                    Expr::Literal(Value::Integer(3)),
                ],
                negate: false,
            })
        );
    }

    #[test]
    fn parses_not_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a NOT IN (1, 2)").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![
                    Expr::Literal(Value::Integer(1)),
                    Expr::Literal(Value::Integer(2)),
                ],
                negate: true,
            })
        );
    }

    #[test]
    fn parses_empty_in_list() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN ()").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::InList {
                expr: Box::new(Expr::Column("a".into())),
                list: vec![],
                negate: false,
            })
        );
    }

    #[test]
    fn in_list_combines_with_and() {
        let tokens = tokenize("SELECT * FROM t WHERE a IN (1, 2) AND b = 3").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::And(
                Box::new(Expr::InList {
                    expr: Box::new(Expr::Column("a".into())),
                    list: vec![
                        Expr::Literal(Value::Integer(1)),
                        Expr::Literal(Value::Integer(2)),
                    ],
                    negate: false,
                }),
                Box::new(eq("b", 3)),
            ))
        );
    }

    #[test]
    fn parses_searched_case_in_where() {
        let tokens =
            tokenize("SELECT * FROM t WHERE CASE WHEN a = 1 THEN 1 ELSE 0 END = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Case {
                    operand: None,
                    branches: vec![(eq("a", 1), Expr::Literal(Value::Integer(1)))],
                    else_result: Some(Box::new(Expr::Literal(Value::Integer(0)))),
                }),
                right: Box::new(Expr::Literal(Value::Integer(1))),
            })
        );
    }

    #[test]
    fn parses_searched_case_with_multiple_when_branches() {
        let tokens = tokenize(
            "SELECT * FROM t WHERE CASE WHEN a = 1 THEN 10 WHEN a = 2 THEN 20 ELSE 0 END = 10",
        )
        .unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        let Expr::Case { branches, .. } = *left else {
            panic!("expected Case");
        };
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn parses_simple_case() {
        let tokens =
            tokenize("SELECT * FROM t WHERE CASE a WHEN 1 THEN 10 ELSE 0 END = 10").unwrap();
        let select = parse_select(&tokens).unwrap();
        assert_eq!(
            select.filter,
            Some(Expr::BinaryOp {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Case {
                    operand: Some(Box::new(Expr::Column("a".into()))),
                    branches: vec![(
                        Expr::Literal(Value::Integer(1)),
                        Expr::Literal(Value::Integer(10))
                    )],
                    else_result: Some(Box::new(Expr::Literal(Value::Integer(0)))),
                }),
                right: Box::new(Expr::Literal(Value::Integer(10))),
            })
        );
    }

    #[test]
    fn parses_case_with_no_else() {
        let tokens = tokenize("SELECT * FROM t WHERE CASE WHEN a = 1 THEN 1 END = 1").unwrap();
        let select = parse_select(&tokens).unwrap();
        let Some(Expr::BinaryOp { left, .. }) = select.filter else {
            panic!("expected BinaryOp");
        };
        let Expr::Case { else_result, .. } = *left else {
            panic!("expected Case");
        };
        assert_eq!(else_result, None);
    }

    #[test]
    fn case_with_no_when_branches_is_an_error() {
        let tokens = tokenize("SELECT * FROM t WHERE CASE ELSE 1 END = 1").unwrap();
        assert!(parse_select(&tokens).is_err());
    }
}
