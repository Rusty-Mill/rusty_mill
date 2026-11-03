---
category: Fixed
changelog: Remove unnecessary MinGW downloads from the Windows compile-only CI job.
---
## 2026-10-27 - Windows compile checks without MinGW

- **Fixed:** remove the full `mingw-w64` installation from the Linux
  `rusty_win32` cross-check job. Both existing `cargo check` and `cargo clippy`
  commands pass with Rust 1.98.1 and its Windows GNU target, separately empty
  Cargo caches and target directories, and no installed MinGW compiler.
- **Changed:** workflow and planner edits select this job even without affected
  Cargo packages; regression checks preserve both exact commands and its
  `required-gate` dependency.
- **Known limitations:** these commands check compilation and lints; they do
  not link or run Windows binaries.
