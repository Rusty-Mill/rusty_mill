> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Group C: RAG Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust-native RAG pipeline with LanceDB as the embedded vector store. Notes are chunked by heading, embedded via the existing AiProvider, stored in LanceDB, and queried with semantic search + LLM generation.

**Architecture:** New `rustsidian-rag` crate with chunker, LanceDB vector store wrapper, and RAG pipeline. Depends on `rustsidian-core` for note types and `AiProvider` for embeddings/chat. Desktop crate adds 4 new Tauri commands. Frontend AI panel (already in NanOEd UI) is rewired from Python stubs to Rust IPC.

**Tech Stack:** LanceDB (embedded vector DB), Arrow (data format), rustsidian-core AiProvider (OpenAI/Ollama embeddings + chat), Tauri IPC

**Spec:** `docs/superpowers/specs/2026-04-05-nanoed-feature-integration-design.md` — Sections C1–C8

**Prerequisite:** Group A must be completed (NanOEd frontend in place). Group B is independent.

---

## File Map

### Files to create
- `crates/rustsidian-rag/Cargo.toml`
- `crates/rustsidian-rag/src/lib.rs`
- `crates/rustsidian-rag/src/chunker.rs`
- `crates/rustsidian-rag/src/store.rs`
- `crates/rustsidian-rag/src/pipeline.rs`
- `crates/rustsidian-rag/src/types.rs`
- `crates/rustsidian-rag/src/error.rs`
- `crates/rustsidian-desktop/src/commands/rag.rs`

### Files to modify
- `Cargo.toml` (workspace) — add `rustsidian-rag` member
- `crates/rustsidian-desktop/Cargo.toml` — add `rustsidian-rag` dep
- `crates/rustsidian-desktop/src/commands/mod.rs` — add `rag` module
- `crates/rustsidian-desktop/src/lib.rs` — register rag commands
- `crates/rustsidian-desktop/src/state.rs` — add RagEngine to AppState
- `crates/rustsidian-mcp/src/server.rs` — add rag tools (optional, can defer)
- `frontend/src/lib/ai.ts` — replace stubs with real IPC calls

---

### Task 1: Create rustsidian-rag Crate Scaffold

**Files:**
- Create: `crates/rustsidian-rag/Cargo.toml`
- Create: `crates/rustsidian-rag/src/lib.rs`
- Create: `crates/rustsidian-rag/src/types.rs`
- Create: `crates/rustsidian-rag/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create crate directory**

```bash
mkdir -p crates/rustsidian-rag/src
```

- [ ] **Step 2: Create Cargo.toml**

Create `crates/rustsidian-rag/Cargo.toml`:

```toml
[package]
name = "rustsidian-rag"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
rustsidian-core = { path = "../rustsidian-core" }
lancedb = "0.16"
arrow-array = "54"
arrow-schema = "54"
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
reqwest = { version = "0.12", features = ["json"] }

[lints]
workspace = true
```

Note: LanceDB and Arrow version numbers should be verified against crates.io at implementation time. Use the latest compatible versions.

- [ ] **Step 3: Create error.rs**

Create `crates/rustsidian-rag/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RagError {
    #[error("LanceDB error: {0}")]
    Lance(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("AI provider error: {0}")]
    Ai(String),

    #[error("Core error: {0}")]
    Core(#[from] rustsidian_core::error::VaultError),

    #[error("{0}")]
    Other(String),
}
```

- [ ] **Step 4: Create types.rs**

Create `crates/rustsidian-rag/src/types.rs`:

```rust
use serde::{Deserialize, Serialize};

/// A chunk of a note, split by heading boundaries.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub note_path: String,
    pub heading: String,
    pub chunk_index: u32,
    pub text: String,
    pub char_range: (usize, usize),
}

/// A match returned from vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMatch {
    pub note_path: String,
    pub heading: String,
    pub chunk_index: u32,
    pub text: String,
    pub score: f32,
}

/// Result from a RAG query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagResult {
    pub answer: String,
    pub sources: Vec<RagSource>,
    pub model: String,
}

