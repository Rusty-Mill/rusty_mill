# Task `p04`: CSV split_record and join_record

Add to `src/lib.rs` of this crate:

```rust
pub fn split_record(line: &str) -> Option<Vec<String>>
pub fn join_record(fields: &[&str]) -> String
```

Acceptance criteria:

1. `split_record` splits on commas; a field may be wrapped in double quotes, inside which commas are literal and `""` is one quote. The empty line is one empty field; a trailing comma adds an empty field.
2. It returns `None` for an unterminated quoted field, a quote inside an unquoted field, or characters after a closing quote other than a comma.
3. `join_record` joins with commas and quotes a field if and only if it contains a comma, a quote, or a leading or trailing space; inside quotes each `"` is doubled. `split_record(join_record(f))` gives `f` back.
4. `tests/p04.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Reading`, `Writing`.

Concerns (the two authors' split when two write the spec):

- **A.** Reading: how `split_record` splits one CSV line into fields.
- **B.** Writing: how `join_record` writes fields as one CSV line.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
