//! On-device semantic embedding.
//!
//! "Every conversation is embedded on-device and blended with keyword
//! results, so 'container stuck' finds 'pod stuck terminating'." No network
//! call, no server-side model, and — here — no downloaded model file either.
//!
//! Two embedders, chosen automatically:
//!
//! * [`LsaEmbedder`] is trained from the user's own indexed conversations by
//!   truncated SVD over a tf-idf term/document matrix. Terms that keep the
//!   same company end up with neighbouring vectors, which is what makes
//!   "container" retrieve "pod" — learned from that user's corpus rather than
//!   from a general-purpose model. It needs a corpus, so it appears after the
//!   first index pass.
//! * [`HashingEmbedder`] is the cold-start fallback: a hashed random
//!   projection over words and character 4-grams. It has no semantics, but it
//!   is robust to typos and morphology and is available with zero documents.
//!
//! See `docs/ARCHITECTURE.md` for why this differs from the shipped static
//! model the reviewed product uses.

pub mod linalg;

use linalg::{l2_normalize, Coo, Dense};
use rand::{Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const DEFAULT_DIM: usize = 128;
/// Below this rank the decomposition carries too little signal to beat the
/// lexical fallback.
const MIN_DIM: usize = 16;
const VOCAB_CAP: usize = 12_000;
const MIN_DOC_FREQ: usize = 2;
const OVERSAMPLE: usize = 10;
const POWER_ITERATIONS: usize = 2;
/// Below this many documents, an SVD has nothing to learn from and the
/// fallback is the honest choice.
pub const MIN_DOCS_TO_TRAIN: usize = 32;

/// Very common English and chat-transcript words carry no retrieval signal
/// and dominate the co-occurrence matrix if left in.
const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "are",
    "but",
    "not",
    "you",
    "all",
    "can",
    "her",
    "was",
    "one",
    "our",
    "out",
    "day",
    "get",
    "has",
    "him",
    "his",
    "how",
    "its",
    "new",
    "now",
    "old",
    "see",
    "two",
    "way",
    "who",
    "did",
    "yes",
    "his",
    "that",
    "this",
    "with",
    "from",
    "they",
    "will",
    "would",
    "there",
    "their",
    "what",
    "about",
    "which",
    "when",
    "make",
    "like",
    "time",
    "just",
    "know",
    "take",
    "into",
    "your",
    "some",
    "them",
    "than",
    "then",
    "look",
    "only",
    "come",
    "over",
    "also",
    "back",
    "after",
    "use",
    "user",
    "assistant",
    "here",
    "have",
    "been",
    "were",
    "does",
    "should",
    "could",
    "let",
    "need",
    "want",
    "well",
    "sure",
    "okay",
    "thanks",
    "please",
];

pub fn is_stopword(t: &str) -> bool {
    STOPWORDS.contains(&t)
}

/// Split text into retrieval tokens.
///
/// Identifiers matter as much as prose here, so `snake_case` and `camelCase`
/// are split into their parts *and* kept whole — searching "authmiddleware"
/// and "auth middleware" should both work.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_lowercase();
        if lower.len() >= 2 && lower.len() <= 40 && !is_stopword(&lower) {
            out.push(lower.clone());
        }
        // Sub-tokens from snake_case / camelCase / digit boundaries.
        if raw.len() > 3 {
            for part in split_identifier(raw) {
                if part.len() >= 3 && !is_stopword(&part) && part != lower {
                    out.push(part);
                }
            }
        }
    }
    out
}

fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch == '_' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_lowercase() || ch.is_numeric();
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

pub trait Embedder: Send + Sync {
    fn name(&self) -> &str;
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
    /// True when this embedder can actually relate different words to each
    /// other. The UI uses it to decide whether "found by meaning" is a claim
    /// it can honestly make.
    fn is_semantic(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Hashing embedder — cold start
// ---------------------------------------------------------------------------

pub struct HashingEmbedder {
    dim: usize,
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        HashingEmbedder { dim: DEFAULT_DIM }
    }
}

impl HashingEmbedder {
    pub fn new(dim: usize) -> Self {
        HashingEmbedder { dim }
    }

