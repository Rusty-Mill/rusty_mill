# Task: `slug`

Add `pub fn slug(input: &str) -> String` to `src/lib.rs` of this crate.

Acceptance criteria:

1. Letters are lower-cased; ASCII letters and digits are kept.
2. Every maximal run of other characters becomes one `-`.
3. No leading or trailing `-`; the empty input gives the empty string.
4. `tests/slug.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
