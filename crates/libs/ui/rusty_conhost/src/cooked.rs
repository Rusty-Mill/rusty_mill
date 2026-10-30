//! The ReadConsole line discipline (design §3.5, `cooked`): ENABLE_LINE_INPUT
//! reads collect characters until Enter, echoing them while
//! ENABLE_ECHO_INPUT is on.
//!
//! This release edits at the end of the line only: characters append,
//! Backspace deletes the last character, Escape clears the line one
//! character at a time ([`Line::erase_one`]), so the caller can hold its
//! echo to the output budget. Echo keeps surrogate pairs whole. The
//! cursor keys, history and the F-key popups come later. Behaviour
//! follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/host/readDataCooked.cpp`, `src/host/stream.cpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use crate::input::Utf8Units;

/// UTF-16 units one line holds (§7.4), or its initial data if that is
/// longer; further typing is ignored.
pub(crate) const MAX_LINE: usize = 8192;

/// How a line ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Done {
    /// Enter.
    Line,
    /// A character in the read's `CtrlWakeupMask`, such as cmd's Tab.
    Wakeup,
}

/// One cooked read in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// UTF-16 units, the initial data first.
    buffer: Vec<u16>,
    /// The most units `buffer` grows to.
    limit: usize,
    /// Escape was pressed and the line is not yet erased.
    erasing: bool,
    /// The echo's UTF-8, holding a high surrogate for its low half.
    utf8: Utf8Units,
    wakeup_mask: u32,
    control_key_state: u32,
    done: Option<Done>,
}

impl Line {
    /// `initial` is ReadConsoleW's `nInitialChars`, which the client has
    /// already displayed.
    pub(crate) fn new(initial: Vec<u16>, wakeup_mask: u32) -> Line {
        Line {
            limit: initial.len().max(MAX_LINE),
            buffer: initial,
            erasing: false,
            utf8: Utf8Units::default(),
            wakeup_mask,
            control_key_state: 0,
            done: None,
        }
    }

    pub(crate) fn done(&self) -> Option<Done> {
        self.done
    }

    /// The most bytes the line ever holds, to reserve while it waits.
    pub(crate) fn reserved(&self) -> usize {
        self.limit * 2
    }

    /// One character (conhost `_handleChar`). Echo, if any, is appended
    /// to `echo`.
    pub(crate) fn char(&mut self, ch: u16, modifiers: u32, processed: bool, echo: &mut Vec<u8>) {
        if self.done.is_some() {
            return;
        }
        if ch < 0x20 && self.wakeup_mask & (1 << ch) != 0 {
            // The client sees the character; the screen doesn't.
            self.buffer.push(ch);
            self.control_key_state = modifiers;
            self.done = Some(Done::Wakeup);
            return;
        }
        match ch {
            0x0d => self.done = Some(Done::Line),
            0x08 | 0x7f if processed => self.backspace(echo),
            _ if self.buffer.len() >= self.limit => {}
            _ => {
                self.buffer.push(ch);
                self.utf8.push(ch, echo);
            }
        }
    }

    /// One editing key without a character (conhost `_handleVkey`).
    /// Escape starts erasing the line; the rest are not supported yet.
    pub(crate) fn vkey(&mut self, vk: u16) {
        if self.done.is_some() || vk != crate::input::vk::ESCAPE {
            return;
        }
        self.erasing = !self.buffer.is_empty();
    }

    /// Whether Escape's erasing is under way: the caller runs
    /// [`Line::erase_one`] until it ends, before the next character.
    pub(crate) fn erasing(&self) -> bool {
        self.erasing
    }

    /// Erase one more character of the line, at most 6 bytes of echo.
    pub(crate) fn erase_one(&mut self, echo: &mut Vec<u8>) {
        self.backspace(echo);
        self.erasing = self.erasing && !self.buffer.is_empty();
    }

    /// Delete the last character, a surrogate pair whole.
    fn backspace(&mut self, echo: &mut Vec<u8>) {
        let Some(last) = self.buffer.pop() else {
            return;
        };
        if self.utf8.pending() {
            // A high surrogate not yet shown: nothing to erase.
            self.utf8.clear();
            return;
        }
        let mut units = [last, 0];
        let mut n = 1;
        if (0xDC00..0xE000).contains(&last)
            && self
                .buffer
                .last()
                .is_some_and(|u| (0xD800..0xDC00).contains(u))
        {
            units = [self.buffer.pop().unwrap_or(last), last];
            n = 2;
        }
        let ch = char::decode_utf16(units[..n].iter().copied())
            .next()
            .and_then(Result::ok)
            .unwrap_or(char::REPLACEMENT_CHARACTER);
        for _ in 0..rusty_term_core::char_width(ch) {
            echo.extend_from_slice(b"\x08 \x08");
        }
    }

