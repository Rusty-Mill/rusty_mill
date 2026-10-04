# Changelog

The crate's version tracks its API and behaviour. Its file formats are
versioned inside the files: every file has a header version, and every record
type has a schema tag (`SchemaTag`). A consumer that changes a record's layout
must change that record's tag and upgrade old files at open; the engine
refuses a table whose tag it does not expect. No format version or magic
constant changed in the releases below.

Newest first.

## 0.2.0 (2026-10-04)

The first release since the engine was extracted from `rusty_multimodal_db`
(2026-09-27).

### Added
- Full-text query builders for prefix matches (`any_of_prefix`), `AND` (`all_of`) and `NOT` (`except`).
- Full-text column filters (`in_column`, `in_columns`) and column exclusion (`except_columns`).
- `for_each_token`, a tokenizer that reuses one buffer.
- `Reversed::inner`, so a stack with two `Reversed` layers can reach the inner
  one's children.
- Journal batches that restore and move records.
- Test support for the above, and tests for store lifecycle, change-feed and
  task-manager recipes, plus a comparison against SQLite's FTS5 (`tests/fulltext_vs_fts5.rs`).
- Benches: `fulltext_prefix`, and a comparison with SQLite at matched durability.

### Changed
- The full-text index numbers each term and document once instead of copying
  ids and terms into every posting. At 15,000 memories, opening took 1.1 to
  1.3 s with 110 MB resident, where it took 4.5 s with 622 MB. Rankings,
  scores and snippets are unchanged.
- A prefix query reads one range of a sorted vocabulary instead of scanning
  every term.
- A store keeps its insert log and slot file open between writes, where each
  insert used to open them twice.

### Compatibility
- No file format changed. Public API additions only; nothing was removed.

## 0.1.0 (2026-09-27)

The engine as extracted: the generic mmap-backed record store with composable
layers, durable slot files, record blobs, the write journal and full-text index.
