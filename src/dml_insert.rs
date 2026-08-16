//! SQL parser: `INSERT` (DML subset, foundation-tier `A4a`). Each `VALUES`
//! slot is a literal or a bound-parameter marker — no other expressions
//! (that's `A6`, the expression evaluator, and `SELECT`'s `WHERE` clause).
//! Grammar reference: <https://www.sqlite.org/lang_insert.html>.

use crate::ddl::ParseError;
use crate::dml_select::{parse_param_marker, Expr};
use crate::token::Token;
use crate::value::Value;

/// `INSERT OR REPLACE`/`INSERT OR IGNORE`'s conflict-resolution mode
/// (issue #123) — the only two of SQLite's five `OR`-clause modes that
/// change observable behavior once constraint enforcement is in place
/// (`OR ABORT`/`OR FAIL`/`OR ROLLBACK` all reduce to this crate's
/// existing hard-error-on-conflict behavior, so [`parse_insert`] accepts
/// that syntax but parses it as a plain `INSERT` — see its own doc
/// comment). Only `PRIMARY KEY`/`UNIQUE` conflicts are subject to either
/// mode; a `NOT NULL`/`CHECK` violation still errors regardless, per
/// this issue's own narrower acceptance scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrConflict {
    /// A conflicting existing row (by `PRIMARY KEY`/`UNIQUE`) is deleted
    /// before the new row is inserted.
    Replace,
    /// A row that would conflict (by `PRIMARY KEY`/`UNIQUE`) is silently
    /// skipped — not inserted, no error, doesn't count toward the
    /// statement's affected-row count.
    Ignore,
}

/// A parsed `INSERT INTO ... VALUES (...)` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Insert {
    pub table_name: String,
    /// Explicit column list, if given (`INSERT INTO t (a, b) VALUES ...`).
    /// `None` means "all columns, in table-definition order" (unresolvable
    /// without a catalog — left to the caller until `A5` exists).
    pub columns: Option<Vec<String>>,
    /// Each `VALUES` slot, as [`Expr::Literal`] or [`Expr::Parameter`]
    /// (never any other `Expr` variant — this parser only ever produces
    /// those two) — see `docs/adr/0002-parameter-markers.md`.
    pub rows: Vec<Vec<Expr>>,
    /// `OR REPLACE`/`OR IGNORE`, if given (issue #123) — see
    /// [`OrConflict`]'s own doc comment.
    pub or_conflict: Option<OrConflict>,
}

/// Parses an `INSERT [OR REPLACE|OR IGNORE|OR ABORT|OR FAIL|OR ROLLBACK]
/// INTO ... VALUES (...)` statement from a token stream (as produced by
/// [`crate::tokenize`]).
pub fn parse_insert(tokens: &[Token]) -> Result<Insert, ParseError> {
    let mut p = InsertParser { tokens, pos: 0 };
    p.expect_ident("INSERT")?;

    let or_conflict = if p.peek_ident("OR") {
        p.advance();
        if p.peek_ident("REPLACE") {
            p.advance();
            Some(OrConflict::Replace)
        } else if p.peek_ident("IGNORE") {
            p.advance();
            Some(OrConflict::Ignore)
        } else if p.peek_ident("ABORT") || p.peek_ident("FAIL") || p.peek_ident("ROLLBACK") {
            // Deferred (epic #111 Part 3) -- accepted syntactically,
            // parsed as a plain INSERT (see OrConflict's doc comment).
            p.advance();
            None
        } else {
            match p.advance() {
                Some(Token::Eof) | None => return Err(ParseError::UnexpectedEof),
                Some(other) => return Err(ParseError::UnexpectedToken(format!("{other:?}"))),
            }
        }
    } else {
        None
    };

    p.expect_ident("INTO")?;
    let table_name = p.expect_any_ident()?;

    let columns = if p.peek_punct("(") {
        p.advance();
        let mut cols = Vec::new();
        loop {
            cols.push(p.expect_any_ident()?);
            if p.peek_punct(",") {
                p.advance();
                continue;
            }
            p.expect_punct(")")?;
            break;
        }
        Some(cols)
    } else {
        None
    };

    p.expect_ident("VALUES")?;

    let mut rows = Vec::new();
    loop {
        p.expect_punct("(")?;
        let mut row = Vec::new();
        loop {
            row.push(p.parse_value_or_param()?);
            if p.peek_punct(",") {
                p.advance();
                continue;
            }
            p.expect_punct(")")?;
            break;
        }
        rows.push(row);
        if p.peek_punct(",") {
            p.advance();
            continue;
        }
        break;
    }

    Ok(Insert {
        table_name,
        columns,
        rows,
        or_conflict,
    })
}

