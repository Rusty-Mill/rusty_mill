//! Fuzz batch edit application: arbitrary texts and edit lists must never
//! panic — invalid batches are rejected with an error, valid ones apply.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_lsp::lsp::{Position, Range, TextEdit};
use rusty_lsp::text::apply_edits;

fuzz_target!(|input: (String, Vec<(u16, u16, u16, u16, String)>)| {
    let (text, raw_edits) = input;
    let edits: Vec<TextEdit> = raw_edits
        .into_iter()
        .take(8)
        .map(|(sl, sc, el, ec, replacement)| {
            TextEdit::new(
                Range::new(
                    Position::new(sl as u32, sc as u32),
                    Position::new(el as u32, ec as u32),
                ),
                replacement,
            )
        })
        .collect();
    let _ = apply_edits(&text, &edits);
});
