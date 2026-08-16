//! SQL parser: `CREATE TABLE` (DDL subset, foundation-tier `A3`). No other
//! statement types live here yet — see `A4a`/`A4b` for `INSERT`/`SELECT`.
//! Grammar reference: <https://www.sqlite.org/lang_createtable.html>.

use crate::dml_select::{parse_expr_at, parse_operand_at, Expr};
use crate::token::Token;
use crate::value::Value;

/// A parsed `CREATE TABLE` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateTable {
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
}

/// A single column definition within a `CREATE TABLE`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnDef {
    pub name: String,
    /// The declared type name, if any (SQLite's type affinity is inferred
    /// from this string; not resolved to an affinity here).
    pub type_name: Option<String>,
    pub primary_key: bool,
    pub not_null: bool,
    /// `UNIQUE` (issue #117). Enforcement is a separate sub-issue (#118)
    /// — this crate only parses and stores the flag here.
    pub unique: bool,
    /// `AUTOINCREMENT`, only accepted directly after `PRIMARY KEY` on an
    /// `INTEGER`-typed column (issue #117) — matching real SQLite's own
    /// restriction, enforced here at parse time rather than accepted
    /// leniently. See [`Parser::parse_column_def`].
    pub autoincrement: bool,
    /// `CHECK(expr)` (issue #117). Parsed with the same expression
    /// grammar as `WHERE` (via [`parse_expr_at`]); enforcement at
    /// insert/update time is out of scope (#118).
    pub check: Option<Expr>,
    /// `DEFAULT value` (issue #117) — a literal, a signed numeric
    /// literal, or a parenthesized expression. Applying this at insert
    /// time is out of scope (#118); this crate only parses and stores it.
    pub default: Option<Expr>,
    /// `REFERENCES table [(column)]` (issue #117), the column-constraint
    /// form of a foreign key. Enforcement (and the table-level `FOREIGN
    /// KEY (...) REFERENCES ...` form) is out of scope here.
    pub references: Option<ForeignKeyRef>,
}

/// A parsed `REFERENCES table [(column)]` column constraint — see
/// [`ColumnDef::references`].
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignKeyRef {
    pub table: String,
    /// The referenced column, if given explicitly. `None` when the
    /// reference omits it (SQLite then uses the referenced table's own
    /// primary key) — this crate doesn't resolve that omission to a
    /// concrete column name, it just records that none was written.
    pub column: Option<String>,
}

/// A parsed `CREATE VIRTUAL TABLE table_name USING module_name(args...)`
/// statement (issue #93 — see `docs/gap-analysis-vtab.md`).
///
/// **Argument-text fidelity, stated plainly:** each argument is a
/// reconstruction of its source tokens (joined with single spaces), not
/// an exact byte-for-byte slice of the original text. Real SQLite
/// preserves exact module-argument text (original whitespace/quoting
/// included); this crate's parser works purely on the tokenized stream
/// — like every other parser here — and has no raw-text span to slice
/// from. Semantically equivalent for the `dequote`/`parameter`/
/// `parse_boolean` helpers (`crate::vtab`) a module uses to interpret
/// its arguments, just not byte-identical.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateVirtualTable {
    pub table_name: String,
    pub module_name: String,
    pub args: Vec<String>,
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

