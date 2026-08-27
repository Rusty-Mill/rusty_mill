//! SQL parser: `UPDATE` (issue #128). `UPDATE table SET col = expr, ...
//! [WHERE ...]` — single table, no `FROM` clause (SQLite's `UPDATE...FROM`
//! extension is explicitly out of scope for this issue). Grammar
//! reference: <https://www.sqlite.org/lang_update.html>.

use crate::ddl::ParseError;
use crate::dml_select::{parse_expr_at, parse_operand_at, Expr};
use crate::token::Token;

/// A parsed `UPDATE ...` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub table_name: String,
    /// `SET col = expr` pairs, in the order they appear in the SQL text.
    /// A later assignment to the same column simply overwrites the
    /// earlier one at execution time (last-one-wins, matching real
    /// SQLite) — this parser doesn't itself dedup them.
    pub assignments: Vec<(String, Expr)>,
    /// `WHERE ...`, if given. `None` matches every row, same as a
    /// `WHERE`-less `SELECT`/`DELETE`.
    pub filter: Option<Expr>,
}

/// Parses an `UPDATE table SET col = expr [, ...] [WHERE ...]` statement
/// from a token stream (as produced by [`crate::tokenize`]).
pub fn parse_update(tokens: &[Token]) -> Result<Update, ParseError> {
    let mut p = UpdateParser { tokens, pos: 0 };
    p.expect_ident("UPDATE")?;
    let table_name = p.expect_any_ident()?;
    p.expect_ident("SET")?;

    let mut assignments = Vec::new();
    loop {
        let column = p.expect_any_ident()?;
        p.expect_punct("=")?;
        // A `SET` value is a plain value expression (literal, column,
        // function call, `CASE`, parameter) -- not itself a boolean
        // filter, so this uses the same narrower grammar `DEFAULT`
        // column constraints do ([`parse_operand_at`]), not the full
        // comparison-chain grammar `WHERE` uses.
        let (expr, new_pos) = parse_operand_at(p.tokens, p.pos)?;
        p.pos = new_pos;
        assignments.push((column, expr));

        if p.peek_punct(",") {
            p.advance();
            continue;
        }
        break;
    }

    let filter = if p.peek_ident("WHERE") {
        p.advance();
        let (expr, _) = parse_expr_at(p.tokens, p.pos)?;
        Some(expr)
    } else {
        None
    };

    Ok(Update {
        table_name,
        assignments,
        filter,
    })
}

struct UpdateParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> UpdateParser<'a> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;
    use crate::value::Value;

    #[test]
    fn parses_update_with_single_assignment_and_no_where() {
        let tokens = tokenize("UPDATE t SET a = 1").unwrap();
        let update = parse_update(&tokens).unwrap();
        assert_eq!(update.table_name, "t");
        assert_eq!(
            update.assignments,
            vec![("a".to_string(), Expr::Literal(Value::Integer(1)))]
        );
        assert_eq!(update.filter, None);
    }

    #[test]
    fn parses_update_with_multiple_assignments() {
        let tokens = tokenize("UPDATE t SET a = 1, b = 'x'").unwrap();
        let update = parse_update(&tokens).unwrap();
        assert_eq!(
            update.assignments,
            vec![
                ("a".to_string(), Expr::Literal(Value::Integer(1))),
                ("b".to_string(), Expr::Literal(Value::Text("x".into()))),
            ]
        );
    }

    #[test]
    fn parses_update_with_where_clause() {
        let tokens = tokenize("UPDATE t SET a = 1 WHERE b = 2").unwrap();
        let update = parse_update(&tokens).unwrap();
        assert!(update.filter.is_some());
    }

    #[test]
    fn set_value_can_reference_a_column() {
        let tokens = tokenize("UPDATE t SET a = b").unwrap();
        let update = parse_update(&tokens).unwrap();
        assert_eq!(
            update.assignments,
            vec![("a".to_string(), Expr::Column("b".to_string()))]
        );
    }

    #[test]
    fn set_value_can_be_a_bound_parameter() {
        let tokens = tokenize("UPDATE t SET a = ?").unwrap();
        let update = parse_update(&tokens).unwrap();
        assert_eq!(
            update.assignments,
            vec![(
                "a".to_string(),
                Expr::Parameter(crate::dml_select::ParamMarker::Anonymous)
            )]
        );
    }

    #[test]
    fn missing_set_keyword_is_an_error() {
        let tokens = tokenize("UPDATE t a = 1").unwrap();
        assert!(matches!(
            parse_update(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn unterminated_update_is_an_error() {
        let tokens = tokenize("UPDATE t SET a =").unwrap();
        assert!(parse_update(&tokens).is_err());
    }
}
