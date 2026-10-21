# Task `p03`: to_roman and from_roman

Add to `src/lib.rs` of this crate:

```rust
pub fn to_roman(n: u32) -> Option<String>
pub fn from_roman(s: &str) -> Option<u32>
```

Acceptance criteria:

1. `to_roman` writes the canonical upper-case numeral (subtractive forms IV, IX, XL, XC, CD, CM) for 1 to 3999 and returns `None` otherwise.
2. `from_roman` accepts exactly the canonical numerals `to_roman` writes and returns their value; anything else (empty, lower case, non-canonical forms such as `IIII`, `VX`, `IC`) is `None`.
3. `tests/p03.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Encoding`, `Decoding`.

Concerns (the two authors' split when two write the spec):

- **A.** Encoding: which numbers `to_roman` accepts and the numeral it writes.
- **B.** Decoding: which numerals `from_roman` accepts.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