/// Parses a `CREATE VIRTUAL TABLE table_name USING module_name(args...)`
/// statement from a token stream. The `(args...)` list is optional
/// (omitted entirely when the module takes none).
pub fn parse_create_virtual_table(tokens: &[Token]) -> Result<CreateVirtualTable, ParseError> {
    let mut p = Parser { tokens, pos: 0 };
    p.expect_ident("CREATE")?;
    p.expect_ident("VIRTUAL")?;
    p.expect_ident("TABLE")?;
    let table_name = p.expect_any_ident()?;
    p.expect_ident("USING")?;
    let module_name = p.expect_any_ident()?;

    let mut args = Vec::new();
    if matches!(p.peek(), Some(Token::Punct("("))) {
        p.advance();
        if !matches!(p.peek(), Some(Token::Punct(")"))) {
            loop {
                args.push(p.parse_module_arg()?);
                if matches!(p.peek(), Some(Token::Punct(","))) {
                    p.advance();
                    continue;
                }
                break;
            }
        }
        p.expect_punct(")")?;
    }

    Ok(CreateVirtualTable {
        table_name,
        module_name,
        args,
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

    fn peek_ident(&self, keyword: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case(keyword))
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
        let mut unique = false;
        let mut autoincrement = false;
        let mut check = None;
        let mut default = None;
        let mut references = None;
        loop {
            match self.peek() {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("PRIMARY") => {
                    self.advance();
                    self.expect_ident("KEY")?;
                    primary_key = true;
                    if self.peek_ident("AUTOINCREMENT") {
                        self.advance();
                        autoincrement = true;
                    }
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("NOT") => {
                    self.advance();
                    self.expect_ident("NULL")?;
                    not_null = true;
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("UNIQUE") => {
                    self.advance();
                    unique = true;
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("CHECK") => {
                    self.advance();
                    self.expect_punct("(")?;
                    let (expr, new_pos) = parse_expr_at(self.tokens, self.pos)?;
                    self.pos = new_pos;
                    self.expect_punct(")")?;
                    check = Some(expr);
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("DEFAULT") => {
                    self.advance();
                    default = Some(self.parse_default_value()?);
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("REFERENCES") => {
                    self.advance();
                    let table = self.expect_any_ident()?;
                    let column = if matches!(self.peek(), Some(Token::Punct("("))) {
                        self.advance();
                        let col = self.expect_any_ident()?;
                        self.expect_punct(")")?;
                        Some(col)
                    } else {
                        None
                    };
                    references = Some(ForeignKeyRef { table, column });
                }
                // `AUTOINCREMENT` only appears immediately after `PRIMARY
                // KEY` above; seeing it here means it showed up on its
                // own, which real SQLite also rejects.
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("AUTOINCREMENT") => {
                    return Err(ParseError::UnexpectedToken(
                        "AUTOINCREMENT must follow PRIMARY KEY".to_string(),
                    ));
                }
                _ => break,
            }
        }

        if autoincrement
            && !type_name
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("INTEGER"))
        {
            return Err(ParseError::UnexpectedToken(
                "AUTOINCREMENT requires an INTEGER PRIMARY KEY column".to_string(),
            ));
        }

        Ok(ColumnDef {
            name,
            type_name,
            primary_key,
            not_null,
            unique,
            autoincrement,
            check,
            default,
            references,
        })
    }

    /// Parses a `DEFAULT` value — SQLite's `signed-number | literal-value
    /// | (expr)` grammar. A bare value (`DEFAULT 1`) has no comparison
    /// operator, so this uses [`parse_operand_at`] (a single primary
    /// expression) rather than [`parse_expr_at`] (which requires one, by
    /// design — see its own doc comment). Unary minus (`DEFAULT -1`) is
    /// handled directly, since the shared expression grammar has no
    /// unary-minus operator; a parenthesized value (`DEFAULT ('x')`) is
    /// unwrapped by recursing, so nesting (`DEFAULT ((-1))`) also works.
    fn parse_default_value(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Token::Punct("-"))) {
            self.advance();
            return match self.advance() {
                Some(Token::Integer(n)) => Ok(Expr::Literal(Value::Integer(-n))),
                Some(Token::Real(f)) => Ok(Expr::Literal(Value::Real(-f))),
                Some(Token::Eof) | None => Err(ParseError::UnexpectedEof),
                Some(other) => Err(ParseError::UnexpectedToken(format!("{other:?}"))),
            };
        }
        if matches!(self.peek(), Some(Token::Punct("("))) {
            self.advance();
            let inner = self.parse_default_value()?;
            self.expect_punct(")")?;
            return Ok(inner);
        }
        let (expr, new_pos) = parse_operand_at(self.tokens, self.pos)?;
        self.pos = new_pos;
        Ok(expr)
    }

    /// Consumes tokens up to (not including) the next top-level `,` or
    /// `)`, reconstructing them into a single source-like string — see
    /// [`CreateVirtualTable`]'s doc comment for the fidelity caveat.
    /// Tracks paren depth so a nested call-shaped argument (e.g.
    /// `foo(1, 2)`) isn't split on its own internal comma.
    fn parse_module_arg(&mut self) -> Result<String, ParseError> {
        let mut depth: i32 = 0;
        let mut parts = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Punct(",")) if depth == 0 => break,
                Some(Token::Punct(")")) if depth == 0 => break,
                Some(Token::Eof) | None => return Err(ParseError::UnexpectedEof),
                Some(Token::Punct("(")) => {
                    depth += 1;
                    parts.push("(".to_string());
                    self.advance();
                }
                Some(Token::Punct(")")) => {
                    depth -= 1;
                    parts.push(")".to_string());
                    self.advance();
                }
                Some(tok) => {
                    parts.push(token_text(tok));
                    self.advance();
                }
            }
        }
        if parts.is_empty() {
            return Err(ParseError::UnexpectedToken(
                "empty module argument".to_string(),
            ));
        }
        Ok(parts.join(" "))
    }
}

