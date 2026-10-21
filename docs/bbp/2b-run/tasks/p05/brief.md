# Task `p05`: wrap and count_words

Add to `src/lib.rs` of this crate:

```rust
pub fn wrap(text: &str, width: usize) -> Vec<String>
pub fn count_words(text: &str) -> usize
```

Acceptance criteria:

1. `wrap` splits text into words on any whitespace and fills lines greedily: a word goes on the current line if the line plus one space plus the word fits in `width` characters; a word longer than `width` sits alone on its own line, unbroken. Lines have no leading or trailing spaces; text with no words gives no lines.
2. `count_words` counts maximal runs of non-whitespace characters (Unicode whitespace, so tabs and newlines separate words).
3. `tests/p05.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Wrapping`, `Counting`.

Concerns (the two authors' split when two write the spec):

- **A.** Wrapping: how `wrap` breaks text into lines.
- **B.** Counting: what `count_words` counts as a word.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
