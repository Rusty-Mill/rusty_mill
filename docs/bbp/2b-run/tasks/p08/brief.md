# Task `p08`: retry policy with backoff

Add to `src/lib.rs` of this crate:

```rust
pub struct Policy { pub base_ms: u64, pub factor: u32, pub max_delay_ms: u64, pub max_attempts: u32, pub deadline_ms: u64 }
pub fn Policy::next_delay(&self, failures: u32, elapsed_ms: u64) -> Option<u64>
```

Acceptance criteria:

1. After `failures` failed attempts (at least 1), the delay is `base_ms * factor^(failures - 1)`, capped at `max_delay_ms`, computed with saturating arithmetic.
2. `next_delay` returns `None` when `failures >= max_attempts`, or when `elapsed_ms + delay` would pass `deadline_ms`. `deadline_ms == 0` means no deadline.
3. `failures == 0` is a caller error: return `None`.
4. `tests/p08.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `next_delay`, `Edge cases`.

Concerns (the two authors' split when two write the spec):

- **A.** Schedule: how long to wait before the next attempt.
- **B.** Budget: when to stop retrying.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
