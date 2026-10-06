//! Vector store backed by `SQLite` for chunk embeddings.
//!
//! Provides CRUD over the `embeddings` table created by schema migration v4.
//! The AI plugin does **not** open its own `SQLite` connection; instead it
//! reaches these operations through storage IPC handlers
//! (`vector_insert`, `vector_query`, `vector_delete_by_file`,
//! `vectorstore_count`) so that storage remains the sole owner of the forge
//! database.
//!
//! Similarity search loads all vectors into memory and ranks them by cosine
//! similarity — appropriate for personal-knowledge-base sizes.
//!
//! # Integrity rules
//!
//! * **Writes are validated before anything is replaced** ([`upsert`]): every
//!   chunk must target the file being written, carry a non-empty, finite
//!   embedding of one shared dimension, and share one `content_hash`. A
//!   rejected write leaves the stored vectors untouched.
//! * **Reads fail loudly on damage.** A stored embedding that is not a blob,
//!   is empty, is not a whole number of `f32`s or holds a non-finite value,
//!   and a file whose chunks disagree about dimension, return
//!   [`StorageError::CorruptFile`] naming the file. They are never skipped.
//! * **Different-dimensional files are not comparable, not corrupt.** After a
//!   model switch, files embedded under the old model keep their old
//!   dimension until re-indexed; [`search`] and near-duplicate detection
//!   exclude them from comparison instead of scoring them.
//! * **Similarity is checked arithmetic**: see [`cosine_similarity`].

use std::collections::HashMap;

use rusqlite::{params, types::ValueRef, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::StorageError;

/// Largest embedding dimension accepted by [`upsert`] and [`search`].
///
/// Mirrors `nexus_ai::vectorstore::MAX_EMBEDDING_DIM`. The AI plugin reaches
/// storage only over IPC, so the two crates cannot share the constant; storage
/// enforces the bound itself so it holds for every caller.
pub const MAX_EMBEDDING_DIM: usize = 12_288;

/// A chunk together with its embedding vector, ready for storage.
///
/// `Serialize`/`Deserialize` so it can round-trip through the IPC layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkEmbedding {
    /// Path of the source file.
    pub file_path: String,
    /// Identifier of the originating block.
    pub block_id: u64,
    /// The textual content of the chunk.
    pub chunk_text: String,
    /// Dense vector representation of the chunk.
    pub embedding: Vec<f32>,
    /// C19 (#372) — hash of the source file's content as of this embed
    /// pass. Every chunk from one `upsert` call shares the same value.
    /// `None` for callers that don't opt into the skip-unchanged-files
    /// optimisation (the row is then always re-embedded, matching
    /// pre-#372 behaviour).
    #[serde(default)]
    pub content_hash: Option<String>,
}

/// A search result returned by [`search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMatch {
    /// Path of the source file.
    pub file_path: String,
    /// Identifier of the originating block.
    pub block_id: u64,
    /// The textual content of the chunk.
    pub chunk_text: String,
    /// Cosine similarity score (higher is more relevant).
    pub score: f32,
}

fn invalid(msg: String) -> StorageError {
    StorageError::InvalidInput(msg)
}

/// Why an embedding is unusable, or `Ok` if it is non-empty, within
/// [`MAX_EMBEDDING_DIM`] and entirely finite.
fn check_embedding(embedding: &[f32]) -> Result<(), String> {
    if embedding.is_empty() {
        return Err("embedding is empty".into());
    }
    if embedding.len() > MAX_EMBEDDING_DIM {
        return Err(format!(
            "embedding has {} dimensions; max is {MAX_EMBEDDING_DIM}",
            embedding.len()
        ));
    }
    if let Some(i) = embedding.iter().position(|v| !v.is_finite()) {
        return Err(format!("embedding component {i} is not finite"));
    }
    Ok(())
}

/// Validate a whole write before any row is touched. See the module docs.
fn validate_chunks(file_path: &str, chunks: &[ChunkEmbedding]) -> Result<(), StorageError> {
    let Some(first) = chunks.first() else {
        return Ok(());
    };
    let dim = first.embedding.len();
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.file_path != file_path {
            return Err(invalid(format!(
                "chunk {i} targets file {:?}, expected {file_path:?}",
                chunk.file_path
            )));
        }
        check_embedding(&chunk.embedding).map_err(|why| invalid(format!("chunk {i}: {why}")))?;
        if chunk.embedding.len() != dim {
            return Err(invalid(format!(
                "chunk {i} has {} dimensions but chunk 0 has {dim}",
                chunk.embedding.len()
            )));
        }
        if chunk.content_hash != first.content_hash {
            return Err(invalid(format!(
                "chunk {i} content_hash differs from chunk 0"
            )));
        }
    }
    if first.content_hash.as_deref() == Some("") {
        return Err(invalid("content_hash is empty".into()));
    }
    Ok(())
}

