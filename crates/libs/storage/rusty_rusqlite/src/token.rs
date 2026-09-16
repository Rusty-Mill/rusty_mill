//! SQL tokenizer: turns a SQL statement into a stream of tokens. No
//! grammar/statement-level semantics live here — see the parser (built on
//! top of this) for that. Grammar reference:
//! <https://www.sqlite.org/lang.html>.

/// A lexical token produced by [`tokenize`].
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A bare identifier or keyword (e.g. `SELECT`, `foo`). Keyword-vs-
    /// identifier disambiguation is the parser's job, not the tokenizer's.
    Ident(String),
    /// An integer literal.
    Integer(i64),
    /// A floating-point literal.
    Real(f64),
    /// A single-quoted string literal, with the surrounding quotes and any
    /// `''`-escaping already resolved to the literal's text.
    String(String),
    /// A blob literal (`X'...'`), decoded from hex.
    Blob(Vec<u8>),
    /// A single- or multi-character punctuation/operator token, e.g. `,`,
    /// `(`, `)`, `=`, `<>`, `<=`.
    Punct(&'static str),
    /// A `?`/`?N`/`:name`/`@name`/`$name`-style bound-parameter marker
    /// (see `docs/adr/0002-parameter-markers.md`). The stored text is the
    /// digits after `?` (empty for a bare `?`), or the sigil-plus-name
    /// text for the named forms — SQLite treats `:foo`/`@foo`/`$foo` as
    /// distinct parameters even when the name matches, so the sigil is
    /// part of the identity, not decoration.
    Param(String),
    /// End of input.
    Eof,
}

/// An error produced while tokenizing.
#[derive(Debug, PartialEq)]
pub enum TokenError {
    UnterminatedString,
    UnterminatedBlob,
    InvalidBlobHex,
    UnexpectedChar(char),
}

/// Tokenizes a SQL statement into a flat list of tokens, ending with
/// [`Token::Eof`].
pub fn tokenize(sql: &str) -> Result<Vec<Token>, TokenError> {
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c == '\'' {
            let (text, next) = read_string(&chars, i)?;
            tokens.push(Token::String(text));
            i = next;
            continue;
        }

        if (c == 'x' || c == 'X') && chars.get(i + 1) == Some(&'\'') {
            let (bytes, next) = read_blob(&chars, i + 1)?;
            tokens.push(Token::Blob(bytes));
            i = next;
            continue;
        }

        if c.is_ascii_digit() {
            let (tok, next) = read_number(&chars, i);
            tokens.push(tok);
            i = next;
            continue;
        }

        if c == '?' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            tokens.push(Token::Param(chars[start + 1..i].iter().collect()));
            continue;
        }

        if c == ':' || c == '@' || c == '$' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i == start + 1 {
                return Err(TokenError::UnexpectedChar(c));
            }
            tokens.push(Token::Param(chars[start..i].iter().collect()));
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
            continue;
        }

        if let Some((punct, len)) = read_punct(&chars, i) {
            tokens.push(Token::Punct(punct));
            i += len;
            continue;
        }

        return Err(TokenError::UnexpectedChar(c));
    }

    tokens.push(Token::Eof);
    Ok(tokens)
}

fn read_string(chars: &[char], start: usize) -> Result<(String, usize), TokenError> {
    let mut i = start + 1;
    let mut text = String::new();
    loop {
        if i >= chars.len() {
            return Err(TokenError::UnterminatedString);
        }
        if chars[i] == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                text.push('\'');
                i += 2;
                continue;
            }
            return Ok((text, i + 1));
        }
        text.push(chars[i]);
        i += 1;
    }
}

