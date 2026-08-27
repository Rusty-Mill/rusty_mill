//! `LIKE`/`GLOB` pattern matching (issue #113), hand-rolled rather than
//! pulling in a glob/regex crate — both patterns are small (SQL literals),
//! so a straightforward dynamic-programming matcher is plenty fast and
//! keeps this crate dependency-free.
//!
//! **`LIKE`**: `%` matches any run of characters (including none), `_`
//! matches exactly one character, ASCII-case-insensitive (SQLite's own
//! default, no `ICU` extension here), with an optional `ESCAPE` character
//! that makes the following character literal.
//!
//! **`GLOB`**: `*` (any run), `?` (one character), `[...]`/`[^...]`
//! (character class, with `-` ranges) — Unix glob-style, case-sensitive,
//! no escape character (matching real SQLite's `GLOB`, which has none).

enum LikeToken {
    Any,
    One,
    Literal(char),
}

fn parse_like_pattern(pattern: &str, escape: Option<char>) -> Vec<LikeToken> {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if escape == Some(c) {
            if let Some(next) = chars.next() {
                tokens.push(LikeToken::Literal(next.to_ascii_lowercase()));
            }
        } else if c == '%' {
            tokens.push(LikeToken::Any);
        } else if c == '_' {
            tokens.push(LikeToken::One);
        } else {
            tokens.push(LikeToken::Literal(c.to_ascii_lowercase()));
        }
    }
    tokens
}

/// Whether `text` matches `pattern` per `LIKE`'s rules, optionally with
/// `escape` making the following pattern character literal.
pub(crate) fn like_match(text: &str, pattern: &str, escape: Option<char>) -> bool {
    let text_chars: Vec<char> = text.chars().map(|c| c.to_ascii_lowercase()).collect();
    let tokens = parse_like_pattern(pattern, escape);
    wildcard_match(&text_chars, &tokens, |c, token| match token {
        LikeToken::Any => unreachable!(),
        LikeToken::One => true,
        LikeToken::Literal(p) => c == *p,
    })
}

enum GlobToken {
    Any,
    One,
    Class(Vec<(char, char)>, bool),
    Literal(char),
}

fn parse_char_class(chars: &[char]) -> Vec<(char, char)> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            ranges.push((chars[i], chars[i + 2]));
            i += 3;
        } else {
            ranges.push((chars[i], chars[i]));
            i += 1;
        }
    }
    ranges
}

fn parse_glob_pattern(pattern: &str) -> Vec<GlobToken> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                tokens.push(GlobToken::Any);
                i += 1;
            }
            '?' => {
                tokens.push(GlobToken::One);
                i += 1;
            }
            '[' => {
                let mut j = i + 1;
                let negate = j < chars.len() && (chars[j] == '^' || chars[j] == '!');
                if negate {
                    j += 1;
                }
                let class_start = j;
                // A `]` immediately after `[` (or `[^`) is a literal
                // member of the class, not the closing bracket.
                if j < chars.len() && chars[j] == ']' {
                    j += 1;
                }
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j >= chars.len() {
                    // Unterminated `[` -- treat it as a literal character.
                    tokens.push(GlobToken::Literal('['));
                    i += 1;
                    continue;
                }
                tokens.push(GlobToken::Class(
                    parse_char_class(&chars[class_start..j]),
                    negate,
                ));
                i = j + 1;
            }
            c => {
                tokens.push(GlobToken::Literal(c));
                i += 1;
            }
        }
    }
    tokens
}

/// Whether `text` matches `pattern` per `GLOB`'s rules.
pub(crate) fn glob_match(text: &str, pattern: &str) -> bool {
    let text_chars: Vec<char> = text.chars().collect();
    let tokens = parse_glob_pattern(pattern);
    wildcard_match(&text_chars, &tokens, |c, token| match token {
        GlobToken::Any => unreachable!(),
        GlobToken::One => true,
        GlobToken::Class(ranges, negate) => {
            let in_class = ranges.iter().any(|&(lo, hi)| c >= lo && c <= hi);
            in_class != *negate
        }
        GlobToken::Literal(p) => c == *p,
    })
}

