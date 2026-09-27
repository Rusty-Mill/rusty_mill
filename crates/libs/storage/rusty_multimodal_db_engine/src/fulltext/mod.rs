//! A full-text index that ranks as SQLite's FTS5 does (`rusty_remind_me`'s
//! ADR-0023 §3a), so a node moving off SQLite keeps its keyword search
//! exactly: same tokens, same matches, same `bm25()` scores in FTS5's sign
//! convention (lower is better), same `snippet()` text.
//!
//! What it covers is what the node asks of FTS5, no more: documents of a
//! fixed number of text columns, tokenized by [`unicode61`] with its
//! defaults, and queries that are any of several phrases (`"a b" OR "c"`).
//! A phrase matches where its tokens appear consecutively in one column.
//!
//! The index is derived data: it lives in memory and is rebuilt from the
//! records when a store opens, like the engine's ordered indexes, so it has
//! no durability of its own.
//!
//! Its behaviour is pinned by differential tests
//! (`tests/fulltext_vs_fts5.rs`): one corpus and set of queries through this
//! index and through FTS5, compared match for match, score for score and
//! snippet for snippet.

pub mod unicode61;
mod unicode_tables;

pub use unicode61::{tokenize, Token};

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// BM25's `k1`, as FTS5 fixes it.
const K1: f64 = 1.2;
/// BM25's `b`, as FTS5 fixes it.
const B: f64 = 0.75;

/// A query: any of these phrases, each already split into tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    phrases: Vec<Vec<String>>,
}

impl Query {
    /// Match any of `phrases`, each tokenized as a document is: what FTS5
    /// does with `"p1" OR "p2" OR ...`. A phrase with no tokens in it
    /// matches nothing, as in FTS5.
    pub fn any_of<'a>(phrases: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            phrases: phrases
                .into_iter()
                .map(|phrase| tokenize(phrase).into_iter().map(|t| t.text).collect())
                .collect(),
        }
    }

    pub fn phrases(&self) -> &[Vec<String>] {
        &self.phrases
    }
}

/// Where one phrase of the query matched: FTS5's `xInst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instance {
    pub column: usize,
    /// Token position of the phrase's first token within the column.
    pub offset: u32,
    /// Which of the query's phrases.
    pub phrase: usize,
}

/// A matching document.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit<K> {
    pub key: K,
    /// FTS5's `bm25()`: negative, and lower is a better match.
    pub score: f64,
    /// Every phrase match in the document, in column then position order.
    pub instances: Vec<Instance>,
}

/// How [`FullTextIndex::snippet`] marks up its excerpt: FTS5's `snippet()`
/// arguments after the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnippetStyle<'a> {
    pub open: &'a str,
    pub close: &'a str,
    pub ellipsis: &'a str,
    /// The most tokens the excerpt spans.
    pub tokens: usize,
}

#[derive(Debug, Clone)]
struct Doc {
    /// Tokens per column.
    sizes: Vec<u32>,
    /// Every distinct term, so removal knows which postings to visit.
    terms: Vec<String>,
}

/// An in-memory full-text index over documents of `C` text columns, keyed
/// by `K`.
#[derive(Debug, Clone)]
pub struct FullTextIndex<K, const C: usize> {
    docs: HashMap<K, Doc>,
    /// Term to document to `(column, position)`s, in order.
    postings: HashMap<String, HashMap<K, Vec<(u32, u32)>>>,
    total_tokens: u64,
}

