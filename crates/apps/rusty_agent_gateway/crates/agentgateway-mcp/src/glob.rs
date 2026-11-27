//! The one pattern language of `identity.strip` and lens policies.

/// `*` matches any run of characters; everything else matches itself.
pub(crate) fn glob(pattern: &str, name: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else {
        return rest.is_empty();
    };
    for piece in middle {
        match rest.find(piece) {
            Some(at) => rest = &rest[at + piece.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::glob;

    #[test]
    fn glob_matches_runs() {
        assert!(glob("*", ""));
        assert!(glob("*", "anything"));
        assert!(glob("board_*", "board_card_get"));
        assert!(glob("*_get", "board_card_get"));
        assert!(glob("b*c*t", "board_card_get"));
        assert!(glob("exact", "exact"));
        assert!(!glob("exact", "exactly"));
        assert!(!glob("board_*", "github_x"));
        assert!(!glob("a*b*a", "aba_"));
    }
}
