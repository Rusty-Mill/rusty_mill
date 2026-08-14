//! SQL parser: `INSERT` (DML subset, foundation-tier `A4a`). Each `VALUES`
//! slot is a literal or a bound-parameter marker — no other expressions
//! (that's `A6`, the expression evaluator, and `SELECT`'s `WHERE` clause).
//! Grammar reference: <https://www.sqlite.org/lang_insert.html>.

use crate::ddl::ParseError;
use crate::dml_select::{parse_param_marker, Expr};
use crate::token::Token;
use crate::value::Value;

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
}

/// Parses an `INSERT INTO ... VALUES (...)` statement from a token stream
/// (as produced by [`crate::tokenize`]).
pub fn parse_insert(tokens: &[Token]) -> Result<Insert, ParseError> {
    let mut p = InsertParser { tokens, pos: 0 };
    p.expect_ident("INSERT")?;
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