impl<K: Clone + Eq + Hash + Ord, const C: usize> Default for FullTextIndex<K, C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + Hash + Ord, const C: usize> FullTextIndex<K, C> {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            postings: HashMap::new(),
            total_tokens: 0,
        }
    }

    /// Documents indexed.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Index `columns` as the document `key`, replacing what it held.
    pub fn upsert(&mut self, key: K, columns: [&str; C]) {
        self.remove(&key);
        let mut sizes = Vec::with_capacity(C);
        let mut positions: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
        for (column, text) in columns.iter().enumerate() {
            let tokens = tokenize(text);
            sizes.push(tokens.len() as u32);
            for (position, token) in tokens.into_iter().enumerate() {
                positions
                    .entry(token.text)
                    .or_default()
                    .push((column as u32, position as u32));
            }
        }
        self.total_tokens += sizes.iter().map(|s| u64::from(*s)).sum::<u64>();
        let terms: Vec<String> = positions.keys().cloned().collect();
        for (term, at) in positions {
            self.postings
                .entry(term)
                .or_default()
                .insert(key.clone(), at);
        }
        self.docs.insert(key, Doc { sizes, terms });
    }

    /// Drop the document `key`. Returns whether it was there.
    pub fn remove(&mut self, key: &K) -> bool {
        let Some(doc) = self.docs.remove(key) else {
            return false;
        };
        self.total_tokens -= doc.sizes.iter().map(|s| u64::from(*s)).sum::<u64>();
        for term in doc.terms {
            if let Some(by_doc) = self.postings.get_mut(&term) {
                by_doc.remove(key);
                if by_doc.is_empty() {
                    self.postings.remove(&term);
                }
            }
        }
        true
    }

    /// Every document matching `query`, best first (FTS5's `ORDER BY
    /// bm25(...)`), ties broken by key.
    pub fn search(&self, query: &Query) -> Vec<Hit<K>> {
        let matches: Vec<HashMap<&K, Vec<(u32, u32)>>> = query
            .phrases
            .iter()
            .map(|phrase| self.phrase_matches(phrase))
            .collect();
        let mut hits: BTreeMap<&K, Vec<Instance>> = BTreeMap::new();
        for (phrase, by_doc) in matches.iter().enumerate() {
            for (key, at) in by_doc {
                let instances = hits.entry(*key).or_default();
                instances.extend(at.iter().map(|&(column, offset)| Instance {
                    column: column as usize,
                    offset,
                    phrase,
                }));
            }
        }

        let rows = self.docs.len() as f64;
        let average = self.total_tokens as f64 / rows;
        // `fts5Bm25GetData`: an IDF per phrase from how many documents hold
        // it anywhere, floored just above zero.
        let idf: Vec<f64> = matches
            .iter()
            .map(|by_doc| {
                let with = by_doc.len() as f64;
                let idf = ((rows - with + 0.5) / (with + 0.5)).ln();
                if idf <= 0.0 {
                    1e-6
                } else {
                    idf
                }
            })
            .collect();

        let mut ranked: Vec<Hit<K>> = hits
            .into_iter()
            .map(|(key, mut instances)| {
                instances.sort_unstable();
                let size: u32 = self.docs[key].sizes.iter().sum();
                let score = bm25(&idf, &instances, f64::from(size), average);
                Hit {
                    key: key.clone(),
                    score,
                    instances,
                }
            })
            .collect();
        ranked.sort_by(|a, b| a.score.total_cmp(&b.score).then_with(|| a.key.cmp(&b.key)));
        ranked
    }

    /// Where `phrase` occurs, per document: the position of its first token.
    fn phrase_matches(&self, phrase: &[String]) -> HashMap<&K, Vec<(u32, u32)>> {
        let Some((first, rest)) = phrase.split_first() else {
            return HashMap::new();
        };
        let Some(starts) = self.postings.get(first) else {
            return HashMap::new();
        };
        let mut found = HashMap::new();
        'docs: for (key, at) in starts {
            let mut followers = Vec::with_capacity(rest.len());
            for term in rest {
                match self.postings.get(term).and_then(|d| d.get(key)) {
                    Some(positions) => followers.push(positions),
                    None => continue 'docs,
                }
            }
            let hits: Vec<(u32, u32)> = at
                .iter()
                .copied()
                .filter(|&(column, offset)| {
                    followers.iter().enumerate().all(|(i, positions)| {
                        positions
                            .binary_search(&(column, offset + 1 + i as u32))
                            .is_ok()
                    })
                })
                .collect();
            if !hits.is_empty() {
                found.insert(key, hits);
            }
        }
        found
    }

    /// FTS5's `snippet()`: an excerpt of at most `style.tokens` tokens from
    /// `column` (or the best column, for `None`), with each phrase match
    /// wrapped in `style.open` and `style.close`.
    ///
    /// `columns` must be the text `hit.key` was indexed with.
    pub fn snippet(
        &self,
        query: &Query,
        hit: &Hit<K>,
        columns: [&str; C],
        column: Option<usize>,
        style: SnippetStyle<'_>,
    ) -> String {
        let Some(doc) = self.docs.get(&hit.key) else {
            return String::new();
        };
        let phrase_size = |phrase: usize| query.phrases[phrase].len() as i64;
        let n_token = style.tokens as i64;
        let mut best_col = column.unwrap_or(0);
        let mut best_start = 0i64;
        let mut best_score = 0i64;
        let mut col_size = 0i64;

        for (i, text) in columns.iter().enumerate() {
            if column.is_some_and(|only| only != i) {
                continue;
            }
            let firsts = sentence_starts(text);
            let doc_size = i64::from(doc.sizes[i]);
            for inst in hit.instances.iter().filter(|inst| inst.column == i) {
                let io = i64::from(inst.offset);
                let (score, adjusted) =
                    snippet_score(&hit.instances, &phrase_size, doc_size, i, io, n_token);
                if score > best_score {
                    best_score = score;
                    best_col = i;
                    best_start = adjusted;
                    col_size = doc_size;
                }
                if !firsts.is_empty() && doc_size > n_token {
                    let mut jj = 0;
                    while jj + 1 < firsts.len() && firsts[jj + 1] <= io {
                        jj += 1;
                    }
                    if firsts[jj] < io {
                        let (score, _) = snippet_score(
                            &hit.instances,
                            &phrase_size,
                            doc_size,
                            i,
                            firsts[jj],
                            n_token,
                        );
                        let score = score + if firsts[jj] == 0 { 120 } else { 100 };
                        if score > best_score {
                            best_score = score;
                            best_col = i;
                            best_start = firsts[jj];
                            col_size = doc_size;
                        }
                    }
                }
            }
        }
        if col_size == 0 {
            col_size = i64::from(doc.sizes[best_col]);
        }
        highlight(
            columns[best_col],
            &hit.instances,
            &phrase_size,
            best_col,
            best_start,
            col_size,
            style,
        )
    }
}

