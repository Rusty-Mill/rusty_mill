---
category: Security
changelog: rusty-search moves to tantivy 0.27, which brings zstd 0.14.3 and clears RUSTSEC-2026-0391, -0398, -0403 and -0410.
---
## 2026-10-26 - tantivy 0.27 for the zstd advisories

- **Security:** four zstd advisories published 2026-10-26 (RUSTSEC-2026-0391, RUSTSEC-2026-0398, RUSTSEC-2026-0403, RUSTSEC-2026-0410) failed `cargo-deny` on every PR, merge-queue entry and the nightly sweep. zstd 0.13 came in only through tantivy 0.26, which `rusty-search-tantivy` uses. The workspace now pins `tantivy = "0.27"`, which resolves zstd 0.14.3, zstd-safe 8.2.0 and zstd-sys 2.1.2. rusty-search's tests and clippy pass unchanged.
- **Compatibility:** tantivy 0.26.1 and 0.27.0 both write index format 7 and accept formats 4 through 7, so existing 0.26 indexes remain readable without a rebuild. The update adds four small transitive crates (frostem, serde-tuple-vec-map, unwrap-infallible, datasketches 0.5), all under licenses `deny.toml` already allows.