/// Replace all embeddings for `file_path` with the given chunks.
///
/// The whole write is validated first ([`StorageError::InvalidInput`], see the
/// module docs), so a rejected write leaves the existing rows untouched. Then
/// deletes any existing rows for the file and inserts the new set inside a
/// single transaction. An empty `chunks` slice is allowed and means "this file
/// has no vectors".
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] if validation fails, or
/// [`StorageError::Database`] if the transaction, delete, or insert fails.
pub fn upsert(
    conn: &Connection,
    namespace: &str,
    file_path: &str,
    chunks: &[ChunkEmbedding],
) -> Result<(), StorageError> {
    validate_chunks(file_path, chunks)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM embeddings WHERE namespace = ?1 AND file_path = ?2;",
        params![namespace, file_path],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO embeddings (namespace, file_path, block_id, chunk_text, embedding, content_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch());",
        )?;
        for chunk in chunks {
            let blob = embedding_to_blob(&chunk.embedding);
            #[allow(clippy::cast_possible_wrap)]
            let block_id = chunk.block_id as i64;
            stmt.execute(params![
                namespace,
                file_path,
                block_id,
                chunk.chunk_text,
                blob,
                chunk.content_hash,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// C19 (#372) — the content hash + embedding dimensionality already
/// stored for `file_path`, or `None` when the file must be re-embedded.
///
/// A signature is returned only when **every** stored row of the file in
/// `namespace` agrees: each has a non-empty text `content_hash`, there is
/// exactly one distinct hash, and each `embedding` is a non-empty blob of one
/// shared length that is a whole number of `f32`s. Anything else (no rows,
/// rows predating the feature with a NULL hash, a hash or embedding of the
/// wrong SQLite type, mixed hashes or lengths, an empty or truncated blob)
/// returns `None`, so ordinary indexing re-embeds and repairs the file. It
/// cannot see a non-finite component inside a well-formed blob; [`upsert`]
/// prevents new ones and a forced re-index repairs old ones.
///
/// The dimension is derived from the stored blob's byte length rather
/// than tracked in a separate column, so a provider/model switch that
/// changes the embedding dimensionality is automatically treated as a
/// content mismatch by the caller (comparing against the *current*
/// provider's dimension) even though the file's own content didn't
/// change.
///
/// # Errors
///
/// Returns [`StorageError`] if the underlying query fails.
pub fn stored_signature(
    conn: &Connection,
    namespace: &str,
    file_path: &str,
) -> Result<Option<(String, usize)>, StorageError> {
    // One aggregate pass, no blob reads. `typeof()` matters because SQLite
    // does not enforce column types: a TEXT or INTEGER can sit in the BLOB
    // `embedding` column, and a blob in the hash column (whose TEXT affinity
    // coerces numbers to text but leaves blobs alone).
    let agg = conn.query_row(
        "SELECT COUNT(*),
                SUM(typeof(content_hash) = 'text'),
                COUNT(DISTINCT content_hash),
                MIN(length(content_hash)),
                SUM(typeof(embedding) = 'blob'),
                MIN(length(embedding)),
                MAX(length(embedding)),
                MIN(content_hash)
         FROM embeddings WHERE namespace = ?1 AND file_path = ?2;",
        params![namespace, file_path],
        |r| {
            Ok(SignatureAggregate {
                rows: r.get(0)?,
                text_hashes: r.get(1)?,
                distinct_hashes: r.get(2)?,
                min_hash_len: r.get(3)?,
                blobs: r.get(4)?,
                min_len: r.get(5)?,
                max_len: r.get(6)?,
                // Read dynamically: a non-text hash must mean "no signature",
                // not a type error.
                hash: r.get(7)?,
            })
        },
    )?;
    Ok(agg.signature())
}

/// What one aggregate query learned about a file's stored rows.
struct SignatureAggregate {
    rows: i64,
    text_hashes: Option<i64>,
    distinct_hashes: i64,
    min_hash_len: Option<i64>,
    blobs: Option<i64>,
    min_len: Option<i64>,
    max_len: Option<i64>,
    hash: rusqlite::types::Value,
}

impl SignatureAggregate {
    /// The `(content_hash, dimension)` if every row agrees, else `None`.
    fn signature(self) -> Option<(String, usize)> {
        let consistent = self.rows > 0
            && self.text_hashes == Some(self.rows)
            && self.distinct_hashes == 1
            && self.min_hash_len.is_some_and(|n| n > 0)
            && self.blobs == Some(self.rows)
            && self.min_len == self.max_len
            && self.min_len.is_some_and(|n| n > 0 && n % 4 == 0);
        if !consistent {
            return None;
        }
        let (rusqlite::types::Value::Text(hash), Some(len)) = (self.hash, self.min_len) else {
            return None;
        };
        Some((hash, usize::try_from(len).ok()? / 4))
    }
}

/// Delete all embeddings associated with `file_path` within `namespace`.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the delete statement fails.
pub fn delete_by_file(
    conn: &Connection,
    namespace: &str,
    file_path: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "DELETE FROM embeddings WHERE namespace = ?1 AND file_path = ?2;",
        params![namespace, file_path],
    )?;
    Ok(())
}

/// How a damaged row is reported: names the file, the row and the repair.
fn corrupt(file_path: &str, row_id: i64, block_id: i64, reason: &str) -> StorageError {
    StorageError::CorruptFile {
        path: file_path.to_string(),
        reason: format!(
            "embedding row {row_id} (block {block_id}): {reason}; \
             delete the file's vectors (`vector_delete_by_file`) or re-index it with force to repair"
        ),
    }
}

/// A file whose chunks were embedded with different dimensions.
fn mixed_dimensions(file_path: &str, seen: usize, found: usize) -> StorageError {
    StorageError::CorruptFile {
        path: file_path.to_string(),
        reason: format!(
            "chunks have mixed embedding dimensions ({seen} and {found}); \
             delete the file's vectors (`vector_delete_by_file`) or re-index it with force to repair"
        ),
    }
}

/// Decode the embedding column of `row` at `idx`. The outer `Result` is a
/// `SQLite` read error; the inner one is damage in the stored value.
fn read_embedding(row: &Row<'_>, idx: usize) -> rusqlite::Result<Result<Vec<f32>, String>> {
    Ok(match row.get_ref(idx)? {
        ValueRef::Blob(blob) => decode_embedding(blob),
        other => Err(format!(
            "stored as SQLite {:?}, not a blob",
            other.data_type()
        )),
    })
}

/// Search for chunks most similar to `query_embedding`.
///
/// Loads all stored embeddings, scores each with [`cosine_similarity`], and
/// returns the top `limit` results sorted by descending score. Ties break by
/// `file_path`, then `block_id`, then row id (all ascending) because block ids
/// are not unique, so the same data always returns the same top `limit`.
///
/// Rows of a different dimension than the query (files embedded under another
/// model) and rows with no defined similarity (zero norm) are excluded.
/// Damaged rows are errors, see the module docs.
///
/// # Errors
///
/// Returns [`StorageError::InvalidInput`] for an empty, oversized or
/// non-finite query, [`StorageError::CorruptFile`] for a damaged row or a file
/// with mixed dimensions, and [`StorageError::Database`] if the underlying
/// query fails.
pub fn search(
    conn: &Connection,
    namespace: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<ChunkMatch>, StorageError> {
    check_embedding(query_embedding).map_err(|why| invalid(format!("query embedding: {why}")))?;

    let mut stmt = conn.prepare(
        "SELECT id, file_path, block_id, chunk_text, embedding FROM embeddings WHERE namespace = ?1;",
    )?;
    let mut rows = stmt.query(params![namespace])?;

    let mut dims: HashMap<String, usize> = HashMap::new();
    let mut scored: Vec<(i64, ChunkMatch)> = Vec::new();
    let (mut other_dimension, mut unscorable) = (0_usize, 0_usize);
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let file_path: String = row.get(1)?;
        let block_id: i64 = row.get(2)?;
        let embedding =
            read_embedding(row, 4)?.map_err(|reason| corrupt(&file_path, id, block_id, &reason))?;
        track_dimension(&mut dims, &file_path, embedding.len())?;
        if embedding.len() != query_embedding.len() {
            other_dimension += 1;
            continue;
        }
        let Some(score) = cosine_similarity(query_embedding, &embedding) else {
            unscorable += 1;
            continue;
        };
        let chunk_text: String = row.get(3)?;
        scored.push((
            id,
            ChunkMatch {
                file_path,
                block_id: block_id.cast_unsigned(),
                chunk_text,
                score,
            },
        ));
    }

    scored.sort_by(|(id_a, a), (id_b, b)| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.block_id.cmp(&b.block_id))
            .then_with(|| id_a.cmp(id_b))
    });
    scored.truncate(limit);

    if other_dimension + unscorable > 0 {
        tracing::debug!(
            other_dimension,
            unscorable,
            "vector search excluded rows it cannot compare"
        );
    }
    Ok(scored.into_iter().map(|(_, m)| m).collect())
}

