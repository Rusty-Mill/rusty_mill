//! Tokenizes an awk program. Regex literals (`/.../`) are only recognized
//! in "operand expected" position (start of expression, or right after an
//! operator/keyword/`(`/`,`/`;`/`{`) — the same practical disambiguation
//! real awk lexers use to tell a leading `/` apart from the division
//! operator, without a fully context-free grammar.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    String(String),
    Regex(String),
    Ident(String),
    Begin,
    End,
    Print,
    If,
    Else,
    Dollar,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    Match,
    NotMatch,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semicolon,
    Comma,
    Eof,
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// Whether a `/` at the current position should be read as the start
    /// of a regex literal (true) or the division operator (false).
    expect_operand: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { chars: src.chars().peekable(), expect_operand: true }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.chars.peek(), Some(c) if c.is_whitespace()) {
                self.chars.next();
            }
            if self.chars.peek() == Some(&'#') {
                while matches!(self.chars.peek(), Some(c) if *c != '\n') {
                    self.chars.next();
                }
            } else {
                break;
            }
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok == Token::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_ws_and_comments();
        let Some(&c) = self.chars.peek() else { return Ok(Token::Eof) };

        let tok = match c {
            '0'..='9' => self.read_number(),
            '"' => self.read_string()?,
            '/' if self.expect_operand => self.read_regex()?,
            c if c.is_alphabetic() || c == '_' => self.read_ident(),
            _ => self.read_symbol()?,
        };

        self.expect_operand = !matches!(
            tok,
            Token::Number(_) | Token::String(_) | Token::Regex(_) | Token::Ident(_) | Token::RParen
        );
        Ok(tok)
    }

    fn read_number(&mut self) -> Token {
        let mut s = String::new();
        while matches!(self.chars.peek(), Some(c) if c.is_ascii_digit() || *c == '.') {
            s.push(self.chars.next().unwrap());
        }
        Token::Number(s.parse().unwrap_or(0.0))
    }

    fn read_string(&mut self) -> Result<Token, String> {
        self.chars.next(); // opening quote
        let mut s = String::new();
        loop {
            match self.chars.next() {
                None => return Err("unterminated string literal".to_string()),
                Some('"') => break,
                Some('\\') => match self.chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(other) => s.push(other),
                    None => return Err("unterminated escape in string literal".to_string()),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(Token::String(s))
    }

    fn read_regex(&mut self) -> Result<Token, String> {
        self.chars.next(); // opening '/'
        let mut s = String::new();
        loop {
            match self.chars.next() {
                None => return Err("unterminated regex literal".to_string()),
                Some('/') => break,
                Some('\\') => {
                    s.push('\\');
                    if let Some(next) = self.chars.next() {
                        s.push(next);
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Token::Regex(s))
    }

    fn read_ident(&mut self) -> Token {
        let mut s = String::new();
        while matches!(self.chars.peek(), Some(c) if c.is_alphanumeric() || *c == '_') {
            s.push(self.chars.next().unwrap());
        }
        match s.as_str() {
            "BEGIN" => Token::Begin,
            "END" => Token::End,
            "print" => Token::Print,
            "if" => Token::If,
            "else" => Token::Else,
            _ => Token::Ident(s),
        }
    }

    fn read_symbol(&mut self) -> Result<Token, String> {
        let c = self.chars.next().unwrap();
        let peek = self.chars.peek().copied();
        Ok(match (c, peek) {
            ('$', _) => Token::Dollar,
            ('=', Some('=')) => {
                self.chars.next();
                Token::Eq
            }
            ('=', _) => Token::Assign,
            ('!', Some('=')) => {
                self.chars.next();
                Token::Ne
            }
            ('!', Some('~')) => {
                self.chars.next();
                Token::NotMatch
            }
            ('!', _) => Token::Not,
            ('<', Some('=')) => {
                self.chars.next();
                Token::Le
            }
            ('<', _) => Token::Lt,
            ('>', Some('=')) => {
                self.chars.next();
                Token::Ge
            }
            ('>', _) => Token::Gt,
            ('&', Some('&')) => {
                self.chars.next();
                Token::And
            }
            ('|', Some('|')) => {
                self.chars.next();
                Token::Or
            }
            ('~', _) => Token::Match,
            ('+', Some('=')) => {
                self.chars.next();
                Token::PlusAssign
            }
            ('-', Some('=')) => {
                self.chars.next();
                Token::MinusAssign
            }
            ('*', Some('=')) => {
                self.chars.next();
                Token::StarAssign
            }
            ('/', Some('=')) => {
                self.chars.next();
                Token::SlashAssign
            }
            ('%', Some('=')) => {
                self.chars.next();
                Token::PercentAssign
            }
            ('+', _) => Token::Plus,
            ('-', _) => Token::Minus,
            ('*', _) => Token::Star,
            ('/', _) => Token::Slash,
            ('%', _) => Token::Percent,
            ('(', _) => Token::LParen,
            (')', _) => Token::RParen,
            ('{', _) => Token::LBrace,
            ('}', _) => Token::RBrace,
            (';', _) => Token::Semicolon,
            (',', _) => Token::Comma,
            (other, _) => return Err(format!("unexpected character '{other}'")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_a_print_action() {
        let toks = Lexer::new("{print $1, $3}").tokenize().unwrap();
        assert_eq!(
            toks,
            vec![
                Token::LBrace,
                Token::Print,
                Token::Dollar,
                Token::Number(1.0),
                Token::Comma,
                Token::Dollar,
                Token::Number(3.0),
                Token::RBrace,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn distinguishes_regex_literal_from_division() {
        let toks = Lexer::new("/foo/ { print }").tokenize().unwrap();
        assert_eq!(toks[0], Token::Regex("foo".to_string()));

        let toks = Lexer::new("{ print $1/2 }").tokenize().unwrap();
        assert!(toks.contains(&Token::Slash));
    }

    #[test]
    fn string_escapes_are_decoded() {
        let toks = Lexer::new(r#""a\tb\n""#).tokenize().unwrap();
        assert_eq!(toks[0], Token::String("a\tb\n".to_string()));
    }
}