/// A source citation in a RAG result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSource {
    pub note_path: String,
    pub heading: String,
    pub score: f32,
    pub chunk_text: String,
}

/// Result from indexing the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexResult {
    pub notes_indexed: u32,
    pub chunks_created: u32,
}

/// Statistics about the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub total_chunks: u64,
    pub total_notes: u64,
}
```

- [ ] **Step 5: Create lib.rs**

Create `crates/rustsidian-rag/src/lib.rs`:

```rust
pub mod chunker;
pub mod error;
pub mod pipeline;
pub mod store;
pub mod types;

pub use error::RagError;
pub use pipeline::RagEngine;
pub use types::{ChunkMatch, IndexResult, RagResult, RagSource, StoreStats};
```

- [ ] **Step 6: Add crate to workspace**

In the root `Cargo.toml`, add `"crates/rustsidian-rag"` to the workspace members:

```toml
[workspace]
members = [
    "crates/rustsidian-core",
    "crates/rustsidian-cli",
    "crates/rustsidian-mcp",
    "crates/rustsidian-desktop",
    "crates/rustsidian-rag",
]
```

- [ ] **Step 7: Create stub modules for chunker, store, pipeline**

Create `crates/rustsidian-rag/src/chunker.rs`:

```rust
use crate::types::Chunk;

/// Split a note into semantic chunks by heading boundaries.
pub fn chunk_note(_path: &str, _content: &str) -> Vec<Chunk> {
    todo!("Implement in Task 2")
}
```

Create `crates/rustsidian-rag/src/store.rs`:

```rust
// LanceDB vector store — implemented in Task 3
```

Create `crates/rustsidian-rag/src/pipeline.rs`:

```rust
// RAG pipeline — implemented in Task 4
```

- [ ] **Step 8: Verify it compiles**

```bash
cargo check -p rustsidian-rag
```

Expected: compiles (stubs are fine for now)

- [ ] **Step 9: Commit**

```bash
git add crates/rustsidian-rag/ Cargo.toml
git commit -m "feat: scaffold rustsidian-rag crate with types and error handling"
```

---

### Task 2: Implement Chunker

**Files:**
- Modify: `crates/rustsidian-rag/src/chunker.rs`

- [ ] **Step 1: Write test for chunker**

Append to `crates/rustsidian-rag/src/chunker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_by_headings() {
        let content = "---\ntitle: Test\n---\n\nIntro paragraph.\n\n# Section One\n\nContent one.\n\n## Section Two\n\nContent two.";
        let chunks = chunk_note("test.md", content);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading, "Introduction");
        assert!(chunks[0].text.contains("Intro paragraph"));
        assert_eq!(chunks[1].heading, "Section One");
        assert!(chunks[1].text.contains("Content one"));
        assert_eq!(chunks[2].heading, "Section Two");
        assert!(chunks[2].text.contains("Content two"));
    }

    #[test]
    fn chunks_without_headings_by_paragraph() {
        let content = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph here.";
        let chunks = chunk_note("test.md", content);

        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].heading, "Introduction");
    }

    #[test]
    fn strips_frontmatter() {
        let content = "---\ntitle: Test\ntags: [a, b]\n---\n\n# Main\n\nBody text.";
        let chunks = chunk_note("test.md", content);

        for chunk in &chunks {
            assert!(!chunk.text.contains("---"));
            assert!(!chunk.text.contains("title: Test"));
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd crates/rustsidian-rag && cargo test chunker -- --nocapture
```

Expected: FAIL — `chunk_note` is a `todo!()`.

- [ ] **Step 3: Implement chunk_note**

Replace the contents of `crates/rustsidian-rag/src/chunker.rs`:

```rust
use crate::types::Chunk;

/// Split a note into semantic chunks by heading boundaries (H1–H6).
///
/// - Strips YAML frontmatter before chunking
/// - Content before first heading becomes "Introduction" chunk
/// - If no headings, returns a single "Introduction" chunk
/// - Each chunk carries note_path, heading, index, and char range
pub fn chunk_note(path: &str, content: &str) -> Vec<Chunk> {
    let body = strip_frontmatter(content);
    let mut chunks = Vec::new();
    let mut current_heading = "Introduction".to_string();
    let mut current_text = String::new();
    let mut current_start: usize = 0;
    let mut chunk_index: u32 = 0;

    for line in body.lines() {
        if let Some(heading) = extract_heading(line) {
            // Flush current chunk if it has content
            let trimmed = current_text.trim();
            if !trimmed.is_empty() {
                chunks.push(Chunk {
                    note_path: path.to_string(),
                    heading: current_heading.clone(),
                    chunk_index,
                    text: trimmed.to_string(),
                    char_range: (current_start, current_start + trimmed.len()),
                });
                chunk_index += 1;
            }
            current_heading = heading;
            current_text = String::new();
            current_start = current_start + current_text.len();
        } else {
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(line);
        }
    }

    // Flush final chunk
    let trimmed = current_text.trim();
    if !trimmed.is_empty() {
        chunks.push(Chunk {
            note_path: path.to_string(),
            heading: current_heading,
            chunk_index,
            text: trimmed.to_string(),
            char_range: (current_start, current_start + trimmed.len()),
        });
    }

    chunks
}

/// Strip YAML frontmatter (--- delimited) from content.
fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---") {
        return content;
    }
    // Find the closing ---
    if let Some(end) = content[3..].find("\n---") {
        let after = end + 3 + 4; // skip past \n---
        if after < content.len() {
            return content[after..].trim_start_matches('\n');
        }
    }
    content
}

