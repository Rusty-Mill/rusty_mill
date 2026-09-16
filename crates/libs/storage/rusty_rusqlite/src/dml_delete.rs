//! SQL parser: `DELETE` (issue #129). `DELETE FROM table [WHERE ...]` —
//! structurally simpler than `UPDATE` (issue #128, right before this one
//! in the epic's own sequencing): no `SET` clause at all. Grammar
//! reference: <https://www.sqlite.org/lang_delete.html>.

use crate::ddl::ParseError;
use crate::dml_select::{parse_expr_at, Expr};
use crate::token::Token;

/// A parsed `DELETE FROM ...` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Delete {
    pub table_name: String,
    /// `WHERE ...`, if given. `None` matches every row, same as a
    /// `WHERE`-less `SELECT`/`UPDATE`.
    pub filter: Option<Expr>,
}

/// Parses a `DELETE FROM table [WHERE ...]` statement from a token
/// stream (as produced by [`crate::tokenize`]).
pub fn parse_delete(tokens: &[Token]) -> Result<Delete, ParseError> {
    let mut p = DeleteParser { tokens, pos: 0 };
    p.expect_ident("DELETE")?;
    p.expect_ident("FROM")?;
    let table_name = p.expect_any_ident()?;

    let filter = if p.peek_ident("WHERE") {
        p.advance();
        let (expr, _) = parse_expr_at(p.tokens, p.pos)?;
        Some(expr)
    } else {
        None
    };

    Ok(Delete { table_name, filter })
}

struct DeleteParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> DeleteParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    #[test]
    fn parses_delete_without_where() {
        let tokens = tokenize("DELETE FROM t").unwrap();
        let delete = parse_delete(&tokens).unwrap();
        assert_eq!(delete.table_name, "t");
        assert_eq!(delete.filter, None);
    }

    #[test]
    fn parses_delete_with_where_clause() {
        let tokens = tokenize("DELETE FROM t WHERE a = 1").unwrap();
        let delete = parse_delete(&tokens).unwrap();
        assert_eq!(delete.table_name, "t");
        assert!(delete.filter.is_some());
    }

    #[test]
    fn missing_from_keyword_is_an_error() {
        let tokens = tokenize("DELETE t").unwrap();
        assert!(matches!(
            parse_delete(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn missing_table_name_is_an_error() {
        let tokens = tokenize("DELETE FROM").unwrap();
        assert_eq!(parse_delete(&tokens), Err(ParseError::UnexpectedEof));
    }
}
