//! Criterion benchmarks backing the crate's performance claims: `LineIndex`
//! vs the O(document) free conversion functions, batch edit application,
//! and transport framing throughput.
//!
//! Run with `cargo bench`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rusty_lsp::jsonrpc::{Message, Notification};
use rusty_lsp::lsp::{Position, PositionEncodingKind, Range, TextEdit};
use rusty_lsp::text::{LineIndex, apply_edits, position_to_offset_with};
use rusty_lsp::transport::{buffered, read_message, write_message};

/// A ~120 KB, 4000-line document with multi-byte characters sprinkled in.
fn large_text() -> String {
    let mut text = String::new();
    for line in 0..4000 {
        text.push_str(&format!(
            "fn item_{line}() {{ let value = \"héllo wörld {line}\"; }}\n"
        ));
    }
    text
}

fn position_lookup(c: &mut Criterion) {
    let text = large_text();
    let index = LineIndex::new(&text);
    let position = Position::new(3500, 20);

    let mut group = c.benchmark_group("position_to_offset");
    group.bench_function("free_function_scan", |b| {
        b.iter(|| {
            position_to_offset_with(
                black_box(&text),
                black_box(position),
                PositionEncodingKind::Utf16,
            )
        })
    });
    group.bench_function("line_index", |b| {
        b.iter(|| {
            index.position_to_offset(
                black_box(&text),
                black_box(position),
                PositionEncodingKind::Utf16,
            )
        })
    });
    group.bench_function("line_index_build", |b| {
        b.iter(|| LineIndex::new(black_box(&text)))
    });
    group.finish();
}

fn edit_application(c: &mut Criterion) {
    let text = large_text();
    let edits: Vec<TextEdit> = (0..100)
        .map(|i| {
            let line = i * 40;
            TextEdit::new(
                Range::new(Position::new(line, 3), Position::new(line, 7)),
                "renamed",
            )
        })
        .collect();
    c.bench_function("apply_edits_100_on_120kb", |b| {
        b.iter(|| apply_edits(black_box(&text), black_box(&edits)).unwrap())
    });
}

fn framing(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let message = Message::Notification(Notification {
        method: "textDocument/publishDiagnostics".to_owned(),
        params: Some(serde_json::json!({
            "uri": "file:///a.rs",
            "diagnostics": [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}},
                "message": "x",
            }],
        })),
    });
    c.bench_function("frame_round_trip", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let (mut client, server) = tokio::io::duplex(64 * 1024);
                write_message(&mut client, black_box(&message))
                    .await
                    .unwrap();
                let mut reader = buffered(server);
                read_message(&mut reader).await.unwrap().unwrap()
            })
        })
    });
}

criterion_group!(benches, position_lookup, edit_application, framing);
criterion_main!(benches);