/// Extract heading text from a markdown heading line (# through ######).
fn extract_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if hashes >= 1 && hashes <= 6 {
            let rest = trimmed[hashes..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_by_headings() {
        let content = "---\ntitle: Test\n---\n\nIntro paragraph.\n\n# Section One\n\nContent one.\n\n## Section Two\n\nContent two.";
        let chunks = chunk_note("test.md", content);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading, "Introduction");
        assert!(chunks[0].text.contains("Intro paragraph"));
        assert_eq!(chunks[1].heading, "Section One");
        assert!(chunks[1].text.contains("Content one"));
        assert_eq!(chunks[2].heading, "Section Two");
        assert!(chunks[2].text.contains("Content two"));
    }

    #[test]
    fn chunks_without_headings_by_paragraph() {
        let content = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph here.";
        let chunks = chunk_note("test.md", content);

        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].heading, "Introduction");
    }

    #[test]
    fn strips_frontmatter() {
        let content = "---\ntitle: Test\ntags: [a, b]\n---\n\n# Main\n\nBody text.";
        let chunks = chunk_note("test.md", content);

        for chunk in &chunks {
            assert!(!chunk.text.contains("---"));
            assert!(!chunk.text.contains("title: Test"));
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd crates/rustsidian-rag && cargo test chunker -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/rustsidian-rag/src/chunker.rs
git commit -m "feat: implement heading-based note chunker for RAG pipeline"
```

---

### Task 3: Implement LanceDB Vector Store

**Files:**
- Modify: `crates/rustsidian-rag/src/store.rs`

- [ ] **Step 1: Implement the vector store**

Replace `crates/rustsidian-rag/src/store.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    Float32Array, RecordBatch, RecordBatchIterator, StringArray, UInt32Array,
    FixedSizeListArray, ArrayRef,
};
use arrow_schema::{DataType, Field, Schema};
use lancedb::connect;
use lancedb::query::{ExecutableQuery, QueryBase};

use crate::error::RagError;
use crate::types::{ChunkMatch, Chunk, StoreStats};

/// LanceDB-backed vector store for RAG chunks.
pub struct VectorStore {
    db: lancedb::Connection,
    table_name: String,
    embedding_dim: usize,
}

impl VectorStore {
    /// Open or create a LanceDB store at `<vault_root>/.vault/lance/`.
    pub async fn open(vault_root: &Path, embedding_dim: usize) -> Result<Self, RagError> {
        let db_path = vault_root.join(".vault").join("lance");
        std::fs::create_dir_all(&db_path).map_err(|e| RagError::Other(e.to_string()))?;

        let db = connect(db_path.to_str().unwrap())
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        let store = Self {
            db,
            table_name: "chunks".to_string(),
            embedding_dim,
        };

        // Ensure table exists
        store.ensure_table().await?;

        Ok(store)
    }

    fn schema(&self) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("note_path", DataType::Utf8, false),
            Field::new("heading", DataType::Utf8, false),
            Field::new("chunk_index", DataType::UInt32, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.embedding_dim as i32,
                ),
                false,
            ),
        ]))
    }

    async fn ensure_table(&self) -> Result<(), RagError> {
        let tables = self.db.table_names()
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        if !tables.contains(&self.table_name) {
            // Create empty table with schema
            let schema = self.schema();
            let batch = RecordBatch::new_empty(schema.clone());
            let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
            self.db
                .create_table(&self.table_name, Box::new(batches))
                .execute()
                .await
                .map_err(|e| RagError::Lance(e.to_string()))?;
        }

        Ok(())
    }

    /// Index chunks for a note. Deletes existing chunks for this path first.
    pub async fn index_note(
        &self,
        path: &str,
        chunks: &[Chunk],
        embeddings: &[Vec<f32>],
    ) -> Result<(), RagError> {
        assert_eq!(chunks.len(), embeddings.len());

        // Delete existing chunks for this note
        self.remove_note(path).await?;

        if chunks.is_empty() {
            return Ok(());
        }

        let schema = self.schema();
        let n = chunks.len();

        let ids: Vec<String> = chunks
            .iter()
            .map(|c| format!("{}:{}", c.note_path, c.chunk_index))
            .collect();
        let note_paths: Vec<&str> = chunks.iter().map(|c| c.note_path.as_str()).collect();
        let headings: Vec<&str> = chunks.iter().map(|c| c.heading.as_str()).collect();
        let indices: Vec<u32> = chunks.iter().map(|c| c.chunk_index).collect();
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();

        // Build FixedSizeList array for vectors
        let flat_values: Vec<f32> = embeddings.iter().flat_map(|v| v.iter().copied()).collect();
        let values_array = Float32Array::from(flat_values);
        let list_array = FixedSizeListArray::try_new_from_values(
            values_array,
            self.embedding_dim as i32,
        )
        .map_err(|e| RagError::Lance(e.to_string()))?;

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)) as ArrayRef,
                Arc::new(StringArray::from(note_paths)) as ArrayRef,
                Arc::new(StringArray::from(headings)) as ArrayRef,
                Arc::new(UInt32Array::from(indices)) as ArrayRef,
                Arc::new(StringArray::from(texts)) as ArrayRef,
                Arc::new(list_array) as ArrayRef,
            ],
        )
        .map_err(|e| RagError::Lance(e.to_string()))?;

        let table = self.db
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table
            .add(Box::new(batches))
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        Ok(())
    }

    /// Remove all chunks for a given note path.
    pub async fn remove_note(&self, path: &str) -> Result<(), RagError> {
        let table = self.db
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        table
            .delete(&format!("note_path = '{}'", path.replace('\'', "''")))
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        Ok(())
    }

    /// Search for the most similar chunks to a query vector.
    pub async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ChunkMatch>, RagError> {
        let table = self.db
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        let results = table
            .vector_search(query_embedding)
            .map_err(|e| RagError::Lance(e.to_string()))?
            .limit(limit)
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        use futures::TryStreamExt;
        let batches: Vec<RecordBatch> = results
            .try_collect()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        let mut matches = Vec::new();
        for batch in &batches {
            let note_paths = batch.column_by_name("note_path")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let headings = batch.column_by_name("heading")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let indices = batch.column_by_name("chunk_index")
                .and_then(|c| c.as_any().downcast_ref::<UInt32Array>());
            let texts = batch.column_by_name("text")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let distances = batch.column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            if let (Some(np), Some(h), Some(ci), Some(t), Some(d)) =
                (note_paths, headings, indices, texts, distances)
            {
                for i in 0..batch.num_rows() {
                    matches.push(ChunkMatch {
                        note_path: np.value(i).to_string(),
                        heading: h.value(i).to_string(),
                        chunk_index: ci.value(i),
                        text: t.value(i).to_string(),
                        score: 1.0 - d.value(i), // Convert distance to similarity
                    });
                }
            }
        }

        Ok(matches)
    }

    /// Get store statistics.
    pub async fn stats(&self) -> Result<StoreStats, RagError> {
        let table = self.db
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| RagError::Lance(e.to_string()))?;

        let total_chunks = table.count_rows(None)
            .await
            .map_err(|e| RagError::Lance(e.to_string()))? as u64;

        // Approximate unique notes from chunks (exact count would require a query)
        let total_notes = total_chunks / 3; // rough estimate — average 3 chunks per note

        Ok(StoreStats {
            total_chunks,
            total_notes,
        })
    }
}
```

- [ ] **Step 2: Add futures dependency**

In `crates/rustsidian-rag/Cargo.toml`, add:

```toml
futures = "0.3"
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p rustsidian-rag
```

Expected: compiles. LanceDB API may have differences from what's shown — adjust types/method names based on compiler errors. The key patterns (connect, create_table, vector_search) are stable.

- [ ] **Step 4: Commit**

```bash
git add crates/rustsidian-rag/src/store.rs crates/rustsidian-rag/Cargo.toml
git commit -m "feat: implement LanceDB vector store for RAG chunks"
```

---

### Task 4: Implement RAG Pipeline

**Files:**
- Modify: `crates/rustsidian-rag/src/pipeline.rs`

- [ ] **Step 1: Implement the RAG engine**

Replace `crates/rustsidian-rag/src/pipeline.rs`:

```rust
use std::sync::{Arc, Mutex};

