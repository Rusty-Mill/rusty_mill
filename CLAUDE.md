# Working in this repo

A Rust implementation of [ACP](https://agentcommunicationprotocol.dev) v0.2.0 — protocol
types, an HTTP client, and a server framework for hosting agents. See `README.md` for what
it does; this file is about how to change it.

## Shipping

Every change goes through a pull request — no direct pushes to `main`.

- **Merge with a merge commit, never squash.** All merges to date (#7, #8, #9, #11) are
  merge commits; keep that consistent.
- **Merge only when CI is green** — all 13 checks, not just the fast ones.
- **Sync `main` and delete the branch** after merging.
- A merged PR is finished. Follow-up work starts a fresh branch off the new `main` rather
  than stacking commits on merged history.

## Verifying before you push

Run the full sweep locally and **check exit status** — do not grep output for the word
"error". A docs failure reached CI on #9 exactly that way: the command failed, the output
didn't contain what was being grepped for, and it read as a pass.

```sh
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features
cargo test --all-features --all-targets
cargo test --all-features --doc
cargo build --no-default-features                              # and per feature:
cargo build --no-default-features --features client            # server, redis-store,
cargo build --no-default-features --features well-known        # types-only
cargo +1.86 test                                               # MSRV
cargo package
RUSTDOCFLAGS="-D warnings --cfg docsrs" cargo +nightly doc --all-features --no-deps
```

CI runs the same set across stable, beta and 1.86.

### Redis

The multi-replica suite runs against both backends. The Redis half **skips** unless
`ACP_TEST_REDIS_URL` is set — and when it is set, an unreachable Redis fails the run
rather than quietly skipping.

```sh
redis-server --daemonize yes --port 6379
ACP_TEST_REDIS_URL=redis://127.0.0.1:6379 cargo test --all-features
```

Run it. Several bugs have only ever shown up on the Redis half, because its round-trips
are slow enough to lose races the in-memory store wins by microseconds.

## Testing concurrency

**Do not write tests that race.** A test which merely *usually* observes the right
ordering passes by luck on a fast store and fails intermittently on a loaded CI runner.
That is how the ordering bug in #21 reached `main` — the test that caught it was a
timing race that had been winning.

Make the ordering observable instead. `tests/ordering.rs` wraps `InMemoryStore` in a
decorator whose session appends take 300ms, so a violation fails every time rather than
occasionally. Prefer that shape: a store decorator that delays or records the operation
under test.

When fixing a race, **verify the new test fails without the fix** — revert the source,
run it, put it back. A concurrency test that was never seen to fail proves nothing.

## Invariants worth knowing before changing the server

- **The replica executing a run is its sole writer.** This is what lets `put_run` be a
  plain overwrite with no distributed locking. Everyone else reads snapshots and sends
  control signals through the store's pub/sub.
- **Terminal transitions apply exactly once**, so a cancellation racing a completion
  cannot rewrite the outcome.
- **The terminal event releases `sync` callers.** Anything a caller could reasonably read
  next — most sharply, the session history — must be written *before* that event goes out.
- **Storage failures fail the run.** Emitting is `async` and returns `Result` precisely so
  that a storage outage produces a failed run rather than a silently truncated one. Session
  writes follow the same rule.
- **A non-terminal run with no live lease has lost its writer** and is reaped by whichever
  replica next reads it.

## Not published

The crate is not on crates.io and is not going there. Depend on it from git. Don't add a
release workflow, a `documentation = "https://docs.rs/..."` link, or a version-based
install snippet.

MSRV is **1.86**. The optional `redis-store` feature needs 1.88, since the `redis` crate's
floor is higher — an optional dependency does not raise the MSRV for everyone else, so
`rust-version` stays at 1.86.

## Prose

Comments and docs explain *why*, not what. Where a choice has a real alternative, say what
was rejected and what it would have cost — the existing code, PR bodies and issues are
written that way, and it is worth keeping.
