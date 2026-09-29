//! The engine's full-text index against SQLite FTS5 itself: one corpus and
//! one set of queries through both, compared match for match, score for
//! score and snippet for snippet (`rusty_remind_me`'s ADR-0023 §3a).
//!
//! The corpus is generated from a fixed seed, so a failure reproduces. It
//! mixes what a node's memories hold: English prose with sentences, mixed
//! case, identifiers with underscores, accented and non-Latin words,
//! numbers, punctuation, and tags stored as JSON arrays. Queries are built
//! the way the node builds them: any of several quoted phrases. The
//! prefix, `AND` and `NOT` forms a task search box needs (issue #382) are
//! generated the same way and compared the same way.

use rusqlite::{params, Connection};
use rusty_multimodal_db_engine::fulltext::{FullTextIndex, Query, SnippetStyle};

const WORDS: &[&str] = &[
    "the",
    "memory",
    "tags",
    "rust",
    "SQLite",
    "search",
    "index",
    "node",
    "hub",
    "sync",
    "outbox",
    "fox",
    "quick",
    "brown",
    "lazy",
    "dog",
    "café",
    "naïve",
    "Résumé",
    "straße",
    "Αθήνα",
    "日本語",
    "テスト",
    "v2",
    "0.9",
    "2026",
    "memory_tags",
    "user's",
    "don't",
    "RUST",
    "Memory",
    "über",
    "façade",
    "coöperate",
    "déjà",
    "vu",
    "a",
    "of",
    "and",
    "to",
    "in",
    "is",
    "for",
    "on",
    "with",
    "daemon",
    "journal",
    "engine",
    "phase",
    "store",
];
const PUNCT: &[&str] = &[
    " ", " ", " ", " ", ", ", ". ", ": ", "; ", " - ", "! ", "? ", "\n", "  ",
];

/// A small deterministic generator (a 64-bit LCG), so no dependency is
/// needed for randomness and every run sees the same corpus.
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

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len())]
    }

    fn prose(&mut self, max_words: usize) -> String {
        let n = 1 + self.below(max_words);
        let mut text = String::new();
        // Sometimes open with separators, which moves the first token off
        // byte 0 and matters to snippets that start there.
        if self.below(4) == 0 {
            text.push_str(self.pick(&["  ", "- ", "(", "\n", "…"]));
        }
        for i in 0..n {
            if i > 0 {
                text.push_str(self.pick(PUNCT));
            }
            text.push_str(self.pick(WORDS));
        }
        if self.below(4) == 0 {
            text.push_str(self.pick(&[".", "!", " ", ")", "\n"]));
        }
        text
    }

    fn tags(&mut self) -> String {
        let n = self.below(4);
        let tags: Vec<String> = (0..n)
            .map(|_| format!("\"{}\"", self.pick(WORDS)))
            .collect();
        format!("[{}]", tags.join(","))
    }

    fn query(&mut self) -> Vec<String> {
        let n = 1 + self.below(4);
        (0..n)
            .map(|_| match self.below(5) {
                0 => format!("{} {}", self.pick(WORDS), self.pick(WORDS)),
                1 => format!("{}_{}", self.pick(WORDS), self.pick(WORDS)),
                _ => self.pick(WORDS).to_string(),
            })
            .collect()
    }
}

