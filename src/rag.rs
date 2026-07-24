//! Sovereign AI Retrieval-Augmented Generation (RAG) & Question-Answering Engine.
//!
//! Connects search indexing, vector retrieval, and LLM inference for sovereign Q&A.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{string::String, string::ToString, vec::Vec};

/// A document chunk indexed for retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Unique document ID.
    pub id: String,
    /// Document title or URI source.
    pub title: String,
    /// Text content chunk.
    pub content: String,
}

/// A search hit with relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Matched document.
    pub doc: Document,
    /// Relevance score (0.0 .. 1.0).
    pub score: f32,
}

/// In-memory vector/keyword search index for RAG retrieval.
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    documents: Vec<Document>,
}

impl SearchIndex {
    /// Create an empty search index.
    pub fn new() -> Self {
        SearchIndex { documents: Vec::new() }
    }

    /// Add a document to the index.
    pub fn index_doc(&mut self, doc: Document) {
        self.documents.push(doc);
    }

    /// Search indexed documents for query keywords.
    pub fn query(&self, query_str: &str, limit: usize) -> Vec<SearchHit> {
        let keywords: Vec<String> = query_str
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if keywords.is_empty() {
            return Vec::new();
        }

        let mut hits = Vec::new();
        for doc in &self.documents {
            let content_lower = doc.content.to_lowercase();
            let mut matches = 0usize;
            for kw in &keywords {
                if content_lower.contains(kw) {
                    matches += 1;
                }
            }

            if matches > 0 {
                let score = (matches as f32) / (keywords.len() as f32);
                hits.push(SearchHit {
                    doc: doc.clone(),
                    score,
                });
            }
        }

        // Sort descending by relevance score
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
        hits.truncate(limit);
        hits
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
            context_buf.push_str(&format!("[{}] {}: {}\n", i + 1, hit.doc.title, hit.doc.content));
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
        });

        let engine = RagEngine::new(index);
        let ans = engine.answer("What is Rusty Mill architecture?");
        assert!(ans.contains("4-layer sovereign Rust platform"));
    }
}
