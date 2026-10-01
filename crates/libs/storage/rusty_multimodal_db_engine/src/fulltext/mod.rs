//! A full-text index that ranks as SQLite's FTS5 does (`rusty_remind_me`'s
//! ADR-0023 §3a), so a node moving off SQLite keeps its keyword search
//! exactly: same tokens, same matches, same `bm25()` scores in FTS5's sign
//! convention (lower is better), same `snippet()` text.
//!
//! What it covers is what the node asks of FTS5, no more: documents of a
//! fixed number of text columns, tokenized by [`unicode61`] with its
//! defaults, and queries that are any of several phrases (`"a b" OR "c"`).
//! A phrase matches where its tokens appear consecutively in one column.
//! Three additive query forms sit on top of that (the task-manager gaps of
//! issue #382): a prefix on a phrase's last token (`"ab cd"*`, for
//! type-ahead), `AND` of queries and `NOT`. Each is FTS5's own operator with
//! FTS5's own ranking, and is pinned by the same differential tests. One
//! nesting is the exception: for `(a AND b) NOT c` FTS5 matches the same
//! documents but drops some phrase instances when it scores them, so only
//! the matches are pinned there (`tests/fulltext_vs_fts5.rs`).
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

pub use unicode61::{for_each_token, tokenize, Token};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::ops::Bound;

/// BM25's `k1`, as FTS5 fixes it.
const K1: f64 = 1.2;
/// BM25's `b`, as FTS5 fixes it.
const B: f64 = 0.75;

/// A query. Built from [`Query::any_of`] (FTS5's `"p1" OR "p2"`), refined
/// with [`Query::any_of_prefix`], [`Query::all_of`] (`AND`),
/// [`Query::except`] (`NOT`), and [`Query::in_columns`] or
/// [`Query::except_columns`] (a column filter).
///
/// Every phrase that can match, in the order the query was built, is a
/// *leaf*; [`Instance::phrase`] numbers leaves. A phrase on the `NOT` side
/// is not a leaf: it removes documents and never appears in a hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    /// Any of `phrases`; with `prefix`, the last token of each is a prefix.
    Any {
        phrases: Vec<Vec<String>>,
        prefix: bool,
    },
    /// Documents matching every query.
    All(Vec<Query>),
    /// Documents matching `keep` and not `drop`.
    Except { keep: Box<Query>, drop: Box<Query> },
    /// `query`, with every phrase confined to `columns`.
    InColumns {
        query: Box<Query>,
        columns: ColumnSet,
    },
}

/// Which columns a filter admits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ColumnSet {
    /// Only these (FTS5's `{c1 c2} : q`).
    Only(Vec<usize>),
    /// Every column but these (FTS5's `- {c1 c2} : q`).
    Except(Vec<usize>),
}

impl ColumnSet {
    fn allows(&self, column: usize) -> bool {
        match self {
            ColumnSet::Only(columns) => columns.contains(&column),
            ColumnSet::Except(columns) => !columns.contains(&column),
        }
    }
}

