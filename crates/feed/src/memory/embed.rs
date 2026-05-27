//! The embedding seam (PRD 03 §Seams; Phase 5). Semantic recall stores a vector
//! per memory (`embedding` BLOB) and ranks by cosine. The aisdk embed call is
//! injected by the caller (the real model lives in `app`), so `feed` stays
//! model-agnostic and the cosine path is offline-testable. Absent an
//! [`Embedder`], recall falls back to FTS5 lexical (mixed corpora are fine —
//! a memory with no vector is reachable only via the lexical fallback).

use async_trait::async_trait;

use crate::error::ToolError;

/// Produces an embedding vector for a piece of text. Object-safe seam
/// (ADR-0024), stored as `Arc<dyn Embedder>`.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed `text` into a dense vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ToolError>;
}

/// Cosine similarity in `[-1, 1]`; `0.0` if either vector is zero/empty or the
/// dimensions differ.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Pack a vector as little-endian `f32` bytes for the `embedding` BLOB.
pub(crate) fn pack(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Unpack a little-endian `f32` BLOB back into a vector.
pub(crate) fn unpack(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(any(test, feature = "fake-embed"))]
/// A deterministic bag-of-words embedder for offline tests: each ASCII-lowercased
/// token contributes to a fixed-size hashed vector (L2-normalized). Paraphrases
/// that share words land near each other in cosine space.
pub struct HashEmbedder {
    dims: usize,
}

#[cfg(any(test, feature = "fake-embed"))]
impl HashEmbedder {
    /// A `dims`-dimensional hashing embedder.
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }
}

#[cfg(any(test, feature = "fake-embed"))]
#[async_trait]
impl Embedder for HashEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ToolError> {
        let mut v = vec![0.0f32; self.dims];
        for tok in text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let mut h: u64 = 1469598103934665603;
            for b in tok.to_ascii_lowercase().bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            let idx = (h as usize) % self.dims;
            v[idx] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn paraphrases_are_closer_than_unrelated() {
        let e = HashEmbedder::new(64);
        let a = e.embed("the build uses cargo to compile").await.unwrap();
        let b = e
            .embed("compile the project with cargo build")
            .await
            .unwrap();
        let c = e.embed("the cat sat on a warm mat").await.unwrap();
        assert!(cosine(&a, &b) > cosine(&a, &c));
    }

    #[test]
    fn pack_round_trips() {
        let v = vec![0.5f32, -1.0, 2.25];
        assert_eq!(unpack(&pack(&v)), v);
    }
}
