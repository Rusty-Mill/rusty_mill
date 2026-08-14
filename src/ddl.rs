//! SQL parser: `CREATE TABLE` (DDL subset, foundation-tier `A3`). No other
//! statement types live here yet — see `A4a`/`A4b` for `INSERT`/`SELECT`.
//! Grammar reference: <https://www.sqlite.org/lang_createtable.html>.

use crate::token::Token;

/// A parsed `CREATE TABLE` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateTable {
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
}

/// A single column definition within a `CREATE TABLE`.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    /// The declared type name, if any (SQLite's type affinity is inferred
    /// from this string; not resolved to an affinity here).
    pub type_name: Option<String>,
    pub primary_key: bool,
    pub not_null: bool,
}

/// An error produced while parsing.
#[derive(Debug, PartialEq)]
pub enum ParseError {
    UnexpectedToken(String),
    UnexpectedEof,
}

/// Parses a `CREATE TABLE` statement from a token stream (as produced by
/// [`crate::tokenize`]).
pub fn parse_create_table(tokens: &[Token]) -> Result<CreateTable, ParseError> {
    let mut p = Parser { tokens, pos: 0 };
    p.expect_ident("CREATE")?;
    p.expect_ident("TABLE")?;
    let table_name = p.expect_any_ident()?;
    p.expect_punct("(")?;

    let mut columns = Vec::new();
    loop {
        columns.push(p.parse_column_def()?);
        match p.peek() {
            Some(Token::Punct(",")) => {
                p.advance();
                continue;
            }
            Some(Token::Punct(")")) => {
                p.advance();
                break;
            }
            Some(Token::Eof) | None => return Err(ParseError::UnexpectedEof),
            Some(other) => return Err(ParseError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    Ok(CreateTable {
        table_name,
        columns,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
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

    fn parse_column_def(&mut self) -> Result<ColumnDef, ParseError> {
        let name = self.expect_any_ident()?;

        let type_name = match self.peek() {
            Some(Token::Ident(s)) if !is_column_constraint_keyword(s) => {
                let t = s.clone();
                self.advance();
                Some(t)
            }
            _ => None,
        };

        let mut primary_key = false;
        let mut not_null = false;
        loop {
            match self.peek() {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("PRIMARY") => {
                    self.advance();
                    self.expect_ident("KEY")?;
                    primary_key = true;
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("NOT") => {
                    self.advance();
                    self.expect_ident("NULL")?;
                    not_null = true;
                }
                _ => break,
            }
        }

        Ok(ColumnDef {
            name,
            type_name,
            primary_key,
            not_null,
        })
    }
}

fn is_column_constraint_keyword(s: &str) -> bool {
    s.eq_ignore_ascii_case("PRIMARY") || s.eq_ignore_ascii_case("NOT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    #[test]
    fn parses_simple_table() {
        let tokens = tokenize("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(create.table_name, "t");
        assert_eq!(
            create.columns,
            vec![
                ColumnDef {
                    name: "a".into(),
                    type_name: Some("INTEGER".into()),
                    primary_key: false,
                    not_null: false,
                },
                ColumnDef {
                    name: "b".into(),
                    type_name: Some("TEXT".into()),
                    primary_key: false,
                    not_null: false,
                },
            ]
        );
    }

    #[test]
    fn parses_primary_key_and_not_null() {
        let tokens =
            tokenize("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert!(create.columns[0].primary_key);
        assert!(create.columns[1].not_null);
    }

    #[test]
    fn parses_column_without_type() {
        let tokens = tokenize("CREATE TABLE t (a)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(create.columns[0].type_name, None);
    }

    #[test]
    fn missing_table_keyword_is_an_error() {
        let tokens = tokenize("CREATE t (a)").unwrap();
        assert!(matches!(
            parse_create_table(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn unterminated_column_list_is_an_error() {
        let tokens = tokenize("CREATE TABLE t (a INTEGER").unwrap();
        assert_eq!(parse_create_table(&tokens), Err(ParseError::UnexpectedEof));
    }
}