    /// The text the read returns, its ControlKeyState, and what to show
    /// for the end of the line (conhost `_handlePostCharInputLoop`): a
    /// line gets `\r\n` (only `\r` without PROCESSED_INPUT), shown even
    /// without echo.
    pub(crate) fn finish(
        mut self,
        processed: bool,
        echoing: bool,
        echo: &mut Vec<u8>,
    ) -> (Vec<u16>, u32) {
        if self.done == Some(Done::Line) {
            if echoing {
                self.utf8.flush(echo);
            }
            let suffix: &[u16] = if processed { &[0x0d, 0x0a] } else { &[0x0d] };
            echo.extend(suffix.iter().map(|&u| u as u8));
            self.buffer.extend_from_slice(suffix);
        }
        (self.buffer, self.control_key_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(line: &mut Line, s: &str, echo: &mut Vec<u8>) {
        for u in s.encode_utf16() {
            line.char(u, 0, true, echo);
        }
    }

    #[test]
    fn a_line_echoes_and_ends_with_crlf() {
        let mut echo = Vec::new();
        let mut line = Line::new(Vec::new(), 0);
        feed(&mut line, "dir\r", &mut echo);
        assert_eq!(line.done(), Some(Done::Line));
        let (text, state) = line.finish(true, true, &mut echo);
        assert_eq!(String::from_utf16_lossy(&text), "dir\r\n");
        assert_eq!(state, 0);
        assert_eq!(echo, b"dir\r\n");
    }

    #[test]
    fn backspace_deletes_whole_characters_and_escape_clears() {
        let mut echo = Vec::new();
        let mut line = Line::new(Vec::new(), 0);
        feed(&mut line, "a😀", &mut echo);
        line.char(0x08, 0, true, &mut echo);
        assert_eq!(line.buffer, [u16::from(b'a')]);
        feed(&mut line, "bc", &mut echo);
        line.vkey(crate::input::vk::ESCAPE);
        let mut steps = 0;
        while line.erasing() {
            let before = echo.len();
            line.erase_one(&mut echo);
            assert!(echo.len() - before <= 3, "one step, one character");
            steps += 1;
        }
        assert_eq!(steps, 3);
        assert!(line.buffer.is_empty());
        assert!(echo.ends_with(b"\x08 \x08\x08 \x08\x08 \x08"));
    }

    #[test]
    fn echo_keeps_a_surrogate_pair_whole() {
        let mut echo = Vec::new();
        let mut line = Line::new(Vec::new(), 0);
        line.char(0xD83D, 0, true, &mut echo);
        assert!(echo.is_empty(), "the high half waits for its partner");
        line.char(0xDE00, 0, true, &mut echo);
        assert_eq!(echo, "😀".as_bytes());
        // A high half erased before its partner shows nothing to erase.
        line.char(0xD83D, 0, true, &mut echo);
        line.char(0x08, 0, true, &mut echo);
        assert_eq!(echo, "😀".as_bytes());
        // A lone half is U+FFFD, a dangling one at Enter too.
        line.char(0xDE00, 0, true, &mut echo);
        line.char(0xD83D, 0, true, &mut echo);
        line.char(0x0d, 0, true, &mut echo);
        let (text, _) = line.finish(true, true, &mut echo);
        assert_eq!(echo, "😀\u{FFFD}\u{FFFD}\r\n".as_bytes());
        assert_eq!(text, [0xD83D, 0xDE00, 0xDE00, 0xD83D, 0x0d, 0x0a]);
    }

    #[test]
    fn without_processed_input_backspace_is_a_character_and_cr_stands_alone() {
        let mut echo = Vec::new();
        let mut line = Line::new(Vec::new(), 0);
        line.char(0x08, 0, false, &mut echo);
        line.char(0x0d, 0, false, &mut echo);
        let (text, _) = line.finish(false, true, &mut echo);
        assert_eq!(text, [0x08, 0x0d]);
    }

    #[test]
    fn a_wakeup_character_ends_the_read_unechoed() {
        let mut echo = Vec::new();
        let mut line = Line::new("cd".encode_utf16().collect(), 1 << 9);
        feed(&mut line, " p", &mut echo);
        line.char(0x09, 0x10, true, &mut echo);
        assert_eq!(line.done(), Some(Done::Wakeup));
        let (text, state) = line.finish(true, true, &mut echo);
        assert_eq!(String::from_utf16_lossy(&text), "cd p\t");
        assert_eq!(state, 0x10);
        assert_eq!(echo, b" p", "initial data isn't echoed, nor is the wakeup");
    }

    #[test]
    fn typing_stops_at_the_cap() {
        let mut echo = Vec::new();
        let mut line = Line::new(vec![u16::from(b'i'); 3], 0);
        for _ in 0..MAX_LINE + 10 {
            line.char(u16::from(b'x'), 0, true, &mut echo);
        }
        assert_eq!(line.buffer.len(), MAX_LINE);
        assert_eq!(line.reserved(), MAX_LINE * 2);
        line.char(0x0d, 0, true, &mut echo);
        assert_eq!(line.done(), Some(Done::Line));
        let long = Line::new(vec![0x20; MAX_LINE + 1], 0);
        assert_eq!(
            long.reserved(),
            (MAX_LINE + 1) * 2,
            "initial data is kept whole"
        );
    }
}
