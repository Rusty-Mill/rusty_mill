# Task `p01`: slug and truncate_slug

Add to `src/lib.rs` of this crate:

```rust
pub fn slug(input: &str) -> String
pub fn truncate_slug(slug: &str, max: usize) -> String
```

Acceptance criteria:

1. `slug`: ASCII letters are lower-cased and kept; ASCII digits are kept; every maximal run of other characters becomes one `-`; no leading or trailing `-`.
2. `truncate_slug`: a slug of at most `max` bytes is returned unchanged.
3. Otherwise the result is the longest prefix made of whole `-`-separated words whose length is at most `max`, with no trailing `-`.
4. If even the first word is longer than `max`, the result is that word cut to `max` bytes; `max == 0` gives the empty string.
5. `tests/p01.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Normalisation`, `Truncation`.

Concerns (the two authors' split when two write the spec):

- **A.** Normalisation: what `slug` keeps, lower-cases and replaces.
- **B.** Truncation: how `truncate_slug` shortens a slug to `max` bytes.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
