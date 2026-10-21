# Task `p06`: token bucket rate limiter

Add to `src/lib.rs` of this crate:

```rust
pub struct Bucket
pub fn Bucket::new(capacity: u32, refill_per_sec: u32) -> Bucket
pub fn Bucket::try_take(&mut self, n: u32, now_ms: u64) -> bool
```

Acceptance criteria:

1. A new bucket is full at time 0. Tokens refill at `refill_per_sec` per second, up to `capacity`, computed in integer arithmetic with the fractional remainder kept between calls (no drift, no floats).
2. `try_take(n, now)` first refills to `now`, then grants all `n` tokens or none: a refusal consumes nothing. `n == 0` is always granted; `n > capacity` is always refused.
3. A `now_ms` earlier than the last call refills nothing and must not panic or underflow.
4. `tests/p06.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `State`, `try_take`, `Edge cases`.

Concerns (the two authors' split when two write the spec):

- **A.** Refill and time: how tokens come back as `now_ms` advances.
- **B.** Admission: when a request for `n` tokens is granted and what it costs.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
