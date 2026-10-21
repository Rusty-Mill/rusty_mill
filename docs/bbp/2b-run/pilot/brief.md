# Task `pilot`: title_case and initials (pilot, not counted)

Add to `src/lib.rs` of this crate:

```rust
pub fn title_case(s: &str) -> String
pub fn initials(s: &str) -> String
```

Acceptance criteria:

1. `title_case` upper-cases the first letter of each whitespace-separated word, lower-cases the rest, and joins the words with one space.
2. `initials` returns the upper-cased first character of each word, with no separators; no words gives the empty string.
3. `tests/pilot.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Title case`, `Initials`.

Concerns (the two authors' split when two write the spec):

- **A.** Title case: how `title_case` capitalises words.
- **B.** Initials: what `initials` returns.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