/// The node's own quoting (`remind_me_core::fts::sanitize_fts_query`'s
/// output shape): each phrase double-quoted, joined with `OR`.
fn fts5_expression(phrases: &[String]) -> String {
    phrases
        .iter()
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// A query in both spellings: ours, and the FTS5 expression it must match.
type Both = (Query, String);

fn quoted(phrase: &str) -> String {
    format!("\"{}\"", phrase.replace('"', "\"\""))
}

fn any_of_both(phrases: &[String]) -> Both {
    (
        Query::any_of(phrases.iter().map(String::as_str)),
        fts5_expression(phrases),
    )
}

/// The node's own shape: any of several phrases.
fn plain_query(rng: &mut Lcg) -> Both {
    any_of_both(&rng.query())
}

/// Type-ahead: the last token of each phrase cut short, so most queries
/// still have hits.
fn prefix_query(rng: &mut Lcg) -> Both {
    let phrases: Vec<String> = rng
        .query()
        .into_iter()
        .map(|phrase| {
            let keep = 1 + rng.below(3);
            let cut: String = phrase
                .chars()
                .take(
                    phrase
                        .chars()
                        .count()
                        .saturating_sub(rng.below(3))
                        .max(keep),
                )
                .collect();
            cut
        })
        .collect();
    let expr = phrases
        .iter()
        .map(|p| format!("{}*", quoted(p)))
        .collect::<Vec<_>>()
        .join(" OR ");
    (
        Query::any_of_prefix(phrases.iter().map(String::as_str)),
        expr,
    )
}

/// `(a OR b) AND (c OR d)` or `(a OR b) NOT (c)`, each side plain or
/// prefix.
fn boolean_query(rng: &mut Lcg) -> Both {
    let (left, left_expr) = side(rng);
    let (right, right_expr) = side(rng);
    if rng.below(2) == 0 {
        (
            Query::all_of([left, right]),
            format!("({left_expr}) AND ({right_expr})"),
        )
    } else {
        (
            left.except(right),
            format!("({left_expr}) NOT ({right_expr})"),
        )
    }
}

/// `((a) AND (b)) NOT (c)`. FTS5 matches the same documents as this index
/// does but, in this nesting only, drops some phrase instances from the
/// row it scores (`(a OR b) AND c` scores differently with a `NOT "zzz"`
/// added that matches nothing), so scores and order are not comparable
/// here: only the set of matches is.
fn nested_query(rng: &mut Lcg) -> Both {
    let (left, left_expr) = side(rng);
    let (right, right_expr) = side(rng);
    let (third, third_expr) = side(rng);
    (
        Query::all_of([left, right]).except(third),
        format!("(({left_expr}) AND ({right_expr})) NOT ({third_expr})"),
    )
}

/// One side of a boolean query: mostly plain, sometimes prefix.
fn side(rng: &mut Lcg) -> Both {
    if rng.below(3) == 0 {
        prefix_query(rng)
    } else {
        plain_query(rng)
    }
}

/// What [`compare_rankings`] holds this index to.
#[derive(Clone, Copy)]
enum Compare {
    /// The same documents, in the same order, with the same scores.
    Ranked,
    /// The same documents.
    Matches,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Matches, order and scores for every query, over a corpus that also sees
/// replacements and deletions.
fn compare_rankings<const C: usize>(
    columns: [&str; C],
    seed: u64,
    docs: usize,
    queries: usize,
    shape: fn(&mut Lcg) -> Both,
    compare: Compare,
) {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE t USING fts5({});",
        columns.join(", ")
    ))
    .unwrap();
    let mut index: FullTextIndex<i64, C> = FullTextIndex::new();
    let mut rng = Lcg(seed);
    let insert = format!(
        "INSERT OR REPLACE INTO t(rowid, {}) VALUES (?1, {})",
        columns.join(", "),
        (2..=C + 1)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut write = |rng: &mut Lcg, rowid: i64| {
        let texts: Vec<String> = (0..C)
            .map(|c| {
                if c == C - 1 && C == 3 {
                    rng.tags()
                } else {
                    rng.prose(40)
                }
            })
            .collect();
        let refs: [&str; C] = std::array::from_fn(|i| texts[i].as_str());
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&rowid];
        for text in &texts {
            values.push(text);
        }
        conn.execute(&insert, values.as_slice()).unwrap();
        index.upsert(rowid, refs);
    };
    for rowid in 1..=docs as i64 {
        write(&mut rng, rowid);
    }
    // Replace some documents and delete others, as memories are edited.
    for _ in 0..docs / 5 {
        let rowid = 1 + rng.below(docs) as i64;
        write(&mut rng, rowid);
    }
    for _ in 0..docs / 10 {
        let rowid = 1 + rng.below(docs) as i64;
        conn.execute("DELETE FROM t WHERE rowid = ?1", params![rowid])
            .unwrap();
        index.remove(&rowid);
    }

    let mut compared = 0;
    for _ in 0..queries {
        let (query, expr) = shape(&mut rng);
        let expected: Vec<(i64, f64)> = conn
            .prepare("SELECT rowid, bm25(t) FROM t WHERE t MATCH ?1 ORDER BY bm25(t), rowid")
            .unwrap()
            .query_map(params![expr], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let got: Vec<(i64, f64)> = index
            .search(&query)
            .into_iter()
            .map(|hit| (hit.key, hit.score))
            .collect();
        let keys = |rows: &[(i64, f64)]| {
            let mut keys: Vec<i64> = rows.iter().map(|(k, _)| *k).collect();
            if matches!(compare, Compare::Matches) {
                keys.sort_unstable();
            }
            keys
        };
        assert_eq!(
            keys(&got),
            keys(&expected),
            "rows or order differ for {expr}"
        );
        if matches!(compare, Compare::Matches) {
            compared += expected.len();
            continue;
        }
        for ((key, ours), (_, theirs)) in got.iter().zip(&expected) {
            assert!(
                close(*ours, *theirs),
                "row {key} for {expr}: {ours} vs FTS5 {theirs}"
            );
        }
        compared += expected.len();
    }
    assert!(
        compared > queries,
        "the corpus should give most queries hits"
    );
}

#[test]
fn the_tokenizer_tables_come_from_the_sqlite_under_test() {
    assert_eq!(
        rusty_multimodal_db_engine::fulltext::unicode61::tables_sqlite_version(),
        rusqlite::version(),
        "regenerate src/fulltext/unicode_tables.rs (scripts/gen_unicode61_tables.py)"
    );
}

#[test]
fn memories_rank_as_fts5_ranks_them() {
    compare_rankings(
        ["content", "category", "tags"],
        0x5eed_0001,
        400,
        300,
        plain_query,
        Compare::Ranked,
    );
}

#[test]
fn wiki_pages_rank_as_fts5_ranks_them() {
    compare_rankings(
        ["title", "content"],
        0x5eed_0002,
        250,
        300,
        plain_query,
        Compare::Ranked,
    );
}

#[test]
fn prefix_queries_rank_as_fts5_ranks_them() {
    compare_rankings(
        ["title", "content"],
        0x5eed_0004,
        250,
        300,
        prefix_query,
        Compare::Ranked,
    );
}

#[test]
fn and_and_not_queries_rank_as_fts5_ranks_them() {
    compare_rankings(
        ["content", "category", "tags"],
        0x5eed_0005,
        400,
        400,
        boolean_query,
        Compare::Ranked,
    );
}

#[test]
fn nested_and_not_queries_match_the_documents_fts5_matches() {
    compare_rankings(
        ["content", "category", "tags"],
        0x5eed_0006,
        400,
        400,
        nested_query,
        Compare::Matches,
    );
}

#[test]
fn snippets_are_the_ones_fts5_writes() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE w USING fts5(title, content);")
        .unwrap();
    let mut index: FullTextIndex<i64, 2> = FullTextIndex::new();
    let mut rng = Lcg(0x5eed_0003);
    let mut texts = Vec::new();
    for rowid in 1..=150i64 {
        let title = rng.prose(6);
        let content = rng.prose(80);
        conn.execute(
            "INSERT INTO w(rowid, title, content) VALUES (?1, ?2, ?3)",
            params![rowid, title, content],
        )
        .unwrap();
        index.upsert(rowid, [title.as_str(), content.as_str()]);
        texts.push((title, content));
    }

    let mut compared = 0;
    for _ in 0..200 {
        let (query, expr) = match rng.below(3) {
            0 => plain_query(&mut rng),
            1 => prefix_query(&mut rng),
            _ => boolean_query(&mut rng),
        };
        let hits = index.search(&query);
        // The wiki's own call, then the other shapes snippet() takes.
        for (column, tokens) in [(1i64, 12usize), (-1, 12), (0, 3), (1, 1), (1, 40)] {
            let expected: Vec<(i64, String)> = conn
                .prepare(
                    "SELECT rowid, snippet(w, ?2, '[', ']', '…', ?3) FROM w
                      WHERE w MATCH ?1 ORDER BY bm25(w), rowid",
                )
                .unwrap()
                .query_map(params![expr, column, tokens as i64], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let style = SnippetStyle {
                open: "[",
                close: "]",
                ellipsis: "…",
                tokens,
            };
            let only = usize::try_from(column).ok();
            for (hit, (rowid, snippet)) in hits.iter().zip(&expected) {
                assert_eq!(hit.key, *rowid);
                let (title, content) = &texts[(*rowid - 1) as usize];
                let ours = index.snippet(&query, hit, [title, content], only, style);
                assert_eq!(
                    &ours, snippet,
                    "snippet(w, {column}, …, {tokens}) of row {rowid} for {expr}"
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 500);
}
