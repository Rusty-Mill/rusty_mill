//! `[cache]` -- an opt-in, in-memory cache of non-streaming
//! `Router::dispatch` responses, in one of two modes (see `CacheMode`):
//! `ResponseCache` (`"exact"`, the default) hashes the entire incoming
//! request, so any difference at all misses; `SemanticCache`
//! (`"semantic"`) fuzzes only the message text, via embedding-cosine-
//! similarity, while still requiring every other field to match exactly.
//!
//! Both are scoped to `dispatch` only, not `dispatch_stream`: faithfully
//! replaying a stored response as a fresh SSE chunk sequence is
//! meaningfully more work than returning a stored `ChatResponse` as-is,
//! and out of scope for this first version.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use rp_core::{ChatRequest, ChatResponse};

use crate::config::CacheConfig;

/// Fixed-capacity, insertion-order-evicting, TTL-bounded cache of
/// `ChatResponse`s keyed by request hash. Not a general-purpose LRU --
/// same "insertion order only, no read-refresh" tradeoff `GenerationCache`
/// already makes, plus a TTL check on read.
pub(crate) struct ResponseCache {
    ttl: Duration,
    max_entries: usize,
    order: VecDeque<u64>,
    entries: HashMap<u64, (Instant, ChatResponse)>,
}

impl ResponseCache {
    pub(crate) fn new(config: &CacheConfig) -> Self {
        Self {
            ttl: Duration::from_secs(config.ttl_secs),
            max_entries: config.max_entries.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// `req`'s cache key -- a hash of its full JSON serialization, so
    /// this is exact-match on every field without needing `ChatRequest`
    /// to implement `Hash` itself (several fields are `f32`/`f64`, which
    /// don't). Two requests that serialize identically always hash
    /// identically; a 64-bit hash carries a theoretical (astronomically
    /// unlikely) collision risk between two *different* requests, the
    /// same tradeoff the issue's own suggested design ("request-hash
    /// keyed") accepts.
    pub(crate) fn key_for(req: &ChatRequest) -> u64 {
        let json = serde_json::to_string(req).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        json.hash(&mut hasher);
        hasher.finish()
    }

    /// `None` for a miss -- either nothing was ever cached under `key`,
    /// or it was but has since aged out of `ttl`. An expired entry is
    /// removed on lookup rather than waiting for eviction, so a
    /// long-idle cache doesn't hold stale entries indefinitely just
    /// because nothing pushed them out.
    pub(crate) fn get(&mut self, key: u64) -> Option<ChatResponse> {
        let (inserted_at, resp) = self.entries.get(&key)?;
        if inserted_at.elapsed() > self.ttl {
            self.entries.remove(&key);
            return None;
        }
        Some(resp.clone())
    }

    pub(crate) fn insert(&mut self, key: u64, resp: ChatResponse) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key);
            if self.order.len() > self.max_entries {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, (Instant::now(), resp));
    }
}

/// Cosine similarity between two equal-length embedding vectors, in
/// `[-1.0, 1.0]` (in practice `[0.0, 1.0]` for the non-negative embedding
/// spaces every mainstream text-embedding model produces). `0.0` for a
/// dimension mismatch or a zero vector -- neither should happen with a
/// single embedding model configured consistently, but this fails safe
/// (a similarity of exactly nothing, never a match) rather than panicking
/// on a `debug_assert`-style invariant a config/provider change could
/// silently violate.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}

struct SemanticEntry {
    /// Hash of every request field *except* `messages` -- two requests
    /// only ever compare against each other when this matches, so
    /// "semantic" only ever fuzzes message content, never the model,
    /// sampling params, tools, or `provider` prefs.
    scope_key: u64,
    embedding: Vec<f32>,
    response: ChatResponse,
    inserted_at: Instant,
}

/// `[cache].mode = "semantic"` backing store. Embedding vectors are
/// computed by the caller (`Router::embed_for_cache`, via this router's
/// own `/v1/embeddings` dispatch path) and passed in already-computed --
/// this type has no knowledge of providers or async I/O, only comparison
/// and eviction, the same "pure logic, caller does the I/O" split
/// `ResponseCache::key_for` already uses.
pub(crate) struct SemanticCache {
    ttl: Duration,
    max_entries: usize,
    threshold: f64,
    entries: VecDeque<SemanticEntry>,
}

impl SemanticCache {
    pub(crate) fn new(config: &CacheConfig) -> Self {
        Self {
            ttl: Duration::from_secs(config.ttl_secs),
            max_entries: config.max_entries.max(1),
            threshold: config.similarity_threshold,
            entries: VecDeque::new(),
        }
    }

    /// Same hash-of-JSON-serialization approach as `ResponseCache::key_for`,
    /// but with `messages` cleared first, so the hash covers every other
    /// field (model, sampling params, tools, `provider` prefs, ...)
    /// without message content ever affecting it.
    pub(crate) fn scope_key_for(req: &ChatRequest) -> u64 {
        let mut scoped = req.clone();
        scoped.messages = Vec::new();
        let json = serde_json::to_string(&scoped).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        json.hash(&mut hasher);
        hasher.finish()
    }