impl Query {
    /// Match any of `phrases`, each tokenized as a document is: what FTS5
    /// does with `"p1" OR "p2" OR ...`. A phrase with no tokens in it
    /// matches nothing, as in FTS5.
    pub fn any_of<'a>(phrases: impl IntoIterator<Item = &'a str>) -> Self {
        Self::any(phrases, false)
    }

    /// [`Self::any_of`] where the last token of each phrase matches any term
    /// it is a prefix of: FTS5's `"p1"* OR "p2"*`, so `"quick br"` finds
    /// `quick brown`. Finding the terms scans the vocabulary, which is fine
    /// for a task or note corpus and is a cost to weigh for a very large
    /// one.
    pub fn any_of_prefix<'a>(phrases: impl IntoIterator<Item = &'a str>) -> Self {
        Self::any(phrases, true)
    }

    fn any<'a>(phrases: impl IntoIterator<Item = &'a str>, prefix: bool) -> Self {
        let phrases = phrases
            .into_iter()
            .map(|phrase| tokenize(phrase).into_iter().map(|t| t.text).collect())
            .collect();
        Self {
            expr: Expr::Any { phrases, prefix },
        }
    }

    /// Match documents that match every one of `queries`: FTS5's
    /// `(q1) AND (q2) AND ...`. No queries match nothing.
    pub fn all_of(queries: impl IntoIterator<Item = Query>) -> Self {
        Self {
            expr: Expr::All(queries.into_iter().collect()),
        }
    }

    /// Match documents that match `self` and do not match `drop`: FTS5's
    /// `(self) NOT (drop)`.
    pub fn except(self, drop: Query) -> Self {
        Self {
            expr: Expr::Except {
                keep: Box::new(self),
                drop: Box::new(drop),
            },
        }
    }

    /// Confine every phrase of `self` to `columns` (zero-based): FTS5's
    /// `{c1 c2} : (self)`. The filter applies to phrases, not to documents,
    /// so `all_of` under it needs each phrase in one of those columns. A
    /// column the index does not have matches nothing. Filters nest by
    /// intersection.
    pub fn in_columns(self, columns: impl IntoIterator<Item = usize>) -> Self {
        Self {
            expr: Expr::InColumns {
                query: Box::new(self),
                columns: ColumnSet::Only(columns.into_iter().collect()),
            },
        }
    }

    /// Confine every phrase of `self` to every column *but* `columns`:
    /// FTS5's `- {c1 c2} : (self)`. Otherwise as [`Self::in_columns`]; an
    /// exclusion of columns the index does not have excludes nothing.
    pub fn except_columns(self, columns: impl IntoIterator<Item = usize>) -> Self {
        Self {
            expr: Expr::InColumns {
                query: Box::new(self),
                columns: ColumnSet::Except(columns.into_iter().collect()),
            },
        }
    }

    /// [`Self::in_columns`] for a single column.
    pub fn in_column(self, column: usize) -> Self {
        self.in_columns([column])
    }

    /// The phrases of a plain [`Self::any_of`] or [`Self::any_of_prefix`]
    /// query, already split into tokens. Empty for an `AND` or `NOT` query;
    /// [`Self::leaves`] is the general form.
    pub fn phrases(&self) -> &[Vec<String>] {
        match &self.expr {
            Expr::Any { phrases, .. } => phrases,
            Expr::All(_) | Expr::Except { .. } | Expr::InColumns { .. } => &[],
        }
    }

    /// Every phrase that can appear in a hit, in [`Instance::phrase`] order.
    pub fn leaves(&self) -> Vec<&[String]> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a [String]>) {
        match &self.expr {
            Expr::Any { phrases, .. } => out.extend(phrases.iter().map(Vec::as_slice)),
            Expr::All(queries) => queries.iter().for_each(|q| q.collect_leaves(out)),
            Expr::Except { keep, .. } => keep.collect_leaves(out),
            Expr::InColumns { query, .. } => query.collect_leaves(out),
        }
    }
}

/// What a query matched: the documents, and where each leaf phrase sits.
struct Matched {
    docs: HashSet<u32>,
    /// One entry per leaf, in [`Query::leaves`] order.
    leaves: Vec<HashMap<u32, Vec<(u32, u32)>>>,
}

