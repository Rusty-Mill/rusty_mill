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
    /// A scalar function call, e.g. `UPPER(name)`. Evaluated only by
    /// `eval::evaluate_with_functions` — plain `evaluate`/`evaluate_bool`
    /// (which predate function-call support) error on this variant rather
    /// than silently treating it as something else.
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
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

    let columns = if p.peek_punct("*") {
        p.advance();
        SelectColumns::All
    } else if p.starts_aggregate_call() {
        let mut calls = vec![p.parse_aggregate_call()?];
        while p.peek_punct(",") {
            p.advance();
            calls.push(p.parse_aggregate_call()?);
        }
        SelectColumns::Aggregates(calls)
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
        Some(p.parse_comparison()?)
    } else {
        None
    };

    Ok(Select {
        columns,
        table_name,
        filter,
    })
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

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_operand()?;
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
            Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
            Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
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
}
