//! Sovereign AI Retrieval-Augmented Generation (RAG) & Question-Answering Engine.
//!
//! Connects search indexing, SIMD vector embedding retrieval, and LLM inference for sovereign Q&A.
//!
//! Split out of `rusty_ansder`, which used to bundle this RAG engine together
//! with an unrelated ASN.1 DER parser under one portmanteau crate name.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{format, string::String, string::ToString, vec::Vec};

/// A document chunk indexed for vector retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Unique document ID.
    pub id: String,
    /// Document title or URI source.
    pub title: String,
    /// Text content chunk.
    pub content: String,
    /// Optional SIMD vector embedding.
    pub embedding: Vec<f32>,
}

/// A search hit with SIMD relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Matched document.
    pub doc: Document,
    /// Vector cosine similarity / SIMD dot product score.
    pub score: f32,
}

/// In-memory vector search index for RAG retrieval.
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    documents: Vec<Document>,
}

impl SearchIndex {
    /// Create an empty search index.
    pub fn new() -> Self {
        SearchIndex {
            documents: Vec::new(),
        }
    }

    /// Add a document to the index.
    pub fn index_doc(&mut self, doc: Document) {
        self.documents.push(doc);
    }

    /// Vector similarity search using `rusty_simd::dot_product`.
    ///
    /// Documents with an empty embedding or an embedding whose dimension
    /// does not match `query_vec` are excluded from the results entirely
    /// (they are not scored as `0.0`, which would let an incompatible
    /// document outrank a valid, merely-dissimilar one with a negative
    /// score).
    pub fn search_vector(&self, query_vec: &[f32], limit: usize) -> Vec<SearchHit> {
        if limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, doc)| !doc.embedding.is_empty() && doc.embedding.len() == query_vec.len())
            .map(|(idx, doc)| (idx, rusty_simd::dot_product(&doc.embedding, query_vec)))
            .collect();

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(idx, score)| SearchHit {
                doc: self.documents[idx].clone(),
                score,
            })
            .collect()
    }

    /// Search indexed documents for query keywords.
    pub fn query(&self, query_str: &str, limit: usize) -> Vec<SearchHit> {
        let keywords: Vec<String> = query_str
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if keywords.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = Vec::new();
        for (idx, doc) in self.documents.iter().enumerate() {
            let content_lower = doc.content.to_lowercase();
            let mut matches = 0usize;
            for kw in &keywords {
                if content_lower.contains(kw) {
                    matches += 1;
                }
            }

            if matches > 0 {
                let score = (matches as f32) / (keywords.len() as f32);
                scored.push((idx, score));
            }
        }

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(idx, score)| SearchHit {
                doc: self.documents[idx].clone(),
                score,
            })
            .collect()
    }
}

/// Sovereign RAG Question Answering Engine.
pub struct RagEngine {
    index: SearchIndex,
}

impl RagEngine {
    /// Create a new RAG Engine wrapping an index.
    pub fn new(index: SearchIndex) -> Self {
        RagEngine { index }
    }

    /// Process a question query using retrieved document context.
    pub fn answer(&self, question: &str) -> String {
        let hits = self.index.query(question, 3);
        if hits.is_empty() {
            return "No relevant context found in sovereign knowledge base.".to_string();
        }

        let mut context_buf = String::new();
        context_buf.push_str("Context:\n");
        for (i, hit) in hits.iter().enumerate() {
            context_buf.push_str(&format!(
                "[{}] {}: {}\n",
                i + 1,
                hit.doc.title,
                hit.doc.content
            ));
        }

        format!("Based on sovereign context:\n{context_buf}\nAnswer: Found {} relevant document(s) addressing '{question}'.", hits.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rag_retrieval_and_answer() {
        let mut index = SearchIndex::new();
        index.index_doc(Document {
            id: "doc1".to_string(),
            title: "Architecture.md".to_string(),
            content: "Rusty Mill uses a 4-layer sovereign Rust platform architecture.".to_string(),
            embedding: vec![1.0, 0.0, 0.0, 1.0],
        });

        let engine = RagEngine::new(index);
        let ans = engine.answer("What is Rusty Mill architecture?");
        assert!(ans.contains("4-layer sovereign Rust platform"));
    }

    #[test]
    fn simd_vector_search() {
        let mut index = SearchIndex::new();
        index.index_doc(Document {
            id: "doc1".to_string(),
            title: "Doc 1".to_string(),
            content: "Content 1".to_string(),
            embedding: vec![1.0, 2.0, 3.0, 4.0],
        });

        let query = vec![1.0, 2.0, 3.0, 4.0];
        let hits = index.search_vector(&query, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].score, 30.0);
    }

    #[test]
    fn search_vector_limit_zero_returns_empty() {
        let mut index = SearchIndex::new();
        index.index_doc(Document {
            id: "doc1".to_string(),
            title: "Doc 1".to_string(),
            content: "Content 1".to_string(),
            embedding: vec![1.0, 2.0, 3.0, 4.0],
        });

        let query = vec![1.0, 2.0, 3.0, 4.0];
        let hits = index.search_vector(&query, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn query_limit_zero_returns_empty() {
        let mut index = SearchIndex::new();
        index.index_doc(Document {
            id: "doc1".to_string(),
            title: "Doc 1".to_string(),
            content: "sovereign platform".to_string(),
            embedding: vec![],
        });

        let hits = index.query("sovereign", 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_vector_excludes_incompatible_embeddings() {
        let mut index = SearchIndex::new();
        // Empty embedding: incompatible, must never be scored/returned.
        index.index_doc(Document {
            id: "empty".to_string(),
            title: "Empty".to_string(),
            content: "Empty embedding".to_string(),
            embedding: vec![],
        });
        // Dimension mismatch: incompatible, must never be scored/returned.
        index.index_doc(Document {
            id: "mismatch".to_string(),
            title: "Mismatch".to_string(),
            content: "Mismatched dimension".to_string(),
            embedding: vec![1.0, 2.0],
        });
        // Valid, dimension-matched, but genuinely dissimilar (negative score).
        index.index_doc(Document {
            id: "negative".to_string(),
            title: "Negative".to_string(),
            content: "Negative score".to_string(),
            embedding: vec![-1.0],
        });
        // Valid, dimension-matched, positively similar.
        index.index_doc(Document {
            id: "positive".to_string(),
            title: "Positive".to_string(),
            content: "Positive score".to_string(),
            embedding: vec![1.0],
        });

        let query = vec![1.0];
        let hits = index.search_vector(&query, 10);

        // Incompatible documents must be excluded entirely, not ranked with a
        // fake 0.0 score that would outrank the valid negative-scoring doc.
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.doc.id != "empty" && h.doc.id != "mismatch"));

        // The valid positive-scoring doc must rank above the valid negative one.
        assert_eq!(hits[0].doc.id, "positive");
        assert_eq!(hits[0].score, 1.0);
        assert_eq!(hits[1].doc.id, "negative");
        assert_eq!(hits[1].score, -1.0);
    }
}
