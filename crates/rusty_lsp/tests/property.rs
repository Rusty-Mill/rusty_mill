//! Deterministic randomized ("property-style") tests for the crate's
//! algorithmic core, using a fixed-seed PRNG — hundreds of generated cases
//! per invariant, zero flakiness, zero dependencies.

use rusty_lsp::lsp::{Position, PositionEncodingKind, Range, TextEdit};
use rusty_lsp::text::{
    LineIndex, apply_edits_with, offset_to_position_with, position_to_offset_with,
};

/// A splitmix64 PRNG: tiny, deterministic, good enough for test-case
/// generation.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }
}

/// Random text mixing ASCII, multi-byte chars, and newlines.
fn random_text(rng: &mut Rng, max_chars: usize) -> String {
    const ALPHABET: [char; 12] = [
        'a', 'b', ' ', '\n', 'é', '汉', '😀', 'x', '\n', '0', 'ß', '\t',
    ];
    (0..rng.below(max_chars + 1))
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

const ENCODINGS: [PositionEncodingKind; 3] = [
    PositionEncodingKind::Utf8,
    PositionEncodingKind::Utf16,
    PositionEncodingKind::Utf32,
];

#[test]
fn line_index_agrees_with_free_functions_on_random_texts() {
    let mut rng = Rng(0xDEC0DE);
    for _ in 0..300 {
        let text = random_text(&mut rng, 80);
        let index = LineIndex::new(&text);
        for encoding in ENCODINGS {
            for _ in 0..20 {
                let offset = rng.below(text.len() + 3);
                assert_eq!(
                    index.offset_to_position(&text, offset, encoding),
                    offset_to_position_with(&text, offset, encoding),
                    "offset {offset} in {text:?} ({encoding:?})"
                );
                let position = Position::new(rng.below(8) as u32, rng.below(12) as u32);
                assert_eq!(
                    index.position_to_offset(&text, position, encoding),
                    position_to_offset_with(&text, position, encoding),
                    "position {position:?} in {text:?} ({encoding:?})"
                );
            }
        }
    }
}

/// Snap a byte offset in `text` down to a char boundary.
fn snap(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[test]
fn apply_edits_matches_a_naive_reference_on_random_edit_batches() {
    let mut rng = Rng(0xED17);
    for _ in 0..300 {
        let text = random_text(&mut rng, 60);

        // Build up to 4 non-overlapping byte ranges, then convert them to
        // positions. Working backwards from random split points guarantees
        // non-overlap by construction.
        let mut cuts: Vec<usize> = (0..rng.below(9))
            .map(|_| snap(&text, rng.below(text.len() + 1)))
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut byte_edits: Vec<(usize, usize, String)> = cuts
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| (pair[0], pair[1], random_text(&mut rng, 6)))
            .collect();

        // The naive reference: apply back-to-front in byte space.
        let mut expected = text.clone();
        for (start, end, replacement) in byte_edits.iter().rev() {
            expected.replace_range(start..end, replacement);
        }

        for encoding in ENCODINGS {
            // Convert to position-based edits, shuffled.
            let mut edits: Vec<TextEdit> = byte_edits
                .iter()
                .map(|(start, end, replacement)| {
                    TextEdit::new(
                        Range::new(
                            offset_to_position_with(&text, *start, encoding),
                            offset_to_position_with(&text, *end, encoding),
                        ),
                        replacement.clone(),
                    )
                })
                .collect();
            for i in (1..edits.len()).rev() {
                edits.swap(i, rng.below(i + 1));
            }
            assert_eq!(
                apply_edits_with(&text, &edits, encoding).expect("non-overlapping"),
                expected,
                "text {text:?}, edits {byte_edits:?} ({encoding:?})"
            );
        }
        byte_edits.clear();
    }
}

#[tokio::test]
async fn transport_round_trips_random_payload_sizes() {
    use rusty_lsp::jsonrpc::{Message, Notification};
    use rusty_lsp::transport::{buffered, read_message, write_message};

    let mut rng = Rng(0x7A115);
    let (mut client, server) = tokio::io::duplex(1 << 22);
    let mut reader = buffered(server);
    for round in 0..50 {
        let max_chars = 1 << rng.below(15);
        let payload = random_text(&mut rng, max_chars);
        let message = Message::Notification(Notification {
            method: format!("test/round{round}"),
            params: Some(serde_json::json!({ "payload": payload })),
        });
        write_message(&mut client, &message).await.expect("write");
        let read = read_message(&mut reader)
            .await
            .expect("read")
            .expect("message");
        assert_eq!(read, message, "round {round}");
    }
}
