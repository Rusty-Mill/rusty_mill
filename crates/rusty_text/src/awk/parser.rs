//! Recursive-descent parser over the token stream, lowest to highest
//! precedence: assignment, `||`, `&&`, `~`/`!~`, relational, concatenation,
//! additive, multiplicative, unary, primary.

use super::ast::{BinOp, Expr, LValue, Pattern, Program, Rule, Stmt};
use super::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected {expected:?}, found {:?}", self.peek()))
        }
    }

    fn skip_semicolons(&mut self) {
        while self.peek() == &Token::Semicolon {
            self.advance();
        }
    }

    pub fn parse_program(mut self) -> Result<Program, String> {
        let mut rules = Vec::new();
        self.skip_semicolons();
        while self.peek() != &Token::Eof {
            rules.push(self.parse_rule()?);
            self.skip_semicolons();
        }
        Ok(Program { rules })
    }

    fn parse_rule(&mut self) -> Result<Rule, String> {
        let pattern = match self.peek() {
            Token::Begin => {
                self.advance();
                Pattern::Begin
            }
            Token::End => {
                self.advance();
                Pattern::End
            }
            Token::LBrace => Pattern::Always,
            _ => Pattern::Expr(self.parse_expr()?),
        };

        let action = if self.peek() == &Token::LBrace {
            Some(self.parse_block_stmts()?)
        } else {
            None
        };

        Ok(Rule { pattern, action })
    }

    fn parse_block_stmts(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        self.skip_semicolons();
        while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
            stmts.push(self.parse_stmt()?);
            self.skip_semicolons();
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek() {
            Token::LBrace => Ok(Stmt::Block(self.parse_block_stmts()?)),
            Token::Print => {
                self.advance();
                let mut exprs = Vec::new();
                if !matches!(self.peek(), Token::Semicolon | Token::RBrace | Token::Eof) {
                    exprs.push(self.parse_expr()?);
                    while self.peek() == &Token::Comma {
                        self.advance();
                        exprs.push(self.parse_expr()?);
                    }
                }
                Ok(Stmt::Print(exprs))
            }
            Token::If => {
                self.advance();
                self.expect(&Token::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                let then_branch = Box::new(self.parse_stmt()?);
                self.skip_semicolons();
                let else_branch = if self.peek() == &Token::Else {
                    self.advance();
                    Some(Box::new(self.parse_stmt()?))
                } else {
                    None
                };
                Ok(Stmt::If(cond, then_branch, else_branch))
            }
            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    pub fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        let expr = self.parse_or()?;

        let compound_op = match self.peek() {
            Token::Assign => None,
            Token::PlusAssign => Some(BinOp::Add),
            Token::MinusAssign => Some(BinOp::Sub),
            Token::StarAssign => Some(BinOp::Mul),
            Token::SlashAssign => Some(BinOp::Div),
            Token::PercentAssign => Some(BinOp::Mod),
            _ => return Ok(expr),
        };
        self.advance();

        let lvalue = match &expr {
            Expr::Var(name) => LValue::Var(name.clone()),
            Expr::Field(inner) => LValue::Field(inner.clone()),
            _ => return Err("left side of an assignment must be a variable or field".to_string()),
        };
        let rhs = self.parse_assign()?;
        let value = match compound_op {
            None => rhs,
            // Desugar `x += e` into `x = x + e` (re-reading `expr` is fine:
            // it's a `Var`/`Field` reference, not a side-effecting call).
            Some(op) => Expr::BinOp(op, Box::new(expr), Box::new(rhs)),
        };
        Ok(Expr::Assign(lvalue, Box::new(value)))
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_match()?;
        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_match()?;
            left = Expr::BinOp(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_match(&mut self) -> Result<Expr, String> {
        let left = self.parse_relational()?;
        match self.peek() {
            Token::Match => {
                self.advance();
                let pattern = self.expect_regex()?;
                Ok(Expr::Match { expr: Box::new(left), pattern, negate: false })
            }
            Token::NotMatch => {
                self.advance();
                let pattern = self.expect_regex()?;
                Ok(Expr::Match { expr: Box::new(left), pattern, negate: true })
            }
            _ => Ok(left),
        }
    }

    fn expect_regex(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Regex(r) => Ok(r),
            other => Err(format!("expected a /regex/ after '~'/'!~', found {other:?}")),
        }
    }

    fn parse_relational(&mut self) -> Result<Expr, String> {
        let left = self.parse_concat()?;
        let op = match self.peek() {
            Token::Eq => BinOp::Eq,
            Token::Ne => BinOp::Ne,
            Token::Lt => BinOp::Lt,
            Token::Le => BinOp::Le,
            Token::Gt => BinOp::Gt,
            Token::Ge => BinOp::Ge,
            _ => return Ok(left),
        };
        self.advance();
        let right = self.parse_concat()?;
        Ok(Expr::BinOp(op, Box::new(left), Box::new(right)))
    }

    /// Two adjacent expressions with no operator between them concatenate
    /// as strings (awk's own rule) — stops at any token that couldn't
    /// start a new operand.
    fn parse_concat(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        while self.starts_operand() {
            let right = self.parse_additive()?;
            left = Expr::Concat(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn starts_operand(&self) -> bool {
        matches!(
            self.peek(),
            Token::Number(_)
                | Token::String(_)
                | Token::Ident(_)
                | Token::Dollar
                | Token::LParen
                | Token::Not
                | Token::Minus
        )
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Token::Minus => {
                self.advance();
                Ok(Expr::Neg(Box::new(self.parse_unary()?)))
            }
            Token::Not => {
                self.advance();
                Ok(Expr::Not(Box::new(self.parse_unary()?)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.advance() {
            Token::Number(n) => Ok(Expr::Num(n)),
            Token::String(s) => Ok(Expr::Str(s)),
            Token::Regex(r) => Ok(Expr::Match { expr: Box::new(Expr::Field(Box::new(Expr::Num(0.0)))), pattern: r, negate: false }),
            Token::Ident(name) => Ok(Expr::Var(name)),
            Token::Dollar => {
                let inner = self.parse_unary()?;
                Ok(Expr::Field(Box::new(inner)))
            }
            Token::LParen => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::Lexer;
    use super::*;

    fn parse(src: &str) -> Program {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn parses_a_bare_action() {
        let prog = parse("{print $1, $3}");
        assert_eq!(prog.rules.len(), 1);
        assert_eq!(prog.rules[0].pattern, Pattern::Always);
    }

    #[test]
    fn parses_begin_and_end() {
        let prog = parse("BEGIN{print \"a\"} {print} END{print \"z\"}");
        assert_eq!(prog.rules.len(), 3);
        assert_eq!(prog.rules[0].pattern, Pattern::Begin);
        assert_eq!(prog.rules[2].pattern, Pattern::End);
    }

    #[test]
    fn parses_relational_pattern_with_default_action() {
        let prog = parse("NR==2");
        assert_eq!(prog.rules.len(), 1);
        assert!(prog.rules[0].action.is_none());
    }

    #[test]
    fn concatenation_binds_tighter_than_relational() {
        // "a" "b" == "ab" should parse as ("a" concat "b") == "ab", not
        // "a" concat ("b" == "ab").
        let prog = parse(r#"{ x = "a" "b" == "ab" }"#);
        if let Some(stmts) = &prog.rules[0].action {
            if let Stmt::Expr(Expr::Assign(_, value)) = &stmts[0] {
                assert!(matches!(**value, Expr::BinOp(BinOp::Eq, _, _)));
                return;
            }
        }
        panic!("unexpected parse shape");
    }
}