/// Record `dim` as `file_path`'s dimension, or fail if it already has another.
fn track_dimension(
    dims: &mut HashMap<String, usize>,
    file_path: &str,
    dim: usize,
) -> Result<(), StorageError> {
    match dims.get(file_path) {
        Some(&seen) if seen != dim => Err(mixed_dimensions(file_path, seen, dim)),
        Some(_) => Ok(()),
        None => {
            dims.insert(file_path.to_string(), dim);
            Ok(())
        }
    }
}

/// Count the total number of stored embeddings.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the count query fails.
pub fn count(conn: &Connection, namespace: &str) -> Result<usize, StorageError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM embeddings WHERE namespace = ?1;",
        params![namespace],
        |r| r.get(0),
    )?;
    usize::try_from(n).map_err(|_| StorageError::IndexInconsistency {
        details: "embedding count overflowed usize".into(),
    })
}

/// Running sum of one file's chunk embeddings.
struct Pool {
    file_path: String,
    sum: Vec<f64>,
    count: u64,
}

impl Pool {
    fn new(file_path: String, first: &[f32]) -> Self {
        let mut pool = Self {
            file_path,
            sum: vec![0.0; first.len()],
            count: 0,
        };
        pool.add(first);
        pool
    }

    fn add(&mut self, embedding: &[f32]) {
        for (acc, v) in self.sum.iter_mut().zip(embedding) {
            *acc += f64::from(*v);
        }
        self.count += 1;
    }

    fn finish(self) -> Result<(String, Vec<f32>), StorageError> {
        let mean = mean_to_f32(&self.file_path, &self.sum, self.count)?;
        Ok((self.file_path, mean))
    }
}

/// Divide in f64, then narrow to f32, refusing a result that is not finite.
///
/// The mean of finite `f32` values cannot exceed `f32::MAX`, so this cannot
/// fail for rows that passed validation; the check keeps the guarantee
/// explicit instead of relying on that argument.
fn mean_to_f32(file_path: &str, sum: &[f64], count: u64) -> Result<Vec<f32>, StorageError> {
    #[allow(clippy::cast_precision_loss)]
    let divisor = count as f64;
    sum.iter()
        .map(|total| {
            #[allow(clippy::cast_possible_truncation)]
            let mean = (total / divisor) as f32;
            if mean.is_finite() {
                Ok(mean)
            } else {
                Err(StorageError::CorruptFile {
                    path: file_path.to_string(),
                    reason: "mean embedding is not finite".to_string(),
                })
            }
        })
        .collect()
}

/// Mean-pool every chunk embedding for each file in `namespace` into a
/// single per-file vector — one row per file, ordered by path, suitable for
/// whole-note similarity comparisons (C23 / #376 near-duplicate note
/// detection).
///
/// Rows are read in `file_path, block_id, id` order and summed in f64, so the
/// result is deterministic. A file whose chunks have different dimensions is
/// an error, not averaged over a subset; files of different dimensions from
/// each other are returned side by side (callers compare only equal
/// dimensions, see [`cosine_similarity`]). Damaged rows are errors, see the
/// module docs.
///
/// # Errors
///
/// Returns [`StorageError::CorruptFile`] for a damaged row, a file with mixed
/// dimensions or a non-finite mean, and [`StorageError::Database`] if the
/// underlying query fails.
pub fn mean_embeddings_by_file(
    conn: &Connection,
    namespace: &str,
) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, block_id, embedding FROM embeddings \
         WHERE namespace = ?1 ORDER BY file_path, block_id, id;",
    )?;
    let mut rows = stmt.query(params![namespace])?;

    let mut out = Vec::new();
    let mut current: Option<Pool> = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let file_path: String = row.get(1)?;
        let block_id: i64 = row.get(2)?;
        let embedding =
            read_embedding(row, 3)?.map_err(|reason| corrupt(&file_path, id, block_id, &reason))?;
        match current.as_mut() {
            Some(pool) if pool.file_path == file_path => {
                if pool.sum.len() != embedding.len() {
                    return Err(mixed_dimensions(
                        &file_path,
                        pool.sum.len(),
                        embedding.len(),
                    ));
                }
                pool.add(&embedding);
            }
            _ => {
                if let Some(done) = current.take() {
                    out.push(done.finish()?);
                }
                current = Some(Pool::new(file_path, &embedding));
            }
        }
    }
    if let Some(done) = current.take() {
        out.push(done.finish()?);
    }
    Ok(out)
}

// ─── Serialization helpers ───────────────────────────────────────────────────