struct InsertParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> InsertParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
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

    fn parse_value_or_param(&mut self) -> Result<Expr, ParseError> {
        match self.advance() {
            Some(Token::Integer(n)) => Ok(Expr::Literal(Value::Integer(*n))),
            Some(Token::Real(f)) => Ok(Expr::Literal(Value::Real(*f))),
            Some(Token::String(s)) => Ok(Expr::Literal(Value::Text(s.clone()))),
            Some(Token::Blob(b)) => Ok(Expr::Literal(Value::Blob(b.clone()))),
            Some(Token::Ident(s)) if s.eq_ignore_ascii_case("NULL") => {
                Ok(Expr::Literal(Value::Null))
            }
            Some(Token::Param(spec)) => Ok(Expr::Parameter(parse_param_marker(spec))),
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
    fn parses_insert_without_column_list() {
        let tokens = tokenize("INSERT INTO t VALUES (1, 'x', NULL)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.table_name, "t");
        assert_eq!(insert.columns, None);
        assert_eq!(
            insert.rows,
            vec![vec![
                Expr::Literal(Value::Integer(1)),
                Expr::Literal(Value::Text("x".into())),
                Expr::Literal(Value::Null)
            ]]
        );
    }

    #[test]
    fn parses_insert_with_column_list() {
        let tokens = tokenize("INSERT INTO t (a, b) VALUES (1, 2)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.columns, Some(vec!["a".into(), "b".into()]));
        assert_eq!(
            insert.rows,
            vec![vec![
                Expr::Literal(Value::Integer(1)),
                Expr::Literal(Value::Integer(2))
            ]]
        );
    }

    #[test]
    fn parses_multiple_value_rows() {
        let tokens = tokenize("INSERT INTO t VALUES (1), (2), (3)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.rows.len(), 3);
    }

    #[test]
    fn plain_insert_has_no_or_conflict() {
        let tokens = tokenize("INSERT INTO t VALUES (1)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.or_conflict, None);
    }

    #[test]
    fn parses_insert_or_replace() {
        let tokens = tokenize("INSERT OR REPLACE INTO t VALUES (1)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.or_conflict, Some(OrConflict::Replace));
        assert_eq!(insert.table_name, "t");
    }

    #[test]
    fn parses_insert_or_ignore() {
        let tokens = tokenize("INSERT OR IGNORE INTO t VALUES (1)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(insert.or_conflict, Some(OrConflict::Ignore));
    }

    #[test]
    fn parses_deferred_or_clauses_as_a_plain_insert() {
        for keyword in ["ABORT", "FAIL", "ROLLBACK"] {
            let tokens = tokenize(&format!("INSERT OR {keyword} INTO t VALUES (1)")).unwrap();
            let insert = parse_insert(&tokens).unwrap();
            assert_eq!(insert.or_conflict, None);
        }
    }

    #[test]
    fn unrecognized_or_clause_is_an_error() {
        let tokens = tokenize("INSERT OR BOGUS INTO t VALUES (1)").unwrap();
        assert!(matches!(
            parse_insert(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn missing_values_keyword_is_an_error() {
        let tokens = tokenize("INSERT INTO t (1)").unwrap();
        assert!(matches!(
            parse_insert(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn unterminated_row_is_an_error() {
        let tokens = tokenize("INSERT INTO t VALUES (1, 2").unwrap();
        assert_eq!(parse_insert(&tokens), Err(ParseError::UnexpectedEof));
    }

    #[test]
    fn parses_anonymous_and_named_parameter_markers() {
        let tokens = tokenize("INSERT INTO t VALUES (?, :name)").unwrap();
        let insert = parse_insert(&tokens).unwrap();
        assert_eq!(
            insert.rows,
            vec![vec![
                Expr::Parameter(crate::dml_select::ParamMarker::Anonymous),
                Expr::Parameter(crate::dml_select::ParamMarker::Named(":name".into())),
            ]]
        );
    }
}
