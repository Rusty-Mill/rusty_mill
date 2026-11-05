---
category: Added
changelog: rusty_erosion, a code-erosion metric (CC concentration and normalized duplication in basis points) for the Let Em Loose erosion gate.
---
## 2026-11-04 - rusty_erosion: the erosion gate's metric

- **Added:** `crates/tools/rusty_erosion`, slice 1 of the erosion gate (card #704, D41, D45, D52). A pure core measures CC concentration (CC in functions above 10, over all CC) and duplication (tokens in a repeated 50-token normalized window, over all tokens), plus total CC, LOC, function count and the worst functions. `Report::delta` compares a candidate with its baseline from exact ratios, so small changes in a large tree are not rounded away. A tree-sitter adapter measures Rust only; other code is listed as `unmeasured`, non-code is ignored, and `#[test]`, `#[cfg(test)]`, `#[cfg(all(test, ..))]`, `tests/` and `tests.rs` are skipped.
- **Known limitations:** no `erosion` binary or host profile yet (slice 2). Macro bodies add tokens but no CC. The metric can be gamed by splitting functions or renaming; the delta's counts are there for calibration to watch. This brings `tree-sitter` and `tree-sitter-rust`, already workspace dependencies used only by the excluded Nexus crates, into the main workspace build.