/// Serialize an embedding vector to a flat little-endian byte blob.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize a flat little-endian byte blob back into an embedding vector.
///
/// The `Err` text says why the blob is unusable: empty, not a whole number of
/// `f32`s, or holding a non-finite component.
fn decode_embedding(blob: &[u8]) -> Result<Vec<f32>, String> {
    if blob.is_empty() {
        return Err("blob is empty".into());
    }
    let (chunks, remainder) = blob.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(format!("blob is {} bytes, not a multiple of 4", blob.len()));
    }
    let embedding: Vec<f32> = chunks.iter().map(|&b| f32::from_le_bytes(b)).collect();
    check_embedding(&embedding)?;
    Ok(embedding)
}

/// Cosine similarity of two embeddings, or `None` when none is defined.
///
/// `None` for an empty vector, a dimension mismatch, a zero-norm vector, or
/// any non-finite component: those pairs are not comparable and callers must
/// exclude them rather than rank them. Accumulates in f64, because finite f32
/// values can overflow an f32 dot product or norm (two `[1e30, 1e30]` vectors
/// would otherwise score `NaN`). The result is clamped to `[-1.0, 1.0]`.
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (&x, &y) in a.iter().zip(b) {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (x, y) = (f64::from(x), f64::from(y));
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    let cosine = (dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(-1.0, 1.0);
    #[allow(clippy::cast_possible_truncation)]
    let narrowed = cosine as f32;
    // `+ 0.0` turns -0.0 into 0.0 so `total_cmp` does not split equal scores.
    Some(narrowed + 0.0)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::configure_pragmas(&conn).unwrap();
        crate::schema::migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let score = cosine_similarity(&v, &v).expect("defined");
        assert!((score - 1.0).abs() < 1e-6, "expected ~1.0, got {score}");
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let score = cosine_similarity(&a, &b).expect("defined");
        assert!(score.abs() < 1e-6, "expected ~0.0, got {score}");
        assert!(score.is_sign_positive(), "an exact zero must not be -0.0");
    }

    #[test]
    fn embedding_blob_round_trip() {
        let original = vec![1.0_f32, -2.5, 3.15, 0.0, f32::MAX];
        let blob = embedding_to_blob(&original);
        let restored = decode_embedding(&blob).expect("valid blob");
        assert_eq!(original, restored);
    }

    #[test]
    fn upsert_and_search() {
        let conn = setup_db();

        let chunks = vec![
            ChunkEmbedding {
                file_path: "a.md".into(),
                block_id: 1,
                chunk_text: "Rust is great".into(),
                embedding: vec![1.0, 0.0, 0.0],
                content_hash: None,
            },
            ChunkEmbedding {
                file_path: "a.md".into(),
                block_id: 2,
                chunk_text: "Python is nice".into(),
                embedding: vec![0.0, 1.0, 0.0],
                content_hash: None,
            },
        ];

        upsert(&conn, "notes", "a.md", &chunks).unwrap();

        let results = search(&conn, "notes", &[0.9, 0.1, 0.0], 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].chunk_text, "Rust is great");
        assert!(results[0].score > 0.9);
    }

    #[test]
    fn namespaces_are_isolated() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "a.md",
            &[ChunkEmbedding {
                file_path: "a.md".into(),
                block_id: 1,
                chunk_text: "a note".into(),
                embedding: vec![1.0, 0.0],
                content_hash: None,
            }],
        )
        .unwrap();
        upsert(
            &conn,
            "memory",
            "memory://x",
            &[ChunkEmbedding {
                file_path: "memory://x".into(),
                block_id: 0,
                chunk_text: "a memory".into(),
                embedding: vec![1.0, 0.0],
                content_hash: None,
            }],
        )
        .unwrap();

        // Each namespace sees only its own rows.
        assert_eq!(count(&conn, "notes").unwrap(), 1);
        assert_eq!(count(&conn, "memory").unwrap(), 1);
        let notes = search(&conn, "notes", &[1.0, 0.0], 5).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].chunk_text, "a note");
        let mem = search(&conn, "memory", &[1.0, 0.0], 5).unwrap();
        assert_eq!(mem.len(), 1);
        assert_eq!(mem[0].chunk_text, "a memory");

        // Deleting in one namespace leaves the other intact.
        delete_by_file(&conn, "memory", "memory://x").unwrap();
        assert_eq!(count(&conn, "memory").unwrap(), 0);
        assert_eq!(count(&conn, "notes").unwrap(), 1);
    }

    #[test]
    fn upsert_replaces_existing() {
        let conn = setup_db();

        let v1 = vec![ChunkEmbedding {
            file_path: "b.md".into(),
            block_id: 1,
            chunk_text: "old".into(),
            embedding: vec![1.0, 0.0],
            content_hash: None,
        }];
        upsert(&conn, "notes", "b.md", &v1).unwrap();
        assert_eq!(count(&conn, "notes").unwrap(), 1);

        let v2 = vec![ChunkEmbedding {
            file_path: "b.md".into(),
            block_id: 1,
            chunk_text: "new".into(),
            embedding: vec![0.0, 1.0],
            content_hash: None,
        }];
        upsert(&conn, "notes", "b.md", &v2).unwrap();
        assert_eq!(count(&conn, "notes").unwrap(), 1);
    }

    #[test]
    fn mean_embeddings_by_file_pools_chunks_per_file() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "a.md",
            &[
                ChunkEmbedding {
                    file_path: "a.md".into(),
                    block_id: 1,
                    chunk_text: "one".into(),
                    embedding: vec![1.0, 0.0],
                    content_hash: None,
                },
                ChunkEmbedding {
                    file_path: "a.md".into(),
                    block_id: 2,
                    chunk_text: "two".into(),
                    embedding: vec![0.0, 1.0],
                    content_hash: None,
                },
            ],
        )
        .unwrap();
        upsert(
            &conn,
            "notes",
            "b.md",
            &[ChunkEmbedding {
                file_path: "b.md".into(),
                block_id: 1,
                chunk_text: "solo".into(),
                embedding: vec![2.0, 2.0],
                content_hash: None,
            }],
        )
        .unwrap();

        let means = mean_embeddings_by_file(&conn, "notes").unwrap();
        assert_eq!(means.len(), 2);
        let a = means.iter().find(|(p, _)| p == "a.md").unwrap();
        assert!((a.1[0] - 0.5).abs() < 1e-6);
        assert!((a.1[1] - 0.5).abs() < 1e-6);
        let b = means.iter().find(|(p, _)| p == "b.md").unwrap();
        assert!((b.1[0] - 2.0).abs() < 1e-6);
        assert!((b.1[1] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn mean_embeddings_by_file_rejects_mixed_dimensions_within_a_file() {
        let conn = setup_db();
        // A stale row from a different embedding model, inserted directly
        // (through `upsert` a mixed write is now rejected up front).
        upsert(
            &conn,
            "notes",
            "c.md",
            &[chunk("c.md", 1, vec![1.0, 0.0, 0.0], None)],
        )
        .unwrap();
        insert_raw(
            &conn,
            "notes",
            "c.md",
            2,
            "stale",
            blob(&[1.0, 0.0]),
            null(),
        );

        let err = mean_embeddings_by_file(&conn, "notes").unwrap_err();
        assert!(
            matches!(&err, StorageError::CorruptFile { path, reason }
                if path == "c.md" && reason.contains("mixed embedding dimensions")),
            "{err:?}"
        );
    }

    #[test]
    fn delete_by_file_removes_embeddings() {
        let conn = setup_db();

        let chunks = vec![ChunkEmbedding {
            file_path: "c.md".into(),
            block_id: 1,
            chunk_text: "data".into(),
            embedding: vec![1.0],
            content_hash: None,
        }];
        upsert(&conn, "notes", "c.md", &chunks).unwrap();
        assert_eq!(count(&conn, "notes").unwrap(), 1);

        delete_by_file(&conn, "notes", "c.md").unwrap();
        assert_eq!(count(&conn, "notes").unwrap(), 0);
    }

    // ── C19 (#372) — stored_signature ────────────────────────────────

    #[test]
    fn stored_signature_returns_none_when_nothing_stored() {
        let conn = setup_db();
        assert_eq!(
            stored_signature(&conn, "notes", "missing.md").unwrap(),
            None
        );
    }

    #[test]
    fn stored_signature_returns_hash_and_dimension_after_upsert() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "d.md",
            &[ChunkEmbedding {
                file_path: "d.md".into(),
                block_id: 1,
                chunk_text: "hello".into(),
                embedding: vec![1.0, 2.0, 3.0],
                content_hash: Some("abc123".into()),
            }],
        )
        .unwrap();

        let sig = stored_signature(&conn, "notes", "d.md").unwrap();
        assert_eq!(sig, Some(("abc123".to_string(), 3)));
    }

    #[test]
    fn stored_signature_is_none_for_rows_predating_the_feature() {
        // A row with content_hash left NULL (e.g. from before migration
        // 011, or a caller that opted out) must read back as "unknown",
        // not as an empty-string match.
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "e.md",
            &[ChunkEmbedding {
                file_path: "e.md".into(),
                block_id: 1,
                chunk_text: "hello".into(),
                embedding: vec![1.0],
                content_hash: None,
            }],
        )
        .unwrap();

        assert_eq!(stored_signature(&conn, "notes", "e.md").unwrap(), None);
    }

    #[test]
    fn stored_signature_is_scoped_to_namespace() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "f.md",
            &[ChunkEmbedding {
                file_path: "f.md".into(),
                block_id: 1,
                chunk_text: "hello".into(),
                embedding: vec![1.0, 2.0],
                content_hash: Some("notes-hash".into()),
            }],
        )
        .unwrap();

        assert_eq!(stored_signature(&conn, "memory", "f.md").unwrap(), None);
    }

    // ─── Integrity: helpers ──────────────────────────────────────────────────

    use rusqlite::types::Value;

    fn null() -> Value {
        Value::Null
    }

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    fn blob(v: &[f32]) -> Value {
        Value::Blob(embedding_to_blob(v))
    }

    fn chunk(file: &str, block: u64, embedding: Vec<f32>, hash: Option<&str>) -> ChunkEmbedding {
        ChunkEmbedding {
            file_path: file.into(),
            block_id: block,
            chunk_text: format!("{file}#{block}"),
            embedding,
            content_hash: hash.map(str::to_string),
        }
    }

    /// Insert a row directly, bypassing `upsert`'s validation, to model
    /// damaged or legacy data. Returns the row id.
    #[allow(clippy::needless_pass_by_value)] // call sites build the values inline
    fn insert_raw(
        conn: &Connection,
        namespace: &str,
        file: &str,
        block: impl rusqlite::ToSql,
        chunk_text: &str,
        embedding: Value,
        hash: Value,
    ) -> i64 {
        conn.execute(
            "INSERT INTO embeddings (namespace, file_path, block_id, chunk_text, embedding, content_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch());",
            params![namespace, file, block, chunk_text, embedding, hash],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    type StoredRow = (i64, String, i64, Vec<u8>, Option<String>);

    /// Every stored row, for proving a rejected write changed nothing.
    fn snapshot(conn: &Connection) -> Vec<StoredRow> {
        let mut stmt = conn
            .prepare("SELECT id, file_path, block_id, embedding, content_hash FROM embeddings ORDER BY id;")
            .unwrap();
        stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// Values a stored `embedding` can hold that are not a usable vector.
    fn damaged_embeddings() -> Vec<(&'static str, Value)> {
        vec![
            ("truncated blob (7 bytes)", Value::Blob(vec![0; 7])),
            ("empty blob", Value::Blob(Vec::new())),
            ("text value", text("abc")),
            ("integer value", Value::Integer(5)),
            ("NaN component", blob(&[1.0, f32::NAN])),
            ("infinite component", blob(&[f32::INFINITY, 1.0])),
        ]
    }

    // ─── Integrity: similarity arithmetic ────────────────────────────────────

    #[test]
    fn cosine_is_none_when_undefined() {
        assert_eq!(cosine_similarity(&[], &[]), None, "empty");
        assert_eq!(
            cosine_similarity(&[1.0, 0.0], &[1.0]),
            None,
            "dimension mismatch"
        );
        assert_eq!(
            cosine_similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0]),
            None,
            "prefix is not a match"
        );
        assert_eq!(
            cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]),
            None,
            "zero norm"
        );
        assert_eq!(
            cosine_similarity(&[1.0, f32::NAN], &[1.0, 0.0]),
            None,
            "NaN"
        );
        assert_eq!(
            cosine_similarity(&[1.0, 0.0], &[f32::INFINITY, 0.0]),
            None,
            "infinity"
        );
    }

    #[test]
    fn cosine_survives_finite_values_that_overflow_f32() {
        // In f32 the dot product of two [1e30, 1e30] vectors is infinite and
        // the score is NaN; in f64 it is ~1.
        for v in [[1e30_f32, 1e30], [f32::MAX, f32::MAX], [1e-30, 1e-30]] {
            let score = cosine_similarity(&v, &v).expect("defined");
            assert!(
                score.is_finite() && (score - 1.0).abs() < 1e-6,
                "{v:?} -> {score}"
            );
        }
    }

    #[test]
    fn cosine_is_clamped_to_unit_range() {
        let v = [0.1_f32, 0.2, 0.3];
        let score = cosine_similarity(&v, &v).expect("defined");
        assert!((0.999_999..=1.0).contains(&score), "{score}");
        let opposite = [-0.1_f32, -0.2, -0.3];
        let score = cosine_similarity(&v, &opposite).expect("defined");
        assert!((-1.0..=-0.999_999).contains(&score), "{score}");
    }

    #[test]
    fn decode_embedding_rejects_unusable_blobs() {
        for (what, value) in damaged_embeddings() {
            if let Value::Blob(b) = value {
                assert!(decode_embedding(&b).is_err(), "{what}");
            }
        }
        assert_eq!(
            decode_embedding(&embedding_to_blob(&[1.5, -2.0])),
            Ok(vec![1.5, -2.0])
        );
    }

    // ─── Integrity: search ───────────────────────────────────────────────────

    #[test]
    fn search_rejects_unusable_queries() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "a.md",
            &[chunk("a.md", 1, vec![1.0, 0.0], None)],
        )
        .unwrap();
        let oversized = vec![1.0; MAX_EMBEDDING_DIM + 1];
        for (what, query) in [
            ("empty", Vec::new()),
            ("NaN", vec![1.0, f32::NAN]),
            ("infinite", vec![f32::INFINITY, 0.0]),
            ("oversized", oversized),
        ] {
            let err = search(&conn, "notes", &query, 5).unwrap_err();
            assert!(
                matches!(err, StorageError::InvalidInput(_)),
                "{what}: {err:?}"
            );
        }
    }

    #[test]
    fn search_errors_on_every_kind_of_damaged_row_and_names_the_file() {
        for (what, damaged) in damaged_embeddings() {
            let conn = setup_db();
            upsert(
                &conn,
                "notes",
                "ok.md",
                &[chunk("ok.md", 1, vec![1.0, 0.0], None)],
            )
            .unwrap();
            let id = insert_raw(&conn, "notes", "bad.md", 7, "bad", damaged, null());

            let err = search(&conn, "notes", &[1.0, 0.0], 5).unwrap_err();
            assert!(
                matches!(&err, StorageError::CorruptFile { path, reason }
                    if path == "bad.md"
                        && reason.contains(&format!("row {id}"))
                        && reason.contains("block 7")),
                "{what}: {err:?}"
            );
        }
    }

    #[test]
    fn search_propagates_rows_that_cannot_be_read() {
        let conn = setup_db();
        // A non-numeric block id cannot be read as an integer; this used to
        // be dropped silently by `filter_map(Result::ok)`.
        insert_raw(
            &conn,
            "notes",
            "a.md",
            "not-a-number",
            "t",
            blob(&[1.0, 0.0]),
            null(),
        );
        let err = search(&conn, "notes", &[1.0, 0.0], 5).unwrap_err();
        assert!(matches!(err, StorageError::Database(_)), "{err:?}");
    }

    #[test]
    fn search_excludes_files_of_another_dimension_instead_of_scoring_them() {
        let conn = setup_db();
        // A stale 2-d file used to score a perfect 1.0 against a 3-d query.
        upsert(
            &conn,
            "notes",
            "old.md",
            &[chunk("old.md", 1, vec![1.0, 0.0], None)],
        )
        .unwrap();
        upsert(
            &conn,
            "notes",
            "new.md",
            &[chunk("new.md", 1, vec![1.0, 0.0, 0.0], None)],
        )
        .unwrap();

        let hits = search(&conn, "notes", &[1.0, 0.0, 0.0], 5).unwrap();
        assert_eq!(
            hits.iter()
                .map(|h| h.file_path.as_str())
                .collect::<Vec<_>>(),
            ["new.md"]
        );
        let hits = search(&conn, "notes", &[1.0, 0.0], 5).unwrap();
        assert_eq!(
            hits.iter()
                .map(|h| h.file_path.as_str())
                .collect::<Vec<_>>(),
            ["old.md"]
        );
    }

    #[test]
    fn search_rejects_a_file_with_mixed_dimensions_even_if_one_dimension_matches() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "m.md",
            &[chunk("m.md", 1, vec![1.0, 0.0], None)],
        )
        .unwrap();
        insert_raw(
            &conn,
            "notes",
            "m.md",
            2,
            "other model",
            blob(&[1.0, 0.0, 0.0]),
            null(),
        );
        let err = search(&conn, "notes", &[1.0, 0.0], 5).unwrap_err();
        assert!(
            matches!(&err, StorageError::CorruptFile { path, reason }
                if path == "m.md" && reason.contains("mixed embedding dimensions")),
            "{err:?}"
        );
    }

    #[test]
    fn search_scores_extreme_finite_values() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "x.md",
            &[chunk("x.md", 1, vec![1e30, 1e30], None)],
        )
        .unwrap();
        upsert(
            &conn,
            "notes",
            "y.md",
            &[chunk("y.md", 1, vec![f32::MAX, f32::MAX], None)],
        )
        .unwrap();
        let hits = search(&conn, "notes", &[1e30, 1e30], 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits
            .iter()
            .all(|h| h.score.is_finite() && (h.score - 1.0).abs() < 1e-6));
    }

    #[test]
    fn search_excludes_zero_norm_rows() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "z.md",
            &[
                chunk("z.md", 1, vec![0.0, 0.0], None),
                chunk("z.md", 2, vec![1.0, 0.0], None),
            ],
        )
        .unwrap();
        let hits = search(&conn, "notes", &[1.0, 0.0], 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].block_id, 2);
        // A zero query is finite and valid but has no similarity to anything.
        assert!(search(&conn, "notes", &[0.0, 0.0], 5).unwrap().is_empty());
    }

    #[test]
    fn search_orders_ties_by_path_then_block_then_row_id() {
        let conn = setup_db();
        let e = || blob(&[1.0, 0.0]);
        // Inserted out of order, with a duplicated block id in one file.
        insert_raw(&conn, "notes", "b.md", 1, "b1", e(), null());
        insert_raw(&conn, "notes", "a.md", 2, "a2", e(), null());
        insert_raw(&conn, "notes", "a.md", 1, "a1-first", e(), null());
        insert_raw(&conn, "notes", "a.md", 1, "a1-second", e(), null());

        let order = |limit| {
            search(&conn, "notes", &[1.0, 0.0], limit)
                .unwrap()
                .into_iter()
                .map(|h| h.chunk_text)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(10), ["a1-first", "a1-second", "a2", "b1"]);
        // The cut-off is stable too: the same two rows every time.
        assert_eq!(order(2), ["a1-first", "a1-second"]);
        assert_eq!(order(2), order(2));
    }

    // ─── Integrity: averaging ────────────────────────────────────────────────

    #[test]
    fn mean_errors_on_every_kind_of_damaged_row_and_names_the_file() {
        for (what, damaged) in damaged_embeddings() {
            let conn = setup_db();
            upsert(
                &conn,
                "notes",
                "ok.md",
                &[chunk("ok.md", 1, vec![1.0, 0.0], None)],
            )
            .unwrap();
            insert_raw(&conn, "notes", "bad.md", 3, "bad", damaged, null());
            let err = mean_embeddings_by_file(&conn, "notes").unwrap_err();
            assert!(
                matches!(&err, StorageError::CorruptFile { path, .. } if path == "bad.md"),
                "{what}: {err:?}"
            );
        }
    }

    #[test]
    fn mean_propagates_rows_that_cannot_be_read() {
        let conn = setup_db();
        insert_raw(
            &conn,
            "notes",
            "a.md",
            "not-a-number",
            "t",
            blob(&[1.0]),
            null(),
        );
        let err = mean_embeddings_by_file(&conn, "notes").unwrap_err();
        assert!(matches!(err, StorageError::Database(_)), "{err:?}");
    }

    #[test]
    fn mean_keeps_files_of_different_dimensions_side_by_side_in_path_order() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "b.md",
            &[chunk("b.md", 1, vec![1.0, 2.0, 3.0], None)],
        )
        .unwrap();
        upsert(
            &conn,
            "notes",
            "a.md",
            &[
                chunk("a.md", 1, vec![1.0, 3.0], None),
                chunk("a.md", 2, vec![3.0, 5.0], None),
            ],
        )
        .unwrap();
        let means = mean_embeddings_by_file(&conn, "notes").unwrap();
        assert_eq!(
            means,
            vec![
                ("a.md".to_string(), vec![2.0, 4.0]),
                ("b.md".to_string(), vec![1.0, 2.0, 3.0])
            ]
        );
    }

    #[test]
    fn mean_of_extreme_finite_values_stays_finite() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "e.md",
            &[
                chunk("e.md", 1, vec![f32::MAX, -f32::MAX], None),
                chunk("e.md", 2, vec![f32::MAX, -f32::MAX], None),
            ],
        )
        .unwrap();
        let means = mean_embeddings_by_file(&conn, "notes").unwrap();
        // Summed in f32 this would overflow to infinity.
        assert_eq!(means, vec![("e.md".to_string(), vec![f32::MAX, -f32::MAX])]);
    }

    #[test]
    fn mean_to_f32_refuses_a_result_that_is_not_finite() {
        assert_eq!(
            mean_to_f32("f.md", &[4.0, -2.0], 2).unwrap(),
            vec![2.0, -1.0]
        );
        // Unreachable from validated rows (the mean of finite f32 values fits
        // in f32), so exercise the guard directly.
        for sum in [1e300_f64, -1e300, f64::INFINITY, f64::NAN] {
            let err = mean_to_f32("f.md", &[sum], 1).unwrap_err();
            assert!(
                matches!(&err, StorageError::CorruptFile { path, .. } if path == "f.md"),
                "{sum}: {err:?}"
            );
        }
    }

    // ─── Integrity: stored signature ─────────────────────────────────────────

    fn signature_of(rows: &[(Value, Value)]) -> Option<(String, usize)> {
        let conn = setup_db();
        for (i, (embedding, hash)) in rows.iter().enumerate() {
            insert_raw(
                &conn,
                "notes",
                "s.md",
                i64::try_from(i).unwrap(),
                "t",
                embedding.clone(),
                hash.clone(),
            );
        }
        stored_signature(&conn, "notes", "s.md").unwrap()
    }

    #[test]
    fn stored_signature_accepts_only_a_fully_consistent_file() {
        let good = blob(&[1.0, 2.0, 3.0]);
        assert_eq!(
            signature_of(&[(good.clone(), text("h")), (good.clone(), text("h"))]),
            Some(("h".to_string(), 3))
        );
    }

    #[test]
    fn stored_signature_is_none_when_any_row_is_inconsistent() {
        let good = blob(&[1.0, 2.0, 3.0]);
        let cases: Vec<(&str, Vec<(Value, Value)>)> = vec![
            ("no rows", vec![]),
            (
                "mixed hashes",
                vec![(good.clone(), text("a")), (good.clone(), text("b"))],
            ),
            (
                "one NULL hash",
                vec![(good.clone(), text("a")), (good.clone(), null())],
            ),
            ("all NULL hashes", vec![(good.clone(), null())]),
            ("empty hash", vec![(good.clone(), text(""))]),
            // The column has TEXT affinity, so an integer is coerced to text
            // on insert; a blob is not, and is not a usable hash.
            (
                "blob hash",
                vec![(good.clone(), Value::Blob(vec![1, 2, 3]))],
            ),
            ("empty blob", vec![(Value::Blob(Vec::new()), text("a"))]),
            (
                "text in the blob column",
                vec![(text("abcdefgh"), text("a"))],
            ),
            (
                "integer in the blob column",
                vec![(Value::Integer(7), text("a"))],
            ),
            ("truncated blob", vec![(Value::Blob(vec![0; 6]), text("a"))]),
            (
                "mixed lengths",
                vec![(good.clone(), text("a")), (blob(&[1.0, 2.0]), text("a"))],
            ),
            (
                "one bad row among good ones",
                vec![
                    (good.clone(), text("a")),
                    (good.clone(), text("a")),
                    (Value::Blob(vec![0; 6]), text("a")),
                ],
            ),
        ];
        for (what, rows) in cases {
            assert_eq!(signature_of(&rows), None, "{what}");
        }
    }

    #[test]
    fn stored_signature_ignores_other_files_and_namespaces() {
        let conn = setup_db();
        upsert(
            &conn,
            "notes",
            "a.md",
            &[chunk("a.md", 1, vec![1.0, 2.0], Some("h"))],
        )
        .unwrap();
        insert_raw(
            &conn,
            "notes",
            "b.md",
            1,
            "t",
            Value::Blob(vec![0; 3]),
            text("x"),
        );
        insert_raw(
            &conn,
            "memory",
            "a.md",
            1,
            "t",
            Value::Blob(vec![0; 3]),
            text("y"),
        );
        assert_eq!(
            stored_signature(&conn, "notes", "a.md").unwrap(),
            Some(("h".to_string(), 2))
        );
    }

    // ─── Integrity: validated writes ─────────────────────────────────────────

    fn seed_two_files(conn: &Connection) {
        upsert(
            conn,
            "notes",
            "a.md",
            &[
                chunk("a.md", 1, vec![1.0, 0.0], Some("ha")),
                chunk("a.md", 2, vec![0.0, 1.0], Some("ha")),
            ],
        )
        .unwrap();
        upsert(
            conn,
            "notes",
            "b.md",
            &[chunk("b.md", 1, vec![1.0, 1.0], Some("hb"))],
        )
        .unwrap();
    }

    #[test]
    fn a_rejected_write_leaves_every_stored_row_untouched() {
        let conn = setup_db();
        seed_two_files(&conn);
        let before = snapshot(&conn);

        let good = |block| chunk("a.md", block, vec![5.0, 5.0], Some("new"));
        let rejected: Vec<(&str, Vec<ChunkEmbedding>)> = vec![
            (
                "chunk for another file",
                vec![good(1), chunk("b.md", 2, vec![5.0, 5.0], Some("new"))],
            ),
            (
                "empty embedding",
                vec![good(1), chunk("a.md", 2, vec![], Some("new"))],
            ),
            (
                "NaN",
                vec![good(1), chunk("a.md", 2, vec![f32::NAN, 1.0], Some("new"))],
            ),
            (
                "infinity",
                vec![
                    good(1),
                    chunk("a.md", 2, vec![f32::NEG_INFINITY, 1.0], Some("new")),
                ],
            ),
            (
                "mixed dimensions",
                vec![good(1), chunk("a.md", 2, vec![1.0, 2.0, 3.0], Some("new"))],
            ),
            (
                "mixed hashes",
                vec![good(1), chunk("a.md", 2, vec![5.0, 5.0], Some("other"))],
            ),
            (
                "hash on some chunks only",
                vec![good(1), chunk("a.md", 2, vec![5.0, 5.0], None)],
            ),
            (
                "empty hash",
                vec![chunk("a.md", 1, vec![5.0, 5.0], Some(""))],
            ),
            (
                "oversized embedding",
                vec![chunk(
                    "a.md",
                    1,
                    vec![1.0; MAX_EMBEDDING_DIM + 1],
                    Some("new"),
                )],
            ),
        ];
        for (what, chunks) in rejected {
            let err = upsert(&conn, "notes", "a.md", &chunks).unwrap_err();
            assert!(
                matches!(err, StorageError::InvalidInput(_)),
                "{what}: {err:?}"
            );
            assert_eq!(snapshot(&conn), before, "{what}: rows changed");
        }
    }

    #[test]
    fn a_chunk_for_another_file_does_not_touch_that_file() {
        let conn = setup_db();
        seed_two_files(&conn);
        let before = snapshot(&conn);
        // Previously the delete used the argument path but the insert used
        // each chunk's own path, adding rows to b.md without replacing its.
        let err = upsert(
            &conn,
            "notes",
            "a.md",
            &[chunk("b.md", 9, vec![1.0, 1.0], Some("x"))],
        )
        .unwrap_err();
        assert!(
            matches!(&err, StorageError::InvalidInput(m) if m.contains("b.md")),
            "{err:?}"
        );
        assert_eq!(snapshot(&conn), before);
    }

    #[test]
    fn a_valid_write_replaces_the_file_and_allows_repeated_block_ids() {
        let conn = setup_db();
        seed_two_files(&conn);
        // A split block yields several chunks with one block id.
        upsert(
            &conn,
            "notes",
            "a.md",
            &[
                chunk("a.md", 4, vec![2.0, 2.0, 2.0], Some("h2")),
                chunk("a.md", 4, vec![3.0, 3.0, 3.0], Some("h2")),
            ],
        )
        .unwrap();
        assert_eq!(
            count(&conn, "notes").unwrap(),
            3,
            "a.md replaced by two rows, b.md kept"
        );
        assert_eq!(
            stored_signature(&conn, "notes", "a.md").unwrap(),
            Some(("h2".to_string(), 3))
        );
        assert_eq!(
            stored_signature(&conn, "notes", "b.md").unwrap(),
            Some(("hb".to_string(), 2))
        );
    }

    #[test]
    fn an_empty_write_clears_the_file() {
        let conn = setup_db();
        seed_two_files(&conn);
        upsert(&conn, "notes", "a.md", &[]).unwrap();
        assert_eq!(count(&conn, "notes").unwrap(), 1);
    }
}
