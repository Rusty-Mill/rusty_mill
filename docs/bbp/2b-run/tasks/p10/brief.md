# Task `p10`: path normalisation

Add to `src/lib.rs` of this crate:

```rust
pub fn normalize(path: &str) -> String
```

Acceptance criteria:

1. `.` segments are removed; `..` removes the previous normal segment. In an absolute path `..` at the root is dropped; in a relative path a `..` with nothing to remove is kept.
2. Repeated `/` collapse to one. A leading `/` is kept. A trailing `/` is kept if the input had one, unless the result is `/` or `.`.
3. A result with no segments is `/` for an absolute path and `.` for a relative one.
4. `tests/p10.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `normalize`, `Edge cases`.

Concerns (the two authors' split when two write the spec):

- **A.** Dot segments: what `.` and `..` do.
- **B.** Separators: repeated, leading and trailing `/`.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
