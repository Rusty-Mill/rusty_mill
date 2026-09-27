//! FTS5's `unicode61` tokenizer, with its defaults: token characters are the
//! Unicode categories `L*`, `N*` and `Co`, case folds, and diacritics are
//! removed (`remove_diacritics=1`).
//!
//! A port of `fts5_tokenize.c` and `fts5_unicode2.c`, so a text splits into
//! exactly the tokens FTS5 would make, with the same byte offsets. The tables
//! in `unicode_tables.rs` are generated from the SQLite amalgamation the
//! workspace builds (`libsqlite3-sys`'s bundled `sqlite3.c`) by
//! `scripts/gen_unicode61_tables.py` in this crate; regenerate them when that
//! SQLite moves, and the differential tests say whether anything changed.

use super::unicode_tables::{
    CATEGORY_BLOCKS, CATEGORY_DATA, CATEGORY_MAP, DIACRITIC_BASE, DIACRITIC_KEYS, FOLD_OFFSETS,
    FOLD_RULES,
};

/// One token: its folded text and where it came from in the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    /// Byte offset of its first character.
    pub start: usize,
    /// Byte offset just past its last character.
    pub end: usize,
}

/// Category indices `unicode61` treats as token characters by default:
/// `L*` (5 to 9, and 30 for the paired upper/lower ranges), `N*` (13 to
/// 15), `Co` (31), and 0, which the category parser always sets (code
/// points the tables do not cover).
const TOKEN_CATEGORIES: u32 = (1 << 0)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 30)
    | (1 << 13)
    | (1 << 14)
    | (1 << 15)
    | (1 << 31);

/// The SQLite version whose tables this tokenizer carries.
pub fn tables_sqlite_version() -> &'static str {
    super::unicode_tables::SQLITE_VERSION
}

/// Split `text` into tokens.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    loop {
        // Skip separators up to the first character that starts a token.
        let Some((start, first)) = chars.find(|(_, c)| starts_token(*c)) else {
            break;
        };
        let mut folded = String::new();
        push_folded(&mut folded, first);
        let mut end = start + first.len_utf8();
        while let Some(&(at, c)) = chars.peek() {
            if !continues_token(c) {
                break;
            }
            push_folded(&mut folded, c);
            end = at + c.len_utf8();
            chars.next();
        }
        tokens.push(Token {
            text: folded,
            start,
            end,
        });
    }
    tokens
}

/// Whether `c` can begin a token.
fn starts_token(c: char) -> bool {
    let code = u32::from(c);
    if code < 128 {
        return ascii_token_char(code);
    }
    is_token_category(code)
}

/// Whether `c` can continue a token: any token character, or a combining
/// diacritic, which extends a token but never starts one.
fn continues_token(c: char) -> bool {
    let code = u32::from(c);
    if code < 128 {
        return ascii_token_char(code);
    }
    is_token_category(code) || is_diacritic(code)
}

fn ascii_token_char(code: u32) -> bool {
    // Code point 0 is never a token character (`sqlite3Fts5UnicodeAscii`).
    code != 0 && is_token_category(code)
}

fn is_token_category(code: u32) -> bool {
    TOKEN_CATEGORIES & (1 << category(code)) != 0
}

fn push_folded(out: &mut String, c: char) {
    let code = u32::from(c);
    if code < 128 {
        out.push(c.to_ascii_lowercase());
        return;
    }
    let folded = fold(code, true);
    // Folding a lone diacritic to nothing drops it.
    if folded != 0 {
        if let Some(c) = char::from_u32(folded) {
            out.push(c);
        }
    }
}

/// `sqlite3Fts5UnicodeCategory`: the category index of `code`.
fn category(code: u32) -> u32 {
    if code >= 1 << 20 {
        return 0;
    }
    let mut lo = usize::from(CATEGORY_BLOCKS[(code >> 16) as usize]);
    let mut hi = usize::from(CATEGORY_BLOCKS[1 + (code >> 16) as usize]);
    let key = (code & 0xFFFF) as u16;
    let mut found: Option<usize> = None;
    while hi > lo {
        let test = (hi + lo) / 2;
        if key >= CATEGORY_MAP[test] {
            found = Some(test);
            lo = test + 1;
        } else {
            hi = test;
        }
    }
    let Some(at) = found else { return 0 };
    let start = u32::from(CATEGORY_MAP[at]);
    let data = u32::from(CATEGORY_DATA[at]);
    if u32::from(key) >= start + (data >> 5) {
        return 0;
    }
    match data & 0x1F {
        // A range alternating upper and lower case.
        30 if (u32::from(key) - start) & 1 == 1 => 5,
        30 => 9,
        other => other,
    }
}

