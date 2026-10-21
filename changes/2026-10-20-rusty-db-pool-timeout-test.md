---
category: Fixed
changelog: **`rusty-db`**'s `acquire_timeout_errors_instead_of_hanging_forever` test no longer fails on loaded Windows runners because its setup used up the 200 ms acquire timeout.
---
## 2026-10-20 - rusty-db: pool_exhaustion acquire-timeout test no longer races its own setup

- **Fixed:** `tests/pool_exhaustion.rs::acquire_timeout_errors_instead_of_hanging_forever` set a 200 ms `acquire_timeout` on a pool whose first connection is opened (file created, WAL set up, table created) under that same timeout, so a loaded Windows runner could spend it before the test reached its assertion and fail with `pool timed out` in about 0.2 s. The test now uses a single in-memory connection (no file, WAL or schema needed to exhaust a one-connection pool), a 1 s timeout with headroom, and stage-labelled failures (creating the pool, the first checkout, the exhausted acquire). It asserts specifically the pool's timeout error, that it fires within a bounded window around the configured timeout, and keeps a 10 s outer deadline so a broken `acquire_timeout` fails instead of hanging the suite. No driver code, pool semantics or retries change; only this test.
- **Tests:** the test itself. Six consecutive local runs pass (Linux); with the pool's `acquire_timeout` not applied it fails at the outer deadline.
- **Known limitations:** not run on Windows locally. The failure was seen only on the Windows CI shard, so the proof is CI passing there; it passed once on retry before this change, which does not show the race is gone.