fn read_blob(chars: &[char], quote_start: usize) -> Result<(Vec<u8>, usize), TokenError> {
    let mut i = quote_start + 1;
    let start = i;
    while i < chars.len() && chars[i] != '\'' {
        i += 1;
    }
    if i >= chars.len() {
        return Err(TokenError::UnterminatedBlob);
    }
    let hex: String = chars[start..i].iter().collect();
    if !hex.len().is_multiple_of(2) {
        return Err(TokenError::InvalidBlobHex);
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let hex_bytes = hex.as_bytes();
    for pair in hex_bytes.chunks(2) {
        let s = std::str::from_utf8(pair).map_err(|_| TokenError::InvalidBlobHex)?;
        let byte = u8::from_str_radix(s, 16).map_err(|_| TokenError::InvalidBlobHex)?;
        bytes.push(byte);
    }
    Ok((bytes, i + 1))
}

fn read_number(chars: &[char], start: usize) -> (Token, usize) {
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let mut is_real = false;
    if chars.get(i) == Some(&'.') {
        is_real = true;
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    let text: String = chars[start..i].iter().collect();
    if is_real {
        (Token::Real(text.parse().unwrap_or(0.0)), i)
    } else {
        match text.parse::<i64>() {
            Ok(n) => (Token::Integer(n), i),
            Err(_) => (Token::Real(text.parse().unwrap_or(0.0)), i),
        }
    }
}

const PUNCT_2: [(&str, &str); 5] = [
    ("<=", "<="),
    (">=", ">="),
    ("<>", "<>"),
    ("!=", "!="),
    ("||", "||"),
];

fn read_punct(chars: &[char], i: usize) -> Option<(&'static str, usize)> {
    if i + 1 < chars.len() {
        let two: String = chars[i..i + 2].iter().collect();
        for (pat, name) in PUNCT_2.iter() {
            if two == *pat {
                return Some((name, 2));
            }
        }
    }
    let one = match chars[i] {
        '(' => "(",
        ')' => ")",
        ',' => ",",
        ';' => ";",
        '=' => "=",
        '<' => "<",
        '>' => ">",
        '+' => "+",
        '-' => "-",
        '*' => "*",
        '/' => "/",
        '.' => ".",
        _ => return None,
    };
    Some((one, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_simple_select() {
        let tokens = tokenize("SELECT a, b FROM t WHERE a = 1").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("SELECT".into()),
                Token::Ident("a".into()),
                Token::Punct(","),
                Token::Ident("b".into()),
                Token::Ident("FROM".into()),
                Token::Ident("t".into()),
                Token::Ident("WHERE".into()),
                Token::Ident("a".into()),
                Token::Punct("="),
                Token::Integer(1),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_string_with_escaped_quote() {
        let tokens = tokenize("'it''s'").unwrap();
        assert_eq!(tokens, vec![Token::String("it's".into()), Token::Eof]);
    }

    #[test]
    fn tokenizes_real_literal() {
        let tokens = tokenize("12.75").unwrap();
        assert_eq!(tokens, vec![Token::Real(12.75), Token::Eof]);
    }

    #[test]
    fn tokenizes_blob_literal() {
        let tokens = tokenize("X'48656c6c6f'").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Blob(vec![0x48, 0x65, 0x6c, 0x6c, 0x6f]), Token::Eof]
        );
    }

    #[test]
    fn tokenizes_multi_char_operators() {
        let tokens = tokenize("a <> b").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("a".into()),
                Token::Punct("<>"),
                Token::Ident("b".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_string_is_an_error() {
        assert_eq!(tokenize("'abc"), Err(TokenError::UnterminatedString));
    }

    #[test]
    fn invalid_blob_hex_is_an_error() {
        assert_eq!(tokenize("X'zz'"), Err(TokenError::InvalidBlobHex));
    }

    #[test]
    fn unexpected_char_is_an_error() {
        assert_eq!(tokenize("a # b"), Err(TokenError::UnexpectedChar('#')));
    }

    #[test]
    fn bare_sigil_with_no_name_is_an_error() {
        assert_eq!(tokenize("a @ b"), Err(TokenError::UnexpectedChar('@')));
        assert_eq!(tokenize("a : b"), Err(TokenError::UnexpectedChar(':')));
        assert_eq!(tokenize("a $ b"), Err(TokenError::UnexpectedChar('$')));
    }

    #[test]
    fn tokenizes_anonymous_and_numbered_params() {
        let tokens = tokenize("a = ? AND b = ?2").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("a".into()),
                Token::Punct("="),
                Token::Param("".into()),
                Token::Ident("AND".into()),
                Token::Ident("b".into()),
                Token::Punct("="),
                Token::Param("2".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_named_params_with_distinct_sigils() {
        assert_eq!(
            tokenize(":foo").unwrap(),
            vec![Token::Param(":foo".into()), Token::Eof]
        );
        assert_eq!(
            tokenize("@foo").unwrap(),
            vec![Token::Param("@foo".into()), Token::Eof]
        );
        assert_eq!(
            tokenize("$foo").unwrap(),
            vec![Token::Param("$foo".into()), Token::Eof]
        );
    }
}