    /// Best-similarity match within `scope_key` whose cosine similarity
    /// to `query_embedding` meets `threshold`, or `None` if nothing in
    /// scope clears the bar (including "nothing in scope at all"). Expired
    /// entries are swept out on every lookup, the same expire-on-read
    /// behavior `ResponseCache::get` already has, rather than only at
    /// insert-time eviction.
    pub(crate) fn get(&mut self, scope_key: u64, query_embedding: &[f32]) -> Option<ChatResponse> {
        let ttl = self.ttl;
        self.entries.retain(|e| e.inserted_at.elapsed() <= ttl);

        self.entries
            .iter()
            .filter(|e| e.scope_key == scope_key)
            .map(|e| (cosine_similarity(&e.embedding, query_embedding), e))
            .filter(|(similarity, _)| *similarity >= self.threshold)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, e)| e.response.clone())
    }

    pub(crate) fn insert(&mut self, scope_key: u64, embedding: Vec<f32>, resp: ChatResponse) {
        self.entries.push_back(SemanticEntry {
            scope_key,
            embedding,
            response: resp,
            inserted_at: Instant::now(),
        });
        if self.entries.len() > self.max_entries {
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_core::{ChatMessage, Choice};

    fn config(ttl_secs: u64, max_entries: usize) -> CacheConfig {
        CacheConfig {
            ttl_secs,
            max_entries,
            mode: crate::config::CacheMode::Exact,
            similarity_threshold: 0.85,
            embedding_model: None,
        }
    }

    fn request(model: &str, text: &str) -> ChatRequest {
        serde_json::from_value(serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": text}]
        }))
        .unwrap()
    }

    fn response(id: &str) -> ChatResponse {
        ChatResponse {
            id: id.to_string(),
            object: "chat.completion",
            created: 0,
            model: "anthropic/m1".to_string(),
            choices: vec![Choice {
                index: 0,
                message: ChatMessage::assistant("ok"),
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: None,
            cost_usd: None,
        }
    }

    // --- key_for ---------------------------------------------------------------

    #[test]
    fn key_for_is_identical_for_identical_requests() {
        let a = request("anthropic/m1", "hi");
        let b = request("anthropic/m1", "hi");
        assert_eq!(ResponseCache::key_for(&a), ResponseCache::key_for(&b));
    }

    #[test]
    fn key_for_differs_when_the_message_text_differs() {
        let a = request("anthropic/m1", "hi");
        let b = request("anthropic/m1", "bye");
        assert_ne!(ResponseCache::key_for(&a), ResponseCache::key_for(&b));
    }

    #[test]
    fn key_for_differs_when_the_model_differs() {
        let a = request("anthropic/m1", "hi");
        let b = request("anthropic/m2", "hi");
        assert_ne!(ResponseCache::key_for(&a), ResponseCache::key_for(&b));
    }

    #[test]
    fn key_for_differs_when_a_sampling_param_differs() {
        let mut a = request("anthropic/m1", "hi");
        let mut b = request("anthropic/m1", "hi");
        a.temperature = Some(0.2);
        b.temperature = Some(0.9);
        assert_ne!(ResponseCache::key_for(&a), ResponseCache::key_for(&b));
    }

    // --- get/insert --------------------------------------------------------------

    #[test]
    fn get_is_none_before_any_insert() {
        let mut cache = ResponseCache::new(&config(60, 10));
        assert!(cache
            .get(ResponseCache::key_for(&request("a/m1", "hi")))
            .is_none());
    }

    #[test]
    fn get_returns_what_was_inserted_under_the_same_key() {
        let mut cache = ResponseCache::new(&config(60, 10));
        let key = ResponseCache::key_for(&request("a/m1", "hi"));
        cache.insert(key, response("resp-1"));

        let hit = cache.get(key).expect("should be cached");
        assert_eq!(hit.id, "resp-1");
    }

    #[test]
    fn get_expires_an_entry_past_its_ttl() {
        // A 0-second TTL means "expired the instant it's inserted" --
        // any nonzero elapsed time (which `Instant::elapsed` always
        // reports, even immediately after insert) exceeds it.
        let mut cache = ResponseCache::new(&config(0, 10));
        let key = ResponseCache::key_for(&request("a/m1", "hi"));
        cache.insert(key, response("resp-1"));

        assert!(cache.get(key).is_none());
    }

    #[test]
    fn insert_evicts_the_oldest_entry_once_over_capacity() {
        let mut cache = ResponseCache::new(&config(60, 2));
        let key_a = ResponseCache::key_for(&request("a/m1", "a"));
        let key_b = ResponseCache::key_for(&request("a/m1", "b"));
        let key_c = ResponseCache::key_for(&request("a/m1", "c"));

        cache.insert(key_a, response("resp-a"));
        cache.insert(key_b, response("resp-b"));
        cache.insert(key_c, response("resp-c"));

        assert!(cache.get(key_a).is_none(), "oldest entry should be evicted");
        assert!(cache.get(key_b).is_some());
        assert!(cache.get(key_c).is_some());
    }

    #[test]
    fn insert_reinserting_an_existing_key_does_not_evict() {
        let mut cache = ResponseCache::new(&config(60, 2));
        let key_a = ResponseCache::key_for(&request("a/m1", "a"));
        let key_b = ResponseCache::key_for(&request("a/m1", "b"));

        cache.insert(key_a, response("resp-a"));
        cache.insert(key_b, response("resp-b"));
        cache.insert(key_a, response("resp-a-2"));

        assert_eq!(cache.get(key_a).unwrap().id, "resp-a-2");
        assert!(cache.get(key_b).is_some());
    }

    // --- SemanticCache -------------------------------------------------------------

    fn semantic_config(threshold: f64) -> CacheConfig {
        CacheConfig {
            ttl_secs: 60,
            max_entries: 10,
            mode: crate::config::CacheMode::Semantic,
            similarity_threshold: threshold,
            embedding_model: Some("openai/text-embedding-3-small".to_string()),
        }
    }

    #[test]
    fn cosine_similarity_is_one_for_identical_vectors() {
        assert!((cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_is_zero_for_orthogonal_vectors() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_is_zero_for_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_similarity_is_zero_for_a_zero_vector() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn semantic_get_is_none_before_any_insert() {
        let mut cache = SemanticCache::new(&semantic_config(0.85));
        assert!(cache.get(1, &[1.0, 0.0]).is_none());
    }

    #[test]
    fn semantic_get_hits_on_a_near_identical_embedding_above_threshold() {
        let mut cache = SemanticCache::new(&semantic_config(0.9));
        cache.insert(1, vec![1.0, 0.0], response("resp-1"));
        // Not byte-identical, but cosine-similarity ~1.0.
        let hit = cache.get(1, &[0.999, 0.001]);
        assert_eq!(hit.expect("should hit").id, "resp-1");
    }

    #[test]
    fn semantic_get_misses_below_threshold() {
        let mut cache = SemanticCache::new(&semantic_config(0.95));
        cache.insert(1, vec![1.0, 0.0], response("resp-1"));
        // Orthogonal -- similarity 0.0, far below any reasonable threshold.
        assert!(cache.get(1, &[0.0, 1.0]).is_none());
    }

    #[test]
    fn semantic_get_never_crosses_scope_keys() {
        // Same embedding, but a different scope (e.g. a different model
        // or sampling params) must never hit -- semantic fuzziness is
        // scoped to message content only.
        let mut cache = SemanticCache::new(&semantic_config(0.5));
        cache.insert(1, vec![1.0, 0.0], response("resp-1"));
        assert!(cache.get(2, &[1.0, 0.0]).is_none());
    }

    #[test]
    fn semantic_get_expires_an_entry_past_its_ttl() {
        let mut config = semantic_config(0.5);
        config.ttl_secs = 0;
        let mut cache = SemanticCache::new(&config);
        cache.insert(1, vec![1.0, 0.0], response("resp-1"));
        assert!(cache.get(1, &[1.0, 0.0]).is_none());
    }

    #[test]
    fn semantic_insert_evicts_the_oldest_entry_once_over_capacity() {
        let mut config = semantic_config(0.5);
        config.max_entries = 2;
        let mut cache = SemanticCache::new(&config);
        cache.insert(1, vec![1.0, 0.0, 0.0], response("resp-a"));
        cache.insert(1, vec![0.0, 1.0, 0.0], response("resp-b"));
        cache.insert(1, vec![0.0, 0.0, 1.0], response("resp-c"));

        // "resp-a"'s embedding is gone entirely -- even an exact-match
        // query for it no longer hits.
        assert!(cache.get(1, &[1.0, 0.0, 0.0]).is_none());
        assert!(cache.get(1, &[0.0, 1.0, 0.0]).is_some());
        assert!(cache.get(1, &[0.0, 0.0, 1.0]).is_some());
    }

    #[test]
    fn semantic_get_picks_the_best_match_among_multiple_candidates() {
        let mut cache = SemanticCache::new(&semantic_config(0.5));
        // Orthogonal to the query -- similarity 0.0.
        cache.insert(1, vec![0.0, 1.0], response("resp-far"));
        // Nearly parallel to the query -- similarity ~0.99.
        cache.insert(1, vec![0.9, 0.1], response("resp-close"));
        let hit = cache.get(1, &[1.0, 0.0]);
        assert_eq!(hit.expect("should hit").id, "resp-close");
    }

    #[test]
    fn scope_key_for_differs_when_a_non_message_field_differs() {
        let mut a = request("anthropic/m1", "hi");
        let mut b = request("anthropic/m1", "hi");
        a.temperature = Some(0.2);
        b.temperature = Some(0.9);
        assert_ne!(
            SemanticCache::scope_key_for(&a),
            SemanticCache::scope_key_for(&b)
        );
    }

    #[test]
    fn scope_key_for_is_identical_regardless_of_message_content() {
        let a = request("anthropic/m1", "hi");
        let b = request("anthropic/m1", "a completely different question");
        assert_eq!(
            SemanticCache::scope_key_for(&a),
            SemanticCache::scope_key_for(&b)
        );
    }
}
