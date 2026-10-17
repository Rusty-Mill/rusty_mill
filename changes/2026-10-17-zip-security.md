---
category: Security
changelog: Upgrade both ZIP dependency paths to ZIP 9.0.4 for RUSTSEC-2026-0367, RUSTSEC-2026-0368, and RUSTSEC-2026-0369.
---
## 2026-10-17 - Patch both ZIP dependency paths

- **Fixed:** Upgrade the workspace ZIP dependency to 9.0.4 and the optional DuckDB benchmark dependency to 1.10506.1, whose libduckdb-sys build dependency uses ZIP 9. The lockfile now resolves both paths to ZIP 9.0.4. Preserve deflate support and bundled DuckDB; no advisory exceptions or policy changes.
- **Compatibility:** Propagate ZIP 9's fallible entry-name decoding in Croc. Cover directory transfer layout, binary and empty files, empty directories, and rejected parent traversal; retain the existing Unix symlink confinement test. Add an embedded DuckDB compatibility test for transactions, prepared queries, missing rows, and aggregation without external services.
- **Known limitations:** DuckDB's bundled C++ build remains expensive and retains the repository's existing Windows CI exclusion. This change does not run external database benchmarks or contact providers.