/// `sqlite3Fts5UnicodeIsdiacritic`.
fn is_diacritic(code: u32) -> bool {
    const MASK0: u32 = 0x0802_9FDF;
    const MASK1: u32 = 0x0003_61F8;
    if !(768..=817).contains(&code) {
        return false;
    }
    if code < 768 + 32 {
        MASK0 & (1 << (code - 768)) != 0
    } else {
        MASK1 & (1 << (code - 768 - 32)) != 0
    }
}

/// `sqlite3Fts5UnicodeFold` for code points of 128 and above.
fn fold(code: u32, remove_diacritics: bool) -> u32 {
    if code < 65536 {
        let mut found = 0;
        let (mut lo, mut hi) = (0i64, FOLD_RULES.len() as i64 - 1);
        while hi >= lo {
            let test = ((hi + lo) / 2) as usize;
            if code >= u32::from(FOLD_RULES[test].0) {
                found = test;
                lo = test as i64 + 1;
            } else {
                hi = test as i64 - 1;
            }
        }
        let (first, flags, range) = FOLD_RULES[found];
        let (first, flags) = (u32::from(first), u32::from(flags));
        let mut folded = code;
        if code < first + u32::from(range) && 0 == (1 & flags & (first ^ code)) {
            folded = (code + u32::from(FOLD_OFFSETS[(flags >> 1) as usize])) & 0xFFFF;
        }
        if remove_diacritics {
            folded = remove_diacritic(folded);
        }
        return folded;
    }
    if (66560..66600).contains(&code) {
        return code + 40;
    }
    code
}

/// `fts5_remove_diacritic` in its simple mode.
fn remove_diacritic(code: u32) -> u32 {
    let key = (code << 3) | 7;
    let mut found = 0;
    let (mut lo, mut hi) = (0i64, DIACRITIC_KEYS.len() as i64 - 1);
    while hi >= lo {
        let test = ((hi + lo) / 2) as usize;
        if key >= u32::from(DIACRITIC_KEYS[test]) {
            found = test;
            lo = test as i64 + 1;
        } else {
            hi = test as i64 - 1;
        }
    }
    let base = DIACRITIC_BASE[found];
    // The high bit marks a mapping only the complex mode applies.
    if base & 0x80 != 0 {
        return code;
    }
    let entry = u32::from(DIACRITIC_KEYS[found]);
    if code > (entry >> 3) + (entry & 7) {
        code
    } else {
        u32::from(base & 0x7F)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(input: &str) -> Vec<String> {
        tokenize(input).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn ascii_splits_on_everything_but_letters_and_digits() {
        assert_eq!(
            texts("memory_tags are useful!"),
            ["memory", "tags", "are", "useful"]
        );
        assert_eq!(texts("v2.0-rc1"), ["v2", "0", "rc1"]);
        assert!(texts("  ?!.,  ").is_empty());
    }

    #[test]
    fn case_folds_and_diacritics_go() {
        assert_eq!(texts("Café NAÏVE Résumé"), ["cafe", "naive", "resume"]);
        assert_eq!(texts("ÆSIR Straße"), ["æsir", "straße"]);
        assert_eq!(texts("ΑΘΗΝΑ"), ["αθηνα"]);
    }

    #[test]
    fn a_combining_mark_extends_a_token_but_never_starts_one() {
        // "e" then U+0301 COMBINING ACUTE ACCENT, which folds away.
        assert_eq!(texts("cafe\u{301} \u{301}x"), ["cafe", "x"]);
    }

    #[test]
    fn offsets_are_byte_offsets_into_the_input() {
        let tokens = tokenize("héllo wörld");
        assert_eq!((tokens[0].start, tokens[0].end), (0, 6));
        assert_eq!((tokens[1].start, tokens[1].end), (7, 13));
        assert_eq!(&"héllo wörld"[7..13], "wörld");
    }

    #[test]
    fn cjk_runs_are_single_tokens() {
        assert_eq!(texts("日本語 テスト"), ["日本語", "テスト"]);
    }

    #[test]
    fn the_tables_record_their_sqlite() {
        assert!(tables_sqlite_version().starts_with("3."));
    }
}