/// Keep only the matches every filter admits, and only the documents left
/// with one.
fn confine(by_doc: &mut HashMap<u32, Vec<(u32, u32)>>, filters: &[&ColumnSet]) {
    by_doc.retain(|_, at| {
        at.retain(|&(column, _)| filters.iter().all(|f| f.allows(column as usize)));
        !at.is_empty()
    });
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

/// One indexed document.
#[derive(Debug, Clone)]
struct Doc {
    /// Tokens per column.
    sizes: Box<[u32]>,
    /// Its distinct terms, by id, ascending.
    terms: Box<[u32]>,
    /// Where each of `terms` starts in `occurrences`, plus the end.
    starts: Box<[u32]>,
    /// `(column, position)` of every token, grouped by term in `terms`
    /// order, each group in order.
    occurrences: Box<[(u32, u32)]>,
}

impl Doc {
    fn size(&self) -> u32 {
        self.sizes.iter().sum()
    }

    /// Where `term` occurs, in column then position order.
    fn positions(&self, term: u32) -> Option<&[(u32, u32)]> {
        let i = self.terms.binary_search(&term).ok()?;
        Some(&self.occurrences[self.starts[i] as usize..self.starts[i + 1] as usize])
    }
}

/// An in-memory full-text index over documents of `C` text columns, keyed
/// by `K`.
///
/// Laid out for size, since a node holds one over every memory: each term
/// is stored once and numbered, each document gets a slot number, postings
/// are sorted slot lists, and a document's positions sit in one block.
/// Keys and term text are never copied per posting.
#[derive(Debug, Clone)]
pub struct FullTextIndex<K, const C: usize> {
    /// Each document's slot.
    slots: HashMap<K, u32>,
    /// Slot to key and document; `None` is a free slot.
    docs: Vec<Option<(K, Doc)>>,
    /// Slots [`remove`](Self::remove) freed, reused before new ones.
    free: Vec<u32>,
    /// Each term's id. Ids are never reused, so the vocabulary only grows,
    /// by the distinct terms ever indexed. Sorted, so a prefix query reads
    /// the range of terms it covers rather than every term.
    term_ids: BTreeMap<Box<str>, u32>,
    /// Term id to the slots of the documents holding it, ascending.
    postings: Vec<Vec<u32>>,
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
            slots: HashMap::new(),
            docs: Vec::new(),
            free: Vec::new(),
            term_ids: BTreeMap::new(),
            postings: Vec::new(),
            total_tokens: 0,
        }
    }

    /// Documents indexed.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Index `columns` as the document `key`, replacing what it held.
    pub fn upsert(&mut self, key: K, columns: [&str; C]) {
        self.remove(&key);
        let mut sizes = Vec::with_capacity(C);
        let mut tokens: Vec<(u32, u32, u32)> = Vec::new();
        for (column, text) in columns.iter().enumerate() {
            let mut position = 0u32;
            for_each_token(text, |term, _, _| {
                let term = self.term_id(term);
                tokens.push((term, column as u32, position));
                position += 1;
            });
            sizes.push(position);
        }
        tokens.sort_unstable();

        let mut terms = Vec::new();
        let mut starts = Vec::new();
        for (i, &(term, _, _)) in tokens.iter().enumerate() {
            if terms.last() != Some(&term) {
                terms.push(term);
                starts.push(i as u32);
            }
        }
        starts.push(tokens.len() as u32);
        let doc = Doc {
            sizes: sizes.into(),
            terms: terms.into(),
            starts: starts.into(),
            occurrences: tokens.iter().map(|&(_, c, p)| (c, p)).collect(),
        };

        let slot = self.take_slot();
        for &term in doc.terms.iter() {
            let list = &mut self.postings[term as usize];
            if let Err(at) = list.binary_search(&slot) {
                list.insert(at, slot);
            }
        }
        self.total_tokens += u64::from(doc.size());
        self.slots.insert(key.clone(), slot);
        self.docs[slot as usize] = Some((key, doc));
    }

    /// The id of `term`, numbering it if it is new.
    fn term_id(&mut self, term: &str) -> u32 {
        if let Some(&id) = self.term_ids.get(term) {
            return id;
        }
        let id = self.postings.len() as u32;
        self.postings.push(Vec::new());
        self.term_ids.insert(term.into(), id);
        id
    }

    /// A free slot, or a new one.
    fn take_slot(&mut self) -> u32 {
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        self.docs.push(None);
        (self.docs.len() - 1) as u32
    }

    /// Drop the document `key`. Returns whether it was there.
    pub fn remove(&mut self, key: &K) -> bool {
        let Some(slot) = self.slots.remove(key) else {
            return false;
        };
        let Some((_, doc)) = self.docs[slot as usize].take() else {
            return false;
        };
        self.total_tokens -= u64::from(doc.size());
        for &term in doc.terms.iter() {
            let list = &mut self.postings[term as usize];
            if let Ok(at) = list.binary_search(&slot) {
                list.remove(at);
            }
            if list.is_empty() {
                list.shrink_to_fit();
            }
        }
        self.free.push(slot);
        true
    }

    /// The document in `slot`. Postings hold only live slots.
    fn doc(&self, slot: u32) -> Option<&(K, Doc)> {
        self.docs.get(slot as usize)?.as_ref()
    }

    /// Every document matching `query`, best first (FTS5's `ORDER BY
    /// bm25(...)`), ties broken by key.
    pub fn search(&self, query: &Query) -> Vec<Hit<K>> {
        let Matched { docs, leaves } = self.evaluate(query, &[]);
        let mut hits: BTreeMap<&K, (u32, Vec<Instance>)> = BTreeMap::new();
        for (phrase, by_doc) in leaves.iter().enumerate() {
            for (&slot, at) in by_doc {
                if !docs.contains(&slot) {
                    continue;
                }
                let Some((key, doc)) = self.doc(slot) else {
                    continue;
                };
                let (_, instances) = hits.entry(key).or_insert_with(|| (doc.size(), Vec::new()));
                instances.extend(at.iter().map(|&(column, offset)| Instance {
                    column: column as usize,
                    offset,
                    phrase,
                }));
            }
        }

        let rows = self.slots.len() as f64;
        let average = self.total_tokens as f64 / rows;
        // `fts5Bm25GetData`: an IDF per phrase from how many documents hold
        // it anywhere, floored just above zero.
        let idf: Vec<f64> = leaves
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
            .map(|(key, (size, mut instances))| {
                instances.sort_unstable();
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

    /// The documents `query` matches and where each of its leaf phrases
    /// occurs. A leaf's positions cover every document holding it, not only
    /// those the whole query keeps: BM25's IDF counts the former.
    fn evaluate(&self, query: &Query, filters: &[&ColumnSet]) -> Matched {
        match &query.expr {
            Expr::Any { phrases, prefix } => {
                let leaves: Vec<_> = phrases
                    .iter()
                    .map(|phrase| {
                        let mut by_doc = if *prefix {
                            self.prefix_matches(phrase)
                        } else {
                            self.phrase_matches(phrase)
                        };
                        confine(&mut by_doc, filters);
                        by_doc
                    })
                    .collect();
                let docs = leaves
                    .iter()
                    .flat_map(|by_doc| by_doc.keys().copied())
                    .collect();
                Matched { docs, leaves }
            }
            Expr::All(queries) => {
                let mut parts = queries.iter().map(|q| self.evaluate(q, filters));
                let Some(first) = parts.next() else {
                    return Matched {
                        docs: HashSet::new(),
                        leaves: Vec::new(),
                    };
                };
                parts.fold(first, |mut all, part| {
                    all.docs.retain(|slot| part.docs.contains(slot));
                    all.leaves.extend(part.leaves);
                    all
                })
            }
            Expr::Except { keep, drop } => {
                let mut kept = self.evaluate(keep, filters);
                let dropped = self.evaluate(drop, filters);
                kept.docs.retain(|slot| !dropped.docs.contains(slot));
                kept
            }
            Expr::InColumns { query, columns } => {
                // Nested filters intersect: a phrase must pass every one.
                let mut nested = filters.to_vec();
                nested.push(columns);
                self.evaluate(query, &nested)
            }
        }
    }

    /// [`Self::phrase_matches`] with the last token taken as a prefix: the
    /// union over every term it is a prefix of. The terms are one range of
    /// the sorted vocabulary, and the phrase's other tokens are resolved once.
    fn prefix_matches(&self, phrase: &[String]) -> HashMap<u32, Vec<(u32, u32)>> {
        let mut found: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        let Some((last, head)) = phrase.split_last() else {
            return found;
        };
        let Some(mut ids) = self.term_ids_of(head) else {
            return found;
        };
        let terms = self
            .term_ids
            .range::<str, _>((Bound::Included(last.as_str()), Bound::Unbounded))
            .take_while(|(term, _)| term.starts_with(last.as_str()));
        for (_, &id) in terms {
            ids.push(id);
            self.collect_phrase(&ids, &mut found);
            ids.pop();
        }
        for at in found.values_mut() {
            at.sort_unstable();
        }
        found
    }

    /// Where `phrase` occurs, per document slot: the position of its first
    /// token.
    fn phrase_matches(&self, phrase: &[String]) -> HashMap<u32, Vec<(u32, u32)>> {
        let mut found = HashMap::new();
        if let Some(ids) = self.term_ids_of(phrase) {
            self.collect_phrase(&ids, &mut found);
        }
        found
    }

    /// Each term's id, or `None` if any is in no document (the phrase then
    /// matches nothing).
    fn term_ids_of(&self, terms: &[String]) -> Option<Vec<u32>> {
        terms
            .iter()
            .map(|term| self.term_ids.get(term.as_str()).copied())
            .collect()
    }

    /// Adds to `found` where the term-id phrase `ids` occurs: per document
    /// slot, the position of its first token.
    fn collect_phrase(&self, ids: &[u32], found: &mut HashMap<u32, Vec<(u32, u32)>>) {
        let Some((&first, rest)) = ids.split_first() else {
            return;
        };
        'docs: for &slot in &self.postings[first as usize] {
            let Some((_, doc)) = self.doc(slot) else {
                continue;
            };
            let Some(at) = doc.positions(first) else {
                continue;
            };
            let mut followers = Vec::with_capacity(rest.len());
            for &term in rest {
                match doc.positions(term) {
                    Some(positions) => followers.push(positions),
                    None => continue 'docs,
                }
            }
            let mut hits = at
                .iter()
                .copied()
                .filter(|&(column, offset)| {
                    followers.iter().enumerate().all(|(i, positions)| {
                        positions
                            .binary_search(&(column, offset + 1 + i as u32))
                            .is_ok()
                    })
                })
                .peekable();
            if hits.peek().is_some() {
                found.entry(slot).or_default().extend(hits);
            }
        }
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
        let Some((_, doc)) = self.slots.get(&hit.key).and_then(|&slot| self.doc(slot)) else {
            return String::new();
        };
        let leaves = query.leaves();
        let phrase_size = |phrase: usize| leaves[phrase].len() as i64;
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
    fn a_freed_slot_holds_the_next_document_alone() {
        let mut index = index(&[(1, ["alpha beta", ""]), (2, ["beta gamma", ""])]);
        assert!(index.remove(&1));
        index.upsert(3, ["delta beta", ""]);
        assert_eq!(index.docs.len(), 2, "slot 0 is reused");
        assert!(index.search(&Query::any_of(["alpha"])).is_empty());
        let keys = |q: &str| {
            let mut keys: Vec<u32> = index
                .search(&Query::any_of([q]))
                .iter()
                .map(|h| h.key)
                .collect();
            keys.sort_unstable();
            keys
        };
        assert_eq!(keys("beta"), [2, 3]);
        assert_eq!(keys("delta beta"), [3]);
        assert_eq!(index.total_tokens, 4);
    }

    #[test]
    fn a_phrase_with_an_unknown_term_matches_nothing() {
        let index = index(&[(1, ["alpha beta", ""])]);
        assert!(index.search(&Query::any_of(["alpha zzz"])).is_empty());
        assert!(index.search(&Query::any_of(["zzz alpha"])).is_empty());
    }

    fn keys(index: &FullTextIndex<u32, 2>, query: &Query) -> Vec<u32> {
        let mut keys: Vec<u32> = index.search(query).iter().map(|h| h.key).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn a_prefix_matches_the_start_of_the_last_token_only() {
        let index = index(&[
            (1, ["quick brown fox", ""]),
            (2, ["quickly bronze", ""]),
            (3, ["brown quick", ""]),
        ]);
        // Type-ahead: the last token is unfinished, the ones before are whole.
        assert_eq!(keys(&index, &Query::any_of_prefix(["quick br"])), [1]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["quick"])), [1, 2, 3]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["quic br"])), []);
        // Without the prefix form the same text is whole tokens.
        assert_eq!(keys(&index, &Query::any_of(["quick"])), [1, 3]);
    }

    /// A prefix reads one range of the sorted vocabulary: every term that
    /// starts with it, including the prefix itself, and none either side.
    #[test]
    fn a_prefix_reads_exactly_its_range_of_the_vocabulary() {
        let index = index(&[
            (1, ["gq", ""]),
            (2, ["gr", ""]),
            (3, ["gra grz", ""]),
            (4, ["gs", ""]),
            (5, ["g", "h"]),
            (6, ["über ubiquitous", ""]),
        ]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["gr"])), [2, 3]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["g"])), [1, 2, 3, 4, 5]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["grzz"])), []);
        assert_eq!(keys(&index, &Query::any_of_prefix(["ü"])), [6]);
        assert_eq!(keys(&index, &Query::any_of_prefix(["gra g"])), [3]);
    }

    #[test]
    fn a_prefix_with_no_tokens_matches_nothing() {
        let index = index(&[(1, ["anything", ""])]);
        assert!(index.search(&Query::any_of_prefix(["_", ""])).is_empty());
    }

    #[test]
    fn all_of_needs_every_query_and_none_needs_nothing_to_match() {
        let index = index(&[
            (1, ["red apple", "fruit"]),
            (2, ["red car", "vehicle"]),
            (3, ["green apple", "fruit"]),
        ]);
        let both = Query::all_of([Query::any_of(["red"]), Query::any_of(["apple", "car"])]);
        assert_eq!(keys(&index, &both), [1, 2]);
        let narrow = Query::all_of([Query::any_of(["red"]), Query::any_of(["apple"])]);
        assert_eq!(keys(&index, &narrow), [1]);
        assert!(index.search(&Query::all_of([])).is_empty());
    }

    #[test]
    fn except_removes_documents_and_leaves_no_instances_of_its_phrase() {
        let index = index(&[
            (1, ["red apple", ""]),
            (2, ["red car", ""]),
            (3, ["red apple car", ""]),
        ]);
        let query = Query::any_of(["red"]).except(Query::any_of(["car"]));
        let hits = index.search(&query);
        assert_eq!(hits.iter().map(|h| h.key).collect::<Vec<_>>(), [1]);
        assert!(hits[0].instances.iter().all(|i| i.phrase == 0));
        assert_eq!(query.leaves().len(), 1);
    }

    #[test]
    fn a_column_filter_confines_phrases_to_its_columns() {
        let index = index(&[
            (1, ["red", "apple"]),
            (2, ["apple", "red"]),
            (3, ["red apple", "green"]),
        ]);
        let red = || Query::any_of(["red"]);
        assert_eq!(keys(&index, &red().in_column(0)), [1, 3]);
        assert_eq!(keys(&index, &red().in_column(1)), [2]);
        assert_eq!(keys(&index, &red().in_columns([0, 1])), [1, 2, 3]);
        // A column the index lacks matches nothing.
        assert!(index.search(&red().in_column(2)).is_empty());
        // The filter is on phrases: both must sit in the chosen column.
        let both = Query::all_of([red(), Query::any_of(["apple"])]);
        assert_eq!(keys(&index, &both.clone().in_column(0)), [3]);
        assert_eq!(keys(&index, &both), [1, 2, 3]);
        // Instances outside the filter are not reported.
        let hits = index.search(&red().in_column(1));
        assert!(hits[0].instances.iter().all(|i| i.column == 1));
    }

    #[test]
    fn nested_column_filters_intersect() {
        let index = index(&[(1, ["red", "red"]), (2, ["green", "red"])]);
        let query = Query::any_of(["red"]).in_columns([0, 1]).in_column(1);
        assert_eq!(keys(&index, &query), [1, 2]);
        let narrow = Query::any_of(["red"]).in_column(0).in_column(1);
        assert!(index.search(&narrow).is_empty());
    }

    #[test]
    fn a_column_exclusion_admits_every_other_column() {
        let index = index(&[
            (1, ["red", "apple"]),
            (2, ["apple", "red"]),
            (3, ["red apple", "red"]),
        ]);
        let red = || Query::any_of(["red"]);
        assert_eq!(keys(&index, &red().except_columns([0])), [2, 3]);
        assert_eq!(keys(&index, &red().except_columns([1])), [1, 3]);
        assert!(index.search(&red().except_columns([0, 1])).is_empty());
        // A column the index lacks excludes nothing.
        assert_eq!(keys(&index, &red().except_columns([5])), [1, 2, 3]);
        // Exclusion and inclusion intersect when nested.
        let both = red().in_columns([0, 1]).except_columns([0]);
        assert_eq!(keys(&index, &both), [2, 3]);
    }

    #[test]
    fn sentence_starts_follow_full_stops_and_colons() {
        assert_eq!(sentence_starts("One two. Three: four.five"), [0, 2, 3]);
    }
}