/// `fts5Bm25Function` for one document.
fn bm25(idf: &[f64], instances: &[Instance], size: f64, average: f64) -> f64 {
    let mut freq = vec![0.0f64; idf.len()];
    for inst in instances {
        freq[inst.phrase] += 1.0;
    }
    let mut score = 0.0;
    for (i, idf) in idf.iter().enumerate() {
        score += idf * ((freq[i] * (K1 + 1.0)) / (freq[i] + K1 * (1.0 - B + B * size / average)));
    }
    -score
}

/// `fts5SentenceFinderCb`: the positions of tokens that start a sentence,
/// being the first token or one after a `.` or `:` and whitespace.
fn sentence_starts(text: &str) -> Vec<i64> {
    let bytes = text.as_bytes();
    let mut firsts = Vec::new();
    for (position, token) in tokenize(text).iter().enumerate() {
        if position == 0 {
            firsts.push(0);
            continue;
        }
        let before = bytes[..token.start]
            .iter()
            .rposition(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r'));
        let skipped_space = before.map_or(token.start > 0, |at| at + 1 != token.start);
        let ends_sentence = before.is_some_and(|at| matches!(bytes[at], b'.' | b':'));
        if skipped_space && ends_sentence {
            firsts.push(position as i64);
        }
    }
    firsts
}

/// `fts5SnippetScore`: how good a window of `n_token` tokens from `start`
/// is, and where to start it so its hits sit in the middle.
fn snippet_score(
    instances: &[Instance],
    phrase_size: &dyn Fn(usize) -> i64,
    doc_size: i64,
    column: usize,
    start: i64,
    n_token: i64,
) -> (i64, i64) {
    let end = start + n_token;
    let mut seen = HashMap::new();
    let (mut score, mut first, mut last) = (0i64, -1i64, 0i64);
    for inst in instances {
        let offset = i64::from(inst.offset);
        if inst.column == column && offset >= start && offset < end {
            score += if seen.insert(inst.phrase, ()).is_some() {
                1
            } else {
                1000
            };
            if first < 0 {
                first = offset;
            }
            last = offset + phrase_size(inst.phrase);
        }
    }
    let mut adjusted = first - (n_token - (last - first)) / 2;
    if adjusted + n_token > doc_size {
        adjusted = doc_size - n_token;
    }
    (score, adjusted.max(0))
}

/// `fts5CInstIter`: phrase matches in one column, overlapping ones merged.
struct Coalesced<'a> {
    instances: &'a [Instance],
    phrase_size: &'a dyn Fn(usize) -> i64,
    column: usize,
    next: usize,
    start: i64,
    end: i64,
}

impl Coalesced<'_> {
    fn advance(&mut self) {
        self.start = -1;
        self.end = -1;
        while let Some(inst) = self.instances.get(self.next) {
            if inst.column == self.column {
                let offset = i64::from(inst.offset);
                let end = offset - 1 + (self.phrase_size)(inst.phrase);
                if self.start < 0 {
                    self.start = offset;
                    self.end = end;
                } else if offset <= self.end {
                    self.end = self.end.max(end);
                } else {
                    break;
                }
            }
            self.next += 1;
        }
    }
}

