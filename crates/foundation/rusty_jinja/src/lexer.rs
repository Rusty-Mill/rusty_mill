//! Tokenizes the expression mini-language inside `{{ }}`/`{% %}` tags
//! (not the surrounding template text — see [`crate::template`] for that).

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    String(String),
    Ident(String),
    Dot,
    Comma,
    Pipe,
    Tilde,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    In,
    Is,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Eof,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, &'static str> {
    let chars: Vec<char> = src.chars().collect();
    let mut pos = 0;
    let mut tokens = Vec::new();

    while pos < chars.len() {
        let c = chars[pos];
        match c {
            c if c.is_whitespace() => pos += 1,
            '0'..='9' => {
                let start = pos;
                while pos < chars.len() && (chars[pos].is_ascii_digit() || chars[pos] == '.') {
                    pos += 1;
                }
                let text: String = chars[start..pos].iter().collect();
                tokens.push(Token::Number(text.parse().map_err(|_| "invalid number")?));
            }
            '"' | '\'' => {
                let quote = c;
                pos += 1;
                let start = pos;
                while pos < chars.len() && chars[pos] != quote {
                    pos += 1;
                }
                if pos >= chars.len() {
                    return Err("unterminated string literal");
                }
                let text: String = chars[start..pos].iter().collect();
                pos += 1;
                tokens.push(Token::String(text));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = pos;
                while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                    pos += 1;
                }
                let text: String = chars[start..pos].iter().collect();
                tokens.push(match text.as_str() {
                    "and" => Token::And,
                    "or" => Token::Or,
                    "not" => Token::Not,
                    "in" => Token::In,
                    "is" => Token::Is,
                    "true" | "True" => Token::Ident("true".into()),
                    "false" | "False" => Token::Ident("false".into()),
                    "none" | "None" | "null" => Token::Ident("none".into()),
                    _ => Token::Ident(text),
                });
            }
            '.' => {
                pos += 1;
                tokens.push(Token::Dot);
            }
            ',' => {
                pos += 1;
                tokens.push(Token::Comma);
            }
            '|' => {
                pos += 1;
                tokens.push(Token::Pipe);
            }
            '~' => {
                pos += 1;
                tokens.push(Token::Tilde);
            }
            '+' => {
                pos += 1;
                tokens.push(Token::Plus);
            }
            '-' => {
                pos += 1;
                tokens.push(Token::Minus);
            }
            '(' => {
                pos += 1;
                tokens.push(Token::LParen);
            }
            ')' => {
                pos += 1;
                tokens.push(Token::RParen);
            }
            '[' => {
                pos += 1;
                tokens.push(Token::LBracket);
            }
            ']' => {
                pos += 1;
                tokens.push(Token::RBracket);
            }
            '=' if chars.get(pos + 1) == Some(&'=') => {
                pos += 2;
                tokens.push(Token::Eq);
            }
            '!' if chars.get(pos + 1) == Some(&'=') => {
                pos += 2;
                tokens.push(Token::Ne);
            }
            '<' if chars.get(pos + 1) == Some(&'=') => {
                pos += 2;
                tokens.push(Token::Le);
            }
            '<' => {
                pos += 1;
                tokens.push(Token::Lt);
            }
            '>' if chars.get(pos + 1) == Some(&'=') => {
                pos += 2;
                tokens.push(Token::Ge);
            }
            '>' => {
                pos += 1;
                tokens.push(Token::Gt);
            }
            _ => return Err("unexpected character in expression"),
        }
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn tokenizes_a_bracket_index_expression() {
        let toks = tokenize("message['role']").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Ident("message".into()),
                Token::LBracket,
                Token::String("role".into()),
                Token::RBracket,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_comparison_and_keywords() {
        let toks = tokenize("message['role'] == 'system' and not loop.last").unwrap();
        assert!(toks.contains(&Token::Eq));
        assert!(toks.contains(&Token::And));
        assert!(toks.contains(&Token::Not));
    }
}
