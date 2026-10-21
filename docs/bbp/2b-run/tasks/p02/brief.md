# Task `p02`: parse_duration and format_duration

Add to `src/lib.rs` of this crate:

```rust
pub fn parse_duration(s: &str) -> Option<u64>
pub fn format_duration(secs: u64) -> String
```

Acceptance criteria:

1. `parse_duration` accepts one or more `<digits><unit>` parts with units `h`, `m`, `s`, in that order, each at most once, with no spaces, and returns the total in seconds.
2. It returns `None` for the empty string, a number without a unit, units out of order or repeated, any other character, and arithmetic overflow.
3. `format_duration` writes hours, minutes and seconds largest first, omits zero parts, and renders 0 as `0s`.
4. `tests/p02.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `Parsing`, `Formatting`.

Concerns (the two authors' split when two write the spec):

- **A.** Parsing: which strings `parse_duration` accepts and what it returns.
- **B.** Formatting: how `format_duration` renders a number of seconds.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
