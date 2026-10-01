//! Design review Tranche 5: what a search-as-you-type prefix query costs on
//! [`FullTextIndex`], against an exact term. `rusty_tick`'s task search and
//! `remind_me`'s memory and wiki search all run `Query::any_of_prefix`.
//!
//! A deterministic corpus: `DOCS` two-column documents (a short title, a
//! longer body) drawn from a `VOCAB`-word vocabulary. Prints the mean
//! microseconds per search, broadest prefix first, and the build time.
//!
//! `cargo bench -p rusty_multimodal_db_engine --bench fulltext_prefix`

use std::hint::black_box;
use std::time::{Duration, Instant};

use rusty_multimodal_db_engine::fulltext::{FullTextIndex, Query};

const DOCS: u32 = 20_000;
const VOCAB: usize = 30_000;
const TITLE_WORDS: usize = 6;
const BODY_WORDS: usize = 20;
const BUDGET: Duration = Duration::from_millis(500);

/// A small LCG: the same corpus on every run and machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn vocabulary(rng: &mut Lcg) -> Vec<String> {
    (0..VOCAB)
        .map(|_| {
            let len = 3 + rng.below(8);
            (0..len)
                .map(|_| char::from(b'a' + rng.below(26) as u8))
                .collect()
        })
        .collect()
}

fn sentence(rng: &mut Lcg, vocab: &[String], words: usize) -> String {
    (0..words)
        .map(|_| vocab[rng.below(vocab.len())].as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Mean time per call over at least `BUDGET`.
fn time(mut f: impl FnMut() -> usize) -> (f64, usize) {
    let hits = f();
    let start = Instant::now();
    let mut runs = 0u32;
    while start.elapsed() < BUDGET {
        black_box(f());
        runs += 1;
    }
    (start.elapsed().as_secs_f64() * 1e6 / f64::from(runs), hits)
}

fn main() {
    let mut rng = Lcg(0x5eed);
    let vocab = vocabulary(&mut rng);
    let docs: Vec<(String, String)> = (0..DOCS)
        .map(|_| {
            (
                sentence(&mut rng, &vocab, TITLE_WORDS),
                sentence(&mut rng, &vocab, BODY_WORDS),
            )
        })
        .collect();

    let start = Instant::now();
    let mut index: FullTextIndex<u32, 2> = FullTextIndex::new();
    for (key, (title, body)) in (0u32..).zip(&docs) {
        index.upsert(key, [title, body]);
    }
    println!(
        "build: {DOCS} docs, {VOCAB}-word vocabulary, {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );

    let exact = vocab[0].clone();
    let phrase_head = format!("{} {}", docs[0].0.split(' ').next().unwrap_or("a"), "g");
    let cases: [(&str, Query); 5] = [
        ("prefix 'g'", Query::any_of_prefix(["g"])),
        ("prefix 'gr'", Query::any_of_prefix(["gr"])),
        ("prefix 'gro'", Query::any_of_prefix(["gro"])),
        ("exact term", Query::any_of([exact.as_str()])),
        (
            "phrase prefix 'w g'",
            Query::any_of_prefix([phrase_head.as_str()]),
        ),
    ];
    println!("{:<22} {:>12} {:>8}", "query", "µs/search", "hits");
    for (name, query) in &cases {
        let (micros, hits) = time(|| index.search(query).len());
        println!("{name:<22} {micros:>12.1} {hits:>8}");
    }
}