/// Reconstructs `tok`'s source-like text — see
/// [`Parser::parse_module_arg`].
fn token_text(tok: &Token) -> String {
    match tok {
        Token::Ident(s) => s.clone(),
        Token::Integer(n) => n.to_string(),
        Token::Real(f) => f.to_string(),
        Token::String(s) => format!("'{}'", s.replace('\'', "''")),
        Token::Blob(b) => {
            let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("X'{hex}'")
        }
        Token::Punct(p) => p.to_string(),
        Token::Param(spec) => format!("?{spec}"),
        Token::Eof => String::new(),
    }
}

fn is_column_constraint_keyword(s: &str) -> bool {
    s.eq_ignore_ascii_case("PRIMARY")
        || s.eq_ignore_ascii_case("NOT")
        || s.eq_ignore_ascii_case("UNIQUE")
        || s.eq_ignore_ascii_case("CHECK")
        || s.eq_ignore_ascii_case("DEFAULT")
        || s.eq_ignore_ascii_case("REFERENCES")
        || s.eq_ignore_ascii_case("AUTOINCREMENT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dml_select::BinaryOp;
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
                    ..Default::default()
                },
                ColumnDef {
                    name: "b".into(),
                    type_name: Some("TEXT".into()),
                    ..Default::default()
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
    fn parses_unique() {
        let tokens = tokenize("CREATE TABLE t (email TEXT UNIQUE)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert!(create.columns[0].unique);
    }

    #[test]
    fn parses_check_constraint() {
        let tokens = tokenize("CREATE TABLE t (age INTEGER CHECK (age >= 0))").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].check,
            Some(Expr::BinaryOp {
                op: BinaryOp::GtEq,
                left: Box::new(Expr::Column("age".into())),
                right: Box::new(Expr::Literal(Value::Integer(0))),
            })
        );
    }

    #[test]
    fn parses_default_literal() {
        let tokens = tokenize("CREATE TABLE t (active INTEGER DEFAULT 1)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].default,
            Some(Expr::Literal(Value::Integer(1)))
        );
    }

    #[test]
    fn parses_default_negative_number() {
        let tokens = tokenize("CREATE TABLE t (balance INTEGER DEFAULT -1)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].default,
            Some(Expr::Literal(Value::Integer(-1)))
        );
    }

    #[test]
    fn parses_default_parenthesized_expression() {
        let tokens = tokenize("CREATE TABLE t (label TEXT DEFAULT ('x'))").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].default,
            Some(Expr::Literal(Value::Text("x".into())))
        );
    }

    #[test]
    fn parses_references_with_explicit_column() {
        let tokens = tokenize("CREATE TABLE t (owner_id INTEGER REFERENCES users(id))").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].references,
            Some(ForeignKeyRef {
                table: "users".into(),
                column: Some("id".into()),
            })
        );
    }

    #[test]
    fn parses_references_without_explicit_column() {
        let tokens = tokenize("CREATE TABLE t (owner_id INTEGER REFERENCES users)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert_eq!(
            create.columns[0].references,
            Some(ForeignKeyRef {
                table: "users".into(),
                column: None,
            })
        );
    }

    #[test]
    fn parses_autoincrement_on_integer_primary_key() {
        let tokens = tokenize("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT)").unwrap();
        let create = parse_create_table(&tokens).unwrap();
        assert!(create.columns[0].primary_key);
        assert!(create.columns[0].autoincrement);
    }

    #[test]
    fn autoincrement_on_a_non_integer_column_is_an_error() {
        let tokens = tokenize("CREATE TABLE t (id TEXT PRIMARY KEY AUTOINCREMENT)").unwrap();
        assert!(matches!(
            parse_create_table(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn autoincrement_without_primary_key_is_an_error() {
        let tokens = tokenize("CREATE TABLE t (id INTEGER AUTOINCREMENT)").unwrap();
        assert!(matches!(
            parse_create_table(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn parses_multiple_constraints_on_one_column() {
        let tokens =
            tokenize("CREATE TABLE t (age INTEGER NOT NULL UNIQUE DEFAULT 0 CHECK (age >= 0))")
                .unwrap();
        let create = parse_create_table(&tokens).unwrap();
        let col = &create.columns[0];
        assert!(col.not_null);
        assert!(col.unique);
        assert_eq!(col.default, Some(Expr::Literal(Value::Integer(0))));
        assert!(col.check.is_some());
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

    #[test]
    fn parses_create_virtual_table_with_args() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING myrange(1, 10, 'x')").unwrap();
        let create = parse_create_virtual_table(&tokens).unwrap();
        assert_eq!(create.table_name, "t");
        assert_eq!(create.module_name, "myrange");
        assert_eq!(create.args, vec!["1", "10", "'x'"]);
    }

    #[test]
    fn parses_create_virtual_table_with_no_args() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING myrange").unwrap();
        let create = parse_create_virtual_table(&tokens).unwrap();
        assert_eq!(create.table_name, "t");
        assert_eq!(create.module_name, "myrange");
        assert!(create.args.is_empty());
    }

    #[test]
    fn parses_create_virtual_table_with_empty_parens() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING myrange()").unwrap();
        let create = parse_create_virtual_table(&tokens).unwrap();
        assert!(create.args.is_empty());
    }

    #[test]
    fn parses_key_value_style_module_args() {
        let tokens =
            tokenize("CREATE VIRTUAL TABLE t USING fts5(content, tokenize = 'porter')").unwrap();
        let create = parse_create_virtual_table(&tokens).unwrap();
        assert_eq!(create.args, vec!["content", "tokenize = 'porter'"]);
    }

    #[test]
    fn nested_parens_in_an_arg_are_not_split_on_their_inner_comma() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING mod(foo(1, 2), 3)").unwrap();
        let create = parse_create_virtual_table(&tokens).unwrap();
        assert_eq!(create.args, vec!["foo ( 1 , 2 )", "3"]);
    }

    #[test]
    fn unterminated_module_arg_list_is_an_error() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING mod(1, 2").unwrap();
        assert_eq!(
            parse_create_virtual_table(&tokens),
            Err(ParseError::UnexpectedEof)
        );
    }

    #[test]
    fn empty_module_arg_is_an_error() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t USING mod(1, , 2)").unwrap();
        assert!(matches!(
            parse_create_virtual_table(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn missing_using_keyword_is_an_error() {
        let tokens = tokenize("CREATE VIRTUAL TABLE t mod(1)").unwrap();
        assert!(matches!(
            parse_create_virtual_table(&tokens),
            Err(ParseError::UnexpectedToken(_))
        ));
    }
}