    fn add_feature(&self, feature: &str, weight: f32, out: &mut [f32]) {
        let digest = Sha256::digest(feature.as_bytes());
        // Four (index, sign) draws per feature: a sparse random projection.
        for chunk in 0..4 {
            let off = chunk * 5;
            let idx = u32::from_le_bytes([
                digest[off],
                digest[off + 1],
                digest[off + 2],
                digest[off + 3],
            ]) as usize
                % self.dim;
            let sign = if digest[off + 4] & 1 == 0 { 1.0 } else { -1.0 };
            out[idx] += sign * weight;
        }
    }
}

impl Embedder for HashingEmbedder {
    fn name(&self) -> &str {
        "hashing-v1"
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn is_semantic(&self) -> bool {
        false
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        for token in tokenize(text) {
            self.add_feature(&token, 1.0, &mut v);
            // Character 4-grams give partial credit for near-misses.
            let chars: Vec<char> = token.chars().collect();
            if chars.len() > 4 {
                for w in chars.windows(4) {
                    let gram: String = w.iter().collect();
                    self.add_feature(&gram, 0.3, &mut v);
                }
            }
        }
        l2_normalize(&mut v);
        v
    }
}

// ---------------------------------------------------------------------------
// LSA embedder — trained locally
// ---------------------------------------------------------------------------

pub struct LsaEmbedder {
    dim: usize,
    vocab: HashMap<String, u32>,
    idf: Vec<f32>,
    /// `vocab.len() × dim`, row-major.
    term_vectors: Vec<f32>,
    doc_count: usize,
}

impl LsaEmbedder {
    pub fn doc_count(&self) -> usize {
        self.doc_count
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Train from a corpus. Each item is one document's full text.
    ///
    /// Returns `None` when the corpus is too small to yield anything better
    /// than the hashing fallback.
    pub fn train(docs: &[String], dim: usize) -> Option<LsaEmbedder> {
        if docs.len() < MIN_DOCS_TO_TRAIN {
            return None;
        }

        // Pass 1: document frequency.
        let tokenized: Vec<Vec<String>> = docs.iter().map(|d| tokenize(d)).collect();
        let mut df: HashMap<&str, usize> = HashMap::new();
        for toks in &tokenized {
            let mut seen: Vec<&str> = toks.iter().map(|s| s.as_str()).collect();
            seen.sort_unstable();
            seen.dedup();
            for t in seen {
                *df.entry(t).or_insert(0) += 1;
            }
        }

        // Vocabulary: terms seen in at least MIN_DOC_FREQ documents but not in
        // essentially all of them, capped, most-frequent first.
        //
        // The ceiling is deliberately high. Shared context words are what
        // relate two terms that never co-occur — dropping "kubectl" and
        // "terminating" as too-common is exactly what would stop "container"
        // from ever reaching "pod". idf already discounts them; this filter
        // only needs to catch the boilerplate a tool stamps on every single
        // transcript.
        let ceiling = ((docs.len() as f64) * 0.9).ceil() as usize;
        let mut candidates: Vec<(&str, usize)> = df
            .into_iter()
            .filter(|(_, c)| *c >= MIN_DOC_FREQ && *c <= ceiling.max(MIN_DOC_FREQ))
            .collect();
        // A small or narrow corpus cannot support a high-rank decomposition,
        // but it can usually support a lower one. Fitting the rank to the
        // vocabulary beats refusing to train and falling back to a model with
        // no semantics at all.
        let dim = dim.min(candidates.len() / 2);
        if dim < MIN_DIM {
            return None;
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        candidates.truncate(VOCAB_CAP);

        let mut vocab: HashMap<String, u32> = HashMap::with_capacity(candidates.len());
        let mut idf = Vec::with_capacity(candidates.len());
        for (i, (term, count)) in candidates.iter().enumerate() {
            vocab.insert((*term).to_string(), i as u32);
            idf.push(((docs.len() as f32 + 1.0) / (*count as f32 + 1.0)).ln() + 1.0);
        }

        // Pass 2: sparse tf-idf term × doc matrix.
        let mut entries: Vec<(u32, u32, f32)> = Vec::new();
        for (doc_i, toks) in tokenized.iter().enumerate() {
            let mut tf: HashMap<u32, f32> = HashMap::new();
            for t in toks {
                if let Some(&ti) = vocab.get(t) {
                    *tf.entry(ti).or_insert(0.0) += 1.0;
                }
            }
            if tf.is_empty() {
                continue;
            }
            let mut norm = 0.0f32;
            let weighted: Vec<(u32, f32)> = tf
                .into_iter()
                .map(|(ti, count)| {
                    let w = (1.0 + count.ln()) * idf[ti as usize];
                    norm += w * w;
                    (ti, w)
                })
                .collect();
            let norm = norm.sqrt().max(1e-8);
            for (ti, w) in weighted {
                entries.push((ti, doc_i as u32, w / norm));
            }
        }
        if entries.is_empty() {
            return None;
        }

        let a = Coo {
            rows: vocab.len(),
            cols: docs.len(),
            entries,
        };

        let term_vectors = randomized_svd_terms(&a, dim)?;

        Some(LsaEmbedder {
            dim,
            vocab,
            idf,
            term_vectors,
            doc_count: docs.len(),
        })
    }
}

/// Truncated SVD of `a` (terms × docs), returning the top-`k` left singular
/// vectors scaled by the square root of their singular values, row-major
/// `terms × k`. Randomized range finder + small dense eigenproblem.
fn randomized_svd_terms(a: &Coo, k: usize) -> Option<Vec<f32>> {
    let sketch = (k + OVERSAMPLE).min(a.cols).min(a.rows);
    if sketch < 2 {
        return None;
    }

    // Fixed seed: indexing the same corpus twice must give the same vectors,
    // otherwise every re-index invalidates every stored embedding.
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(0x1_1EAF_5EED);
    let mut omega = Dense::zeros(a.cols, sketch);
    for v in omega.data.iter_mut() {
        *v = rng.gen_range(-1.0f32..1.0f32);
    }

    let mut y = a.mul_dense(&omega);
    linalg::orthonormalize_columns(&mut y);
    for _ in 0..POWER_ITERATIONS {
        let z = a.transpose_mul_dense(&y);
        y = a.mul_dense(&z);
        linalg::orthonormalize_columns(&mut y);
    }
    let q = y; // terms × sketch, orthonormal columns

    // B = Qᵀ A  (sketch × docs), then C = B Bᵀ (sketch × sketch).
    let mut c = Dense::zeros(sketch, sketch);
    {
        let mut b = Dense::zeros(sketch, a.cols);
        for &(r, col, v) in &a.entries {
            let qrow = q.row(r as usize);
            for s in 0..sketch {
                b.data[s * a.cols + col as usize] += qrow[s] * v;
            }
        }
        for i in 0..sketch {
            for j in i..sketch {
                let mut acc = 0.0f32;
                let (bi, bj) = (b.row(i), b.row(j));
                for d in 0..a.cols {
                    acc += bi[d] * bj[d];
                }
                c.data[i * sketch + j] = acc;
                c.data[j * sketch + i] = acc;
            }
        }
    }

    let (eigenvalues, eigenvectors) = linalg::symmetric_eigen(c);

    // U_k = Q · W_k, scaled by σ^{1/2} = λ^{1/4}.
    let k = k.min(sketch);
    let mut out = vec![0.0f32; q.rows * k];
    for t in 0..q.rows {
        let qrow = q.row(t);
        for j in 0..k {
            let mut acc = 0.0f32;
            for s in 0..sketch {
                acc += qrow[s] * eigenvectors.data[s * sketch + j];
            }
            let scale = eigenvalues[j].max(0.0).powf(0.25);
            out[t * k + j] = acc * scale;
        }
    }
    Some(out)
}

impl Embedder for LsaEmbedder {
    fn name(&self) -> &str {
        "lsa-v1"
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn is_semantic(&self) -> bool {
        true
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        let mut hits = 0usize;
        for token in tokenize(text) {
            if let Some(&ti) = self.vocab.get(&token) {
                let w = self.idf[ti as usize];
                let row = &self.term_vectors[ti as usize * self.dim..(ti as usize + 1) * self.dim];
                for (o, r) in v.iter_mut().zip(row) {
                    *o += w * r;
                }
                hits += 1;
            }
        }
        if hits == 0 {
            // Nothing in the trained vocabulary. Return zero rather than
            // anything else: a zero vector has cosine 0 against everything and
            // is filtered out downstream, so the semantic arm simply abstains.
            //
            // The tempting alternative — fall back to the hashing embedder —
            // is wrong, because its output lives in a different space from the
            // stored document vectors. Comparing across the two produces
            // arbitrary similarities that would then be shown to the user
            // under a "found by meaning" label. Abstaining is the honest
            // answer; keyword search still runs.
            return v;
        }
        l2_normalize(&mut v);
        v
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn take_u32(input: &[u8], at: &mut usize) -> Option<u32> {
    let bytes = input.get(*at..*at + 4)?;
    *at += 4;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

impl LsaEmbedder {
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_u32(&mut out, self.dim as u32);
        put_u32(&mut out, self.vocab.len() as u32);
        put_u32(&mut out, self.doc_count as u32);

        let mut ordered: Vec<(&String, &u32)> = self.vocab.iter().collect();
        ordered.sort_by_key(|(_, &i)| i);
        for (term, _) in ordered {
            put_u32(&mut out, term.len() as u32);
            out.extend_from_slice(term.as_bytes());
        }
        for w in &self.idf {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for w in &self.term_vectors {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Option<LsaEmbedder> {
        let mut at = 0usize;
        let dim = take_u32(bytes, &mut at)? as usize;
        let vocab_len = take_u32(bytes, &mut at)? as usize;
        let doc_count = take_u32(bytes, &mut at)? as usize;
        if dim == 0 || vocab_len == 0 {
            return None;
        }

        let mut vocab = HashMap::with_capacity(vocab_len);
        for i in 0..vocab_len {
            let len = take_u32(bytes, &mut at)? as usize;
            let term = std::str::from_utf8(bytes.get(at..at + len)?)
                .ok()?
                .to_string();
            at += len;
            vocab.insert(term, i as u32);
        }

        let read_floats = |count: usize, at: &mut usize| -> Option<Vec<f32>> {
            let need = count * 4;
            let slice = bytes.get(*at..*at + need)?;
            *at += need;
            Some(
                slice
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            )
        };

        let idf = read_floats(vocab_len, &mut at)?;
        let term_vectors = read_floats(vocab_len * dim, &mut at)?;

        Some(LsaEmbedder {
            dim,
            vocab,
            idf,
            term_vectors,
            doc_count,
        })
    }
}

pub fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_splits_identifiers_and_keeps_the_whole() {
        let toks = tokenize("refactor authMiddleware into shared_hook");
        assert!(toks.contains(&"authmiddleware".to_string()));
        assert!(toks.contains(&"auth".to_string()));
        assert!(toks.contains(&"middleware".to_string()));
        assert!(toks.contains(&"shared".to_string()));
        assert!(toks.contains(&"hook".to_string()));
        // Stopwords are dropped.
        assert!(!toks.contains(&"into".to_string()));
    }

    #[test]
    fn hashing_embedder_is_deterministic_and_normalised() {
        let e = HashingEmbedder::default();
        let a = e.embed("postgres index tuning");
        let b = e.embed("postgres index tuning");
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
        assert!(!e.is_semantic());
    }

    #[test]
    fn hashing_embedder_scores_related_text_above_unrelated() {
        let e = HashingEmbedder::default();
        let q = e.embed("postgres index tuning");
        let near = e.embed("tuning a postgres index");
        let far = e.embed("swift ui animation curves");
        assert!(
            linalg::cosine(&q, &near) > linalg::cosine(&q, &far),
            "lexical overlap should dominate"
        );
    }

    /// A corpus with three distinct topics. "container" and "pod" never appear
    /// in the same document, but the documents they appear in share the rest
    /// of their vocabulary — which is the only thing LSA has to go on.
    fn synthetic_corpus() -> Vec<String> {
        let filler = [
            "cluster node deployment restart",
            "namespace workload rollout status",
            "kubectl describe events pending",
            "drain evict finalizer graceful",
        ];
        let mut docs = Vec::new();
        for i in 0..40 {
            let f = filler[i % filler.len()];
            docs.push(format!("container stuck terminating {f} kubectl namespace"));
            docs.push(format!("pod stuck terminating {f} kubectl namespace"));
            docs.push(format!(
                "swift animation curve easing layout view controller gesture {}",
                filler[(i + 1) % filler.len()]
                    .split(' ')
                    .next()
                    .unwrap_or("x")
            ));
            docs.push(format!(
                "postgres index vacuum analyze planner query table sequential scan {}",
                i % 7
            ));
        }
        docs
    }

    /// The capability under test is the interesting one: two words that never
    /// appear together should still end up close when they keep the same
    /// company across a corpus.
    #[test]
    fn lsa_relates_words_that_share_context() {
        let docs = synthetic_corpus();
        let model = LsaEmbedder::train(&docs, 32).expect("corpus is large enough to train");
        assert!(model.is_semantic());

        let container = model.embed("container stuck");
        let pod = model.embed("pod terminating");
        let swift = model.embed("swift animation curve");

        let related = linalg::cosine(&container, &pod);
        let unrelated = linalg::cosine(&container, &swift);
        assert!(
            related > unrelated,
            "container/pod ({related}) should beat container/swift ({unrelated})"
        );
    }

    /// Out-of-vocabulary text must abstain rather than emit a vector from a
    /// different space, which would surface as a bogus "found by meaning" hit.
    #[test]
    fn lsa_abstains_on_out_of_vocabulary_text() {
        let model = LsaEmbedder::train(&synthetic_corpus(), 32).unwrap();
        let v = model.embed("zzzq wibblefrotz nonexistentterm");
        assert!(
            v.iter().all(|x| *x == 0.0),
            "expected a zero vector, got {:?}",
            &v[..4]
        );
        let real = model.embed("pod terminating");
        assert!(
            linalg::cosine(&v, &real).abs() < 1e-6,
            "an abstaining vector must not score against anything"
        );
    }

    #[test]
    fn lsa_declines_to_train_on_a_tiny_corpus() {
        let docs: Vec<String> = (0..5).map(|i| format!("doc number {i}")).collect();
        assert!(LsaEmbedder::train(&docs, 32).is_none());
    }

    #[test]
    fn lsa_survives_a_serialization_round_trip() {
        let docs = synthetic_corpus();
        let model = LsaEmbedder::train(&docs, 16).expect("trains");
        let restored = LsaEmbedder::deserialize(&model.serialize()).expect("round trips");
        assert_eq!(restored.vocab_size(), model.vocab_size());
        assert_eq!(restored.doc_count(), model.doc_count());
        let a = model.embed("vacuum analyze");
        let b = restored.embed("vacuum analyze");
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    fn vectors_round_trip_through_blobs() {
        let v = vec![0.5f32, -0.25, 0.125];
        assert_eq!(decode_vector(&encode_vector(&v)), v);
    }
}