/// Shared dynamic-programming wildcard matcher: `token_matches` decides
/// whether a single text character satisfies a non-"any" pattern token
/// (an `Any` token is handled generically here, the same way for both
/// `LIKE`/`GLOB`, since "match a run of any length" doesn't depend on
/// what kind of pattern this is).
fn wildcard_match<T>(text: &[char], tokens: &[T], token_matches: impl Fn(char, &T) -> bool) -> bool
where
    T: IsAny,
{
    let (n, m) = (text.len(), tokens.len());
    let mut dp = vec![vec![false; m + 1]; n + 1];
    dp[0][0] = true;
    for (j, token) in tokens.iter().enumerate() {
        if token.is_any() {
            dp[0][j + 1] = dp[0][j];
        }
    }
    for i in 1..=n {
        for (j, token) in tokens.iter().enumerate() {
            dp[i][j + 1] = if token.is_any() {
                dp[i - 1][j + 1] || dp[i][j]
            } else {
                dp[i - 1][j] && token_matches(text[i - 1], token)
            };
        }
    }
    dp[n][m]
}

trait IsAny {
    fn is_any(&self) -> bool;
}

impl IsAny for LikeToken {
    fn is_any(&self) -> bool {
        matches!(self, LikeToken::Any)
    }
}

impl IsAny for GlobToken {
    fn is_any(&self) -> bool {
        matches!(self, GlobToken::Any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_percent_matches_any_run() {
        assert!(like_match("hello world", "hello%", None));
        assert!(like_match("hello", "hello%", None));
        assert!(like_match("hello world", "%world", None));
        assert!(!like_match("goodbye", "hello%", None));
    }

    #[test]
    fn like_underscore_matches_exactly_one_char() {
        assert!(like_match("cat", "c_t", None));
        assert!(!like_match("ct", "c_t", None));
        assert!(!like_match("caat", "c_t", None));
    }

    #[test]
    fn like_is_ascii_case_insensitive() {
        assert!(like_match("HELLO", "hello", None));
        assert!(like_match("Hello World", "hello%", None));
    }

    #[test]
    fn like_escape_makes_wildcards_literal() {
        assert!(like_match("50%", "50\\%", Some('\\')));
        assert!(!like_match("50x", "50\\%", Some('\\')));
    }

    #[test]
    fn like_empty_pattern_only_matches_empty_text() {
        assert!(like_match("", "", None));
        assert!(!like_match("a", "", None));
    }

    #[test]
    fn glob_star_matches_any_run() {
        assert!(glob_match("hello.txt", "*.txt"));
        assert!(glob_match("hello.txt", "hello*"));
        assert!(!glob_match("hello.png", "*.txt"));
    }

    #[test]
    fn glob_question_mark_matches_exactly_one_char() {
        assert!(glob_match("cat", "c?t"));
        assert!(!glob_match("ct", "c?t"));
    }

    #[test]
    fn glob_is_case_sensitive() {
        assert!(!glob_match("HELLO", "hello"));
        assert!(glob_match("HELLO", "HELLO"));
    }

    #[test]
    fn glob_char_class_matches_any_member() {
        assert!(glob_match("cat", "[bc]at"));
        assert!(glob_match("bat", "[bc]at"));
        assert!(!glob_match("rat", "[bc]at"));
    }

    #[test]
    fn glob_char_class_range() {
        assert!(glob_match("5", "[0-9]"));
        assert!(!glob_match("a", "[0-9]"));
    }

    #[test]
    fn glob_negated_char_class() {
        assert!(!glob_match("cat", "[^bc]at"));
        assert!(glob_match("rat", "[^bc]at"));
    }
}
