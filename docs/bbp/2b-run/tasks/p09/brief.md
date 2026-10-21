# Task `p09`: money formatting

Add to `src/lib.rs` of this crate:

```rust
pub fn format_amount(milli: i64) -> String
```

Acceptance criteria:

1. `milli` is an amount in thousandths of a unit. It is rounded to cents, half away from zero, then written with `,` between thousands groups and exactly two decimals.
2. A negative result has a leading `-`; an amount that rounds to zero is written `0.00`, never `-0.00`.
3. Every `i64`, including `i64::MIN`, is formatted without panic.
4. `tests/p09.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `format_amount`, `Edge cases`.

Concerns (the two authors' split when two write the spec):

- **A.** Rounding: how thousandths become cents.
- **B.** Layout: sign, thousands separators and decimals.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
