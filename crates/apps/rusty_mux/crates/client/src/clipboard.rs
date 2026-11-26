//! Setting the user's clipboard from a client with OSC 52, the escape most
//! terminals take to copy text to the system clipboard (card #1573).

/// The longest text sent: a terminal may drop a longer sequence, and a path
/// is far under it (`PATH_MAX` is 4096).
pub const MAX: usize = 4096;

/// The OSC 52 sequence that sets the system clipboard to `text`, or `None`
/// when it is empty or over [`MAX`] bytes. The text is base64, so nothing in
/// it can end the sequence early or start another.
#[must_use]
pub fn osc52_set(text: &str) -> Option<String> {
    if text.is_empty() || text.len() > MAX {
        return None;
    }
    Some(format!(
        "\x1b]52;c;{}\x07",
        rusty_base64::encode_standard(text.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_base64_inside_one_sequence() {
        assert_eq!(
            osc52_set("src/lib.rs").unwrap(),
            "\x1b]52;c;c3JjL2xpYi5ycw==\x07"
        );
        let hostile = osc52_set("a\x07\x1b]0;x\x07").unwrap();
        assert_eq!(hostile.matches('\x07').count(), 1, "one terminator");
        assert_eq!(hostile.matches('\x1b').count(), 1, "one introducer");
    }

    #[test]
    fn nothing_or_too_much_is_not_sent() {
        assert_eq!(osc52_set(""), None);
        assert!(osc52_set(&"a".repeat(MAX)).is_some());
        assert_eq!(osc52_set(&"a".repeat(MAX + 1)), None);
    }
}