use rustsidian_core::ai::{AiConfig, AiMessage, AiProvider, AiRole};
use rustsidian_core::Vault;

use crate::chunker::chunk_note;
use crate::error::RagError;
use crate::store::VectorStore;
use crate::types::{IndexResult, RagResult, RagSource, StoreStats};

const RAG_SYSTEM_PROMPT: &str = "\
You are a knowledge assistant for a personal vault. Answer the question using ONLY \
the provided context. Cite sources using [[Note Name]] wikilink syntax. If the \
context doesn't contain enough information, say so.";

/// RAG engine: chunking, embedding, vector search, and LLM generation.
pub struct RagEngine {
    store: VectorStore,
    config: AiConfig,
}

impl RagEngine {
    /// Create a new RAG engine with an open vector store.
    pub async fn new(vault_root: &std::path::Path, config: AiConfig) -> Result<Self, RagError> {
        // Default embedding dimension — OpenAI text-embedding-3-small = 1536, Ollama varies
        let embedding_dim = 1536;
        let store = VectorStore::open(vault_root, embedding_dim).await?;
        Ok(Self { store, config })
    }

    /// Index all notes in the vault.
    pub async fn index_vault(&self, vault: &Vault) -> Result<IndexResult, RagError> {
        let paths = vault.list_all_note_paths().map_err(RagError::Core)?;
        let mut notes_indexed = 0u32;
        let mut chunks_created = 0u32;

        let provider = AiProvider::new(self.config.clone());

        for path in &paths {
            let note = vault.get_note_by_path(path).map_err(RagError::Core)?;
            if let Some(note) = note {
                let chunks = chunk_note(path, &note.content);
                if chunks.is_empty() {
                    continue;
                }

                // Embed each chunk
                let mut embeddings = Vec::new();
                for chunk in &chunks {
                    let embedding = self.embed_text(&provider, &chunk.text).await?;
                    embeddings.push(embedding);
                }

                self.store.index_note(path, &chunks, &embeddings).await?;
                notes_indexed += 1;
                chunks_created += chunks.len() as u32;

                tracing::debug!("Indexed {path}: {} chunks", chunks.len());
            }
        }

        Ok(IndexResult {
            notes_indexed,
            chunks_created,
        })
    }