/// `fts5HighlightCb` over `text`, limited to the window
/// `[range_start, range_start + style.tokens - 1]`, with the ellipses
/// `fts5SnippetFunction` adds around it.
fn highlight(
    text: &str,
    instances: &[Instance],
    phrase_size: &dyn Fn(usize) -> i64,
    column: usize,
    range_start: i64,
    col_size: i64,
    style: SnippetStyle<'_>,
) -> String {
    let range_end = range_start + style.tokens as i64 - 1;
    let mut iter = Coalesced {
        instances,
        phrase_size,
        column,
        next: 0,
        start: -1,
        end: -1,
    };
    iter.advance();
    let mut out = String::new();
    if range_start > 0 {
        out.push_str(style.ellipsis);
    }
    while iter.start >= 0 && iter.start < range_start {
        iter.advance();
    }
    let slice = |from: usize, to: usize| text.get(from..to.max(from)).unwrap_or("");
    let mut off = 0usize;
    let mut open = false;
    for (position, token) in tokenize(text).iter().enumerate() {
        let position = position as i64;
        if position < range_start || position > range_end {
            continue;
        }
        if range_start > 0 && position == range_start {
            off = token.start;
        }
        if open && (position <= iter.start || iter.start < 0) && token.start > off {
            out.push_str(style.close);
            open = false;
        }
        if position == iter.start && !open {
            out.push_str(slice(off, token.start));
            out.push_str(style.open);
            off = token.start;
            open = true;
        }
        if position == iter.end {
            if !open {
                out.push_str(style.open);
                open = true;
            }
            out.push_str(slice(off, token.end));
            off = token.end;
            iter.advance();
        }
        if position == range_end {
            if open {
                if iter.start >= 0 && position >= iter.start {
                    out.push_str(slice(off, token.end));
                    off = token.end;
                }
                out.push_str(style.close);
                open = false;
            }
            out.push_str(slice(off, token.end));
            off = token.end;
        }
    }
    if open {
        out.push_str(style.close);
    }
    if range_end >= col_size - 1 {
        out.push_str(slice(off, text.len()));
    } else {
        out.push_str(style.ellipsis);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(docs: &[(u32, [&str; 2])]) -> FullTextIndex<u32, 2> {
        let mut index = FullTextIndex::new();
        for (key, columns) in docs {
            index.upsert(*key, *columns);
        }
        index
    }

    #[test]
    fn a_phrase_needs_its_tokens_in_order_in_one_column() {
        let index = index(&[
            (1, ["memory tags here", ""]),
            (2, ["tags memory", ""]),
            (3, ["memory", "tags"]),
        ]);
        let hits = index.search(&Query::any_of(["memory_tags"]));
        assert_eq!(hits.iter().map(|h| h.key).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn any_phrase_matches_and_better_matches_rank_first() {
        let index = index(&[
            (1, ["fox fox fox", ""]),
            (2, ["a fox among many other words", ""]),
            (3, ["no match", ""]),
        ]);
        let hits = index.search(&Query::any_of(["fox", "nothing"]));
        assert_eq!(hits.iter().map(|h| h.key).collect::<Vec<_>>(), [1, 2]);
        assert!(hits[0].score < hits[1].score && hits[1].score < 0.0);
    }

    #[test]
    fn upsert_replaces_and_remove_forgets() {
        let mut index = index(&[(1, ["alpha", ""]), (2, ["beta", ""])]);
        index.upsert(1, ["gamma", ""]);
        assert!(index.search(&Query::any_of(["alpha"])).is_empty());
        assert_eq!(index.search(&Query::any_of(["gamma"]))[0].key, 1);
        assert!(index.remove(&2));
        assert!(!index.remove(&2));
        assert!(index.search(&Query::any_of(["beta"])).is_empty());
        assert_eq!(index.len(), 1);
        assert_eq!(index.total_tokens, 1);
    }

    #[test]
    fn an_empty_phrase_matches_nothing() {
        let index = index(&[(1, ["anything", ""])]);
        assert!(index.search(&Query::any_of(["_", "?!"])).is_empty());
    }

    #[test]
    fn sentence_starts_follow_full_stops_and_colons() {
        assert_eq!(sentence_starts("One two. Three: four.five"), [0, 2, 3]);
    }
}
