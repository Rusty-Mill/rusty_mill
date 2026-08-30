//! Recursive-descent parser over the expression token stream, lowest to
//! highest precedence: `or`, `and`, `not`, comparison/`in`/`is`, `~`
//! concatenation, `+`/`-`, postfix (`.attr`, `[index]`, `|filter`),
//! primary.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::ast::{BinOp, Expr};
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

type PResult<T> = Result<T, &'static str>;

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

    fn expect(&mut self, expected: &Token) -> PResult<()> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err("unexpected token in expression")
        }
    }

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_or()
    }

    pub fn parse_expr_to_eof(&mut self) -> PResult<Expr> {
        let e = self.parse_expr()?;
        if self.peek() != &Token::Eof {
            return Err("trailing tokens after expression");
        }
        Ok(e)
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut left = self.parse_and()?;
        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut left = self.parse_not()?;
        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::BinOp(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> PResult<Expr> {
        if self.peek() == &Token::Not {
            self.advance();
            Ok(Expr::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let left = self.parse_concat()?;
        match self.peek().clone() {
            Token::Eq => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Eq,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::Ne => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Ne,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::Lt => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Lt,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::Le => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Le,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::Gt => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Gt,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::Ge => {
                self.advance();
                Ok(Expr::BinOp(
                    BinOp::Ge,
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                ))
            }
            Token::In => {
                self.advance();
                Ok(Expr::In(
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                    false,
                ))
            }
            Token::Not if self.tokens.get(self.pos + 1) == Some(&Token::In) => {
                self.advance();
                self.advance();
                Ok(Expr::In(
                    Box::new(left),
                    Box::new(self.parse_concat()?),
                    true,
                ))
            }
            Token::Is => {
                self.advance();
                let negate = if self.peek() == &Token::Not {
                    self.advance();
                    true
                } else {
                    false
                };
                match self.advance() {
                    Token::Ident(name) => Ok(Expr::Test(Box::new(left), name, negate)),
                    _ => Err("expected a test name after 'is'"),
                }
            }
            _ => Ok(left),
        }
    }

    fn parse_concat(&mut self) -> PResult<Expr> {
        let mut left = self.parse_additive()?;
        while self.peek() == &Token::Tilde {
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Concat(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut left = self.parse_postfix()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_postfix()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek().clone() {
                Token::Dot => {
                    self.advance();
                    let name = match self.advance() {
                        Token::Ident(n) => n,
                        _ => return Err("expected a name after '.'"),
                    };
                    if self.peek() == &Token::LParen {
                        let args = self.parse_call_args()?;
                        expr = Expr::Filter(Box::new(expr), name, args);
                    } else {
                        expr = Expr::Attr(Box::new(expr), name);
                    }
                }
                Token::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    expr = match index {
                        Expr::Str(s) => Expr::Attr(Box::new(expr), s),
                        other => Expr::Index(Box::new(expr), Box::new(other)),
                    };
                }
                Token::Pipe => {
                    self.advance();
                    let name = match self.advance() {
                        Token::Ident(n) => n,
                        _ => return Err("expected a filter name after '|'"),
                    };
                    let args = if self.peek() == &Token::LParen {
                        self.parse_call_args()?
                    } else {
                        Vec::new()
                    };
                    expr = Expr::Filter(Box::new(expr), name, args);
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> PResult<Vec<Expr>> {
        self.expect(&Token::LParen)?;
        let mut args = Vec::new();
        if self.peek() != &Token::RParen {
            args.push(self.parse_expr()?);
            while self.peek() == &Token::Comma {
                self.advance();
                args.push(self.parse_expr()?);
            }
        }
        self.expect(&Token::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        match self.advance() {
            Token::Number(n) => Ok(Expr::Num(n)),
            Token::String(s) => Ok(Expr::Str(s)),
            Token::Ident(name) => match name.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "none" => Ok(Expr::None),
                _ => Ok(Expr::Var(name)),
            },
            Token::Minus => Ok(Expr::BinOp(
                BinOp::Sub,
                Box::new(Expr::Num(0.0)),
                Box::new(self.parse_postfix()?),
            )),
            Token::LParen => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            _ => Err("expected a value in expression"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse(src: &str) -> Expr {
        Parser::new(tokenize(src).unwrap())
            .parse_expr_to_eof()
            .unwrap()
    }

    #[test]
    fn parses_bracket_index_as_attr() {
        let e = parse("message['role']");
        assert_eq!(
            e,
            Expr::Attr(Box::new(Expr::Var("message".into())), "role".into())
        );
    }

    #[test]
    fn parses_dotted_attr() {
        let e = parse("loop.last");
        assert_eq!(
            e,
            Expr::Attr(Box::new(Expr::Var("loop".into())), "last".into())
        );
    }

    #[test]
    fn parses_filter_chain() {
        let e = parse("message['content'] | trim | upper");
        assert_eq!(
            e,
            Expr::Filter(
                Box::new(Expr::Filter(
                    Box::new(Expr::Attr(
                        Box::new(Expr::Var("message".into())),
                        "content".into()
                    )),
                    "trim".into(),
                    alloc::vec::Vec::new(),
                )),
                "upper".into(),
                alloc::vec::Vec::new(),
            )
        );
    }

    #[test]
    fn parses_method_call_as_filter() {
        let e = parse("message['content'].strip()");
        assert_eq!(
            e,
            Expr::Filter(
                Box::new(Expr::Attr(
                    Box::new(Expr::Var("message".into())),
                    "content".into()
                )),
                "strip".into(),
                alloc::vec::Vec::new(),
            )
        );
    }

    #[test]
    fn parses_is_defined_test() {
        let e = parse("system_message is defined");
        assert_eq!(
            e,
            Expr::Test(
                Box::new(Expr::Var("system_message".into())),
                "defined".into(),
                false
            )
        );
    }

    #[test]
    fn parses_is_not_defined_test() {
        let e = parse("system_message is not defined");
        assert_eq!(
            e,
            Expr::Test(
                Box::new(Expr::Var("system_message".into())),
                "defined".into(),
                true
            )
        );
    }
}