    /// Re-index a single note (after edit).
    pub async fn index_note(&self, path: &str, content: &str) -> Result<(), RagError> {
        let provider = AiProvider::new(self.config.clone());
        let chunks = chunk_note(path, content);

        if chunks.is_empty() {
            self.store.remove_note(path).await?;
            return Ok(());
        }

        let mut embeddings = Vec::new();
        for chunk in &chunks {
            let embedding = self.embed_text(&provider, &chunk.text).await?;
            embeddings.push(embedding);
        }

        self.store.index_note(path, &chunks, &embeddings).await?;
        Ok(())
    }

    /// Query the vault using RAG.
    pub async fn query(&self, question: &str, n_context: usize) -> Result<RagResult, RagError> {
        let provider = AiProvider::new(self.config.clone());

        // 1. Embed the question
        let query_embedding = self.embed_text(&provider, question).await?;

        // 2. Vector search for relevant chunks
        let matches = self.store.search(&query_embedding, n_context).await?;

        if matches.is_empty() {
            return Ok(RagResult {
                answer: "No relevant content found in the vault.".to_string(),
                sources: Vec::new(),
                model: self.config.model.clone(),
            });
        }

        // 3. Assemble context
        let context: String = matches
            .iter()
            .enumerate()
            .map(|(i, m)| {
                format!(
                    "[Source {}: [[{}]] > {}]\n{}\n",
                    i + 1,
                    m.note_path.replace(".md", "").replace(".mdx", ""),
                    m.heading,
                    m.text
                )
            })
            .collect();

        // 4. Generate answer
        let messages = vec![
            AiMessage {
                role: AiRole::System,
                content: RAG_SYSTEM_PROMPT.to_string(),
            },
            AiMessage {
                role: AiRole::User,
                content: format!("Context:\n{context}\n\nQuestion: {question}"),
            },
        ];

        let response = provider
            .chat(messages)
            .await
            .map_err(|e| RagError::Ai(e.to_string()))?;

        // 5. Build result with sources
        let sources = matches
            .iter()
            .map(|m| RagSource {
                note_path: m.note_path.clone(),
                heading: m.heading.clone(),
                score: m.score,
                chunk_text: m.text.clone(),
            })
            .collect();

        Ok(RagResult {
            answer: response.content,
            sources,
            model: self.config.model.clone(),
        })
    }

