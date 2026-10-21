# Task `p07`: LRU cache with expiry

Add to `src/lib.rs` of this crate:

```rust
pub struct Cache
pub fn Cache::new(capacity: usize, ttl_ms: u64) -> Cache
pub fn Cache::insert(&mut self, key: &str, value: &str, now_ms: u64)
pub fn Cache::get(&mut self, key: &str, now_ms: u64) -> Option<String>
pub fn Cache::len(&self, now_ms: u64) -> usize
```

Acceptance criteria:

1. An entry inserted at `t` expires at `t + ttl_ms`: from then on `get` returns `None` (and drops it) and `len` does not count it. Reading an entry does not extend its life; inserting an existing key replaces its value and restarts its life.
2. `get` of a live entry and `insert` mark the key most recently used. When an insert of a new key would exceed `capacity`, expired entries are dropped first; only if none were expired is the least recently used live entry evicted.
3. `capacity == 0` stores nothing.
4. `tests/p07.rs` (already in the repository, currently failing to compile) passes under `cargo test`.

The spec has these sections, in this order: `State`, `insert`, `get`, `len`.

Concerns (the two authors' split when two write the spec):

- **A.** Capacity and recency: what is evicted when the cache is full.
- **B.** Expiry: when an entry stops being visible.

Constraints: no new dependencies; no `unwrap` or `expect` outside tests; keep the change to `src/lib.rs`.
