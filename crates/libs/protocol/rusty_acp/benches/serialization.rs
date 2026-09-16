//! What the wire format costs.
//!
//! Every request and every event round-trips through `serde_json`, so this is
//! the one cost paid on literally every code path — including the ones where
//! nothing else happens. The cases here are chosen for where that cost stops
//! being negligible: a message carrying a base64 artifact is three orders of
//! magnitude larger than a text one, and an agent streaming token by token
//! encodes a `message.part` per token.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusty_acp::types::{
    AgentManifest, AgentName, Event, Message, MessagePart, Run, RunCreateRequest,
};

/// A message part carrying `bytes` of binary content, base64-encoded.
fn artifact(bytes: usize) -> MessagePart {
    MessagePart::binary_artifact("chart.png", "image/png", vec![0u8; bytes])
}

fn events(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode/event");

    // The streaming hot path: one of these per token.
    let part = Event::MessagePart { part: MessagePart::text("a plausible token") };
    group.bench_function("message.part", |b| {
        b.iter(|| serde_json::to_string(std::hint::black_box(&part)).unwrap())
    });

    // A terminal event carries the whole run, including its output.
    let mut run = Run::new(AgentName::new("bench").unwrap(), None);
    run.output = vec![Message::agent("a sentence of output")];
    let completed = Event::RunCompleted { run: Box::new(run) };
    group.bench_function("run.completed", |b| {
        b.iter(|| serde_json::to_string(std::hint::black_box(&completed)).unwrap())
    });

    group.finish();
}

/// Artifacts are where encoding stops being free, so measure against size
/// rather than picking one and hoping it is representative.
fn artifacts(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode/artifact");

    for bytes in [1024, 64 * 1024, 1024 * 1024] {
        let event = Event::MessagePart { part: artifact(bytes) };
        let encoded = serde_json::to_string(&event).unwrap();

        // Throughput is over the *source* bytes, so the numbers stay
        // comparable as the base64 expansion changes.
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(BenchmarkId::new("encode", bytes), &event, |b, event| {
            b.iter(|| serde_json::to_string(std::hint::black_box(event)).unwrap())
        });
        group.bench_with_input(BenchmarkId::new("decode", bytes), &encoded, |b, encoded| {
            b.iter(|| serde_json::from_str::<Event>(std::hint::black_box(encoded)).unwrap())
        });
    }

    group.finish();
}

fn requests(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode/request");

    let request = RunCreateRequest::new(
        AgentName::new("bench").unwrap(),
        [Message::user("a request a client might plausibly send")],
    );
    let encoded = serde_json::to_string(&request).unwrap();
    group.bench_function("run-create/encode", |b| {
        b.iter(|| serde_json::to_string(std::hint::black_box(&request)).unwrap())
    });
    group.bench_function("run-create/decode", |b| {
        b.iter(|| serde_json::from_str::<RunCreateRequest>(std::hint::black_box(&encoded)).unwrap())
    });

    // Served on every `GET /agents`, so its cost is paid by discovery rather
    // than by running anything.
    let manifest = AgentManifest::new(AgentName::new("bench").unwrap(), "A benchmark agent");
    group.bench_function("manifest/encode", |b| {
        b.iter(|| serde_json::to_string(std::hint::black_box(&manifest)).unwrap())
    });

    group.finish();
}

criterion_group!(benches, events, artifacts, requests);
criterion_main!(benches);