    /// Get vector store statistics.
    pub async fn stats(&self) -> Result<StoreStats, RagError> {
        self.store.stats().await
    }

    /// Embed a text using the configured AI provider's embedding endpoint.
    async fn embed_text(&self, provider: &AiProvider, text: &str) -> Result<Vec<f32>, RagError> {
        let index = rustsidian_core::ai::EmbeddingIndex::open(std::path::Path::new("."))
            .map_err(|e| RagError::Embedding(e.to_string()))?;
        index
            .embed_text(text, &self.config)
            .await
            .map_err(|e| RagError::Embedding(e.to_string()))
    }
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p rustsidian-rag
```

Expected: compiles. The `EmbeddingIndex::embed_text` method exists in core — it calls OpenAI or Ollama embedding endpoints.

- [ ] **Step 3: Commit**

```bash
git add crates/rustsidian-rag/src/pipeline.rs
git commit -m "feat: implement RAG pipeline with embed→search→generate flow"
```

---

### Task 5: Add RAG Tauri Commands

**Files:**
- Create: `crates/rustsidian-desktop/src/commands/rag.rs`
- Modify: `crates/rustsidian-desktop/src/commands/mod.rs`
- Modify: `crates/rustsidian-desktop/src/lib.rs`
- Modify: `crates/rustsidian-desktop/src/state.rs`
- Modify: `crates/rustsidian-desktop/Cargo.toml`

- [ ] **Step 1: Add rustsidian-rag dependency to desktop crate**

In `crates/rustsidian-desktop/Cargo.toml`, add to `[dependencies]`:

```toml
rustsidian-rag = { path = "../rustsidian-rag" }
```

- [ ] **Step 2: Add RagEngine to AppState**

In `crates/rustsidian-desktop/src/state.rs`, add the import and field:

```rust
use rustsidian_core::search::SearchEngine;
use rustsidian_core::Vault;
use rustsidian_rag::RagEngine;
use std::sync::Mutex;

use crate::commands::terminal::TerminalSession;

pub struct AppState {
    pub vault: Mutex<Option<Vault>>,
    pub config_path: std::path::PathBuf,
    pub watcher: Mutex<Option<rustsidian_core::vault::watcher::VaultWatcher>>,
    pub search_engine: Mutex<Option<SearchEngine>>,
    pub terminal: Mutex<Option<TerminalSession>>,
    pub pty_master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    pub ai_config: Mutex<Option<rustsidian_core::AiConfig>>,
    pub plugin_host: Mutex<Option<rustsidian_core::PluginHost>>,
    pub rag_engine: tokio::sync::Mutex<Option<RagEngine>>,
}
```

Note: `RagEngine` uses async methods, so it needs `tokio::sync::Mutex` instead of `std::sync::Mutex`.

- [ ] **Step 3: Update AppState initialization in lib.rs**

In `crates/rustsidian-desktop/src/lib.rs`, add `rag_engine` to the `app.manage(AppState { ... })` block:

```rust
            app.manage(AppState {
                vault: std::sync::Mutex::new(initial_vault),
                config_path,
                watcher: std::sync::Mutex::new(None),
                search_engine: std::sync::Mutex::new(initial_search),
                terminal: std::sync::Mutex::new(None),
                pty_master: std::sync::Mutex::new(None),
                ai_config: std::sync::Mutex::new(initial_ai),
                plugin_host: std::sync::Mutex::new(None),
                rag_engine: tokio::sync::Mutex::new(None),
            });
```

- [ ] **Step 4: Create rag commands module**

Create `crates/rustsidian-desktop/src/commands/rag.rs`:

```rust
use tauri::State;

use crate::state::AppState;
use rustsidian_rag::{IndexResult, RagEngine, RagResult, StoreStats};

/// Index all notes in the vault for RAG queries.
#[tauri::command]
#[specta::specta]
pub async fn rag_index_vault(state: State<'_, AppState>) -> Result<IndexResult, String> {
    let (vault_root, ai_config) = {
        let guard = state.vault.lock().map_err(|e| e.to_string())?;
        let vault = guard.as_ref().ok_or("No vault open")?;
        let root = vault.root().to_path_buf();
        let ai_guard = state.ai_config.lock().map_err(|e| e.to_string())?;
        let config = ai_guard.as_ref().ok_or("AI not configured")?.clone();
        (root, config)
    };

    // Initialize or reuse RAG engine
    let mut rag_guard = state.rag_engine.lock().await;
    if rag_guard.is_none() {
        let engine = RagEngine::new(&vault_root, ai_config)
            .await
            .map_err(|e| e.to_string())?;
        *rag_guard = Some(engine);
    }

    let engine = rag_guard.as_ref().unwrap();

    // Index vault — need to access vault while holding rag lock
    let vault_guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = vault_guard.as_ref().ok_or("No vault open")?;
    engine.index_vault(vault).await.map_err(|e| e.to_string())
}

/// Re-index a single note after editing.
#[tauri::command]
#[specta::specta]
pub async fn rag_index_note(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let content = {
        let guard = state.vault.lock().map_err(|e| e.to_string())?;
        let vault = guard.as_ref().ok_or("No vault open")?;
        let note = vault.get_note_by_path(&path).map_err(|e| e.to_string())?;
        note.map(|n| n.content).unwrap_or_default()
    };

    let rag_guard = state.rag_engine.lock().await;
    let engine = rag_guard.as_ref().ok_or("RAG engine not initialized — run index first")?;
    engine.index_note(&path, &content).await.map_err(|e| e.to_string())
}

/// Query the vault using RAG (semantic search + AI generation).
#[tauri::command]
#[specta::specta]
pub async fn rag_query(
    question: String,
    n_context: u32,
    state: State<'_, AppState>,
) -> Result<RagResult, String> {
    let rag_guard = state.rag_engine.lock().await;
    let engine = rag_guard.as_ref().ok_or("RAG engine not initialized — run index first")?;
    engine
        .query(&question, n_context as usize)
        .await
        .map_err(|e| e.to_string())
}

/// Get RAG index statistics.
#[tauri::command]
#[specta::specta]
pub async fn rag_stats(state: State<'_, AppState>) -> Result<StoreStats, String> {
    let rag_guard = state.rag_engine.lock().await;
    let engine = rag_guard.as_ref().ok_or("RAG engine not initialized — run index first")?;
    engine.stats().await.map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Register rag module and commands**

In `crates/rustsidian-desktop/src/commands/mod.rs`, add:

```rust
pub mod rag;
```

In `crates/rustsidian-desktop/src/lib.rs`, add to `collect_commands![]`:

```rust
        commands::rag::rag_index_vault,
        commands::rag::rag_index_note,
        commands::rag::rag_query,
        commands::rag::rag_stats,
```

- [ ] **Step 6: Verify it compiles**

```bash
cargo check -p rustsidian-desktop
```

Expected: compiles. Specta will need `Serialize + Type` derives on `RagResult`, `IndexResult`, `StoreStats` — these are already derived in `types.rs`. If specta can't find them, add `specta::Type` derive.

- [ ] **Step 7: Commit**

```bash
git add crates/rustsidian-desktop/ crates/rustsidian-rag/
git commit -m "feat: add RAG Tauri commands and wire into AppState"
```

---

### Task 6: Rewire Frontend AI Panel

**Files:**
- Modify: `frontend/src/lib/ai.ts`

- [ ] **Step 1: Replace AI stubs with real IPC calls**

Update `frontend/src/lib/ai.ts` — replace the stubbed RAG functions with real Tauri invocations:

```typescript
// Replace the aiAsk stub:
export async function aiAsk(question: string, nContext: number = 5): Promise<RagResult> {
  const result = await invoke<RagResult>("rag_query", { question, nContext });
  return result ?? { answer: "RAG not configured.", sources: [], model: "" };
}

// Replace the aiIndex stub:
export async function aiIndex(): Promise<RagIndexResult> {
  const result = await invoke<RagIndexResult>("rag_index_vault");
  return result ?? { notes_indexed: 0, chunks_created: 0 };
}

// Replace the aiStats stub:
export async function aiStats(): Promise<RagStats> {
  const result = await invoke<RagStats>("rag_stats");
  return result ?? { total_chunks: 0, total_notes: 0 };
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
cd frontend && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/ai.ts
git commit -m "feat: wire AI panel to Rust RAG commands"
```

---

### Task 7: End-to-End RAG Test

**Files:**
- No new files — validation

- [ ] **Step 1: Build and launch**

```bash
cargo tauri dev
```

- [ ] **Step 2: Configure AI provider**

Open settings and configure an AI provider (Ollama recommended for local testing):
- Provider: Ollama
- Base URL: http://localhost:11434
- Model: llama3.2 (or any model with embedding support)

- [ ] **Step 3: Index the vault**

Open the AI panel (Ctrl+Shift+A). Click the index button. Verify:
- Progress indicator appears
- Notes are indexed (check terminal for tracing output)
- Stats show chunk/note counts

- [ ] **Step 4: Test a RAG query**

Type a question in the AI panel that relates to content in the vault. Verify:
- Answer appears with relevant information
- Source links are clickable [[wikilinks]]
- Sources show note paths, headings, and relevance scores

- [ ] **Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix: Group C smoke test fixes"
```

---

## Summary

| Task | Description | New/Modified Files |
|------|-------------|-------------------|
| 1 | Scaffold rustsidian-rag crate | New crate directory + workspace |
| 2 | Implement chunker | `chunker.rs` |
| 3 | Implement LanceDB vector store | `store.rs` |
| 4 | Implement RAG pipeline | `pipeline.rs` |
| 5 | Add RAG Tauri commands | `rag.rs`, `state.rs`, `lib.rs`, `mod.rs` |
| 6 | Rewire frontend AI panel | `ai.ts` |
| 7 | End-to-end RAG test | Validation |
