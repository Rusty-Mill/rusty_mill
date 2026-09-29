# ADR-0002: Per-user tokens and one store directory per user

Status: Accepted by the owner (2026-09-29), including the three open questions,
which are answered below. No code yet.
Date: 2026-09-29

Related: ADR-0001 (HTTP stack), `SPIKE-FINDINGS.md` (gap 6),
Rusty-Mill/rusty_mill#382, `rusty_multimodal_db_engine`'s `DirLock`.

## Context

The API has one bearer token (`RUSTY_TICK_TOKEN`) and one data directory, so
there is exactly one user and no identity in the code. `pool::StorePool`
(merged in #386) can keep a bounded number of per-user stores open, but
nothing gives it a user to open. This ADR decides how a request becomes a
user, and what changes so the pool has a caller.

Facts from the code that shape it:

- A user's data is a `Service` (a `TaskStore` plus a `ListStore` in one
  directory), not just a `TaskStore`. **`StorePool` holds only a
  `TaskStore`, so as merged it cannot serve the API.**
- The server keeps one `Mutex<Service>` and runs each request under it
  (ADR-0001). That is acceptable for one user and stays acceptable for a few.
- Single-user startup (`Service::open`) takes no `DirLock`, so two
  `rusty_tick` processes on one directory would overwrite each other. The
  pool takes the lock; the single-user path does not.
- Per-user cost is small and linear: about 260 bytes and 1.5 µs per record
  when a store is open (`SPIKE-FINDINGS.md`).

## Decision

1. **Two modes, chosen by one file.** If `<data-dir>/users.json` exists the
   server runs multi-user; otherwise it runs as today, with
   `RUSTY_TICK_TOKEN` and the data directory's own store. Existing
   deployments change nothing. Setting `RUSTY_TICK_TOKEN` in multi-user mode
   is an error at startup, so nobody believes a token is enforced when it is
   not.
2. **A token names its user:** `Authorization: Bearer <user key>.<secret>`.
   The user key is the existing `UserKey` (1 to 64 of `A-Z a-z 0-9 _ -`;
   `.` is not allowed in a key, so the first `.` splits it). The secret is 32
   bytes from `rusty_rand`, encoded with `rusty_base64` (URL-safe, no
   padding). Naming the user lets the server look up one record instead of
   trying every token.
3. **Only digests are stored.** `users.json` holds, per user, a list of
   `{id, label, sha256, createdMs}` tokens (several, so each device can be
   revoked alone) and a `disabled` flag. A 256-bit random secret needs no
   password KDF, so SHA-256 is enough.
4. **Verification is uniform.** Split the header, look up the user, hash the
   secret, compare against each of the user's digests in constant time. An
   unknown user, a disabled user, a bad secret and a malformed header all
   give the same `401` and the same work (an unknown user is compared
   against a fixed dummy digest), so responses reveal nothing about which
   keys exist.
5. **Isolation is a directory.** A user's data lives in
   `<data-dir>/users/<key>/`, opened through a pool under a `DirLock`. No
   query ever crosses users, because no store does.
6. **The pool holds `Service`, not `TaskStore`.** `StorePool` becomes
   `ServicePool` (opening `Service::open(dir, clock)`), keeping its bound,
   least-recently-used eviction and lock. It is not made generic: there is one
   thing to pool.
7. **Concurrency is unchanged in kind.** `Mutex<Service>` becomes
   `Mutex<ServicePool>`; a request authenticates, then runs
   `pool.with(user, |service| api.handle(...))` under that one lock. Opening
   an evicted user's store happens under it too, so it stalls everyone for
   about 2 ms per 1,000 records of that user (15 ms at 10,000). That is
   accepted now; per-user locks are the next step if it is not, and the pool
   API does not have to change for it. **The pool holds at most 32 open
   users**, a constant (`DEFAULT_MAX_OPEN_USERS`), not a flag: at 260 bytes
   per record that bounds memory at about 8 MiB per 1,000 records each, and
   a flag can be added the day someone hits it.
8. **Provisioning is a CLI, not an HTTP route.** `rusty_tick user add <key>
   [--label L]` prints the full token once and stores its digest; `user
   list`, `user revoke <key> <token id>`, `user disable <key>` and `user
   enable <key>` do the rest. There is no HTTP admin API, so the first user
   needs filesystem access and no bootstrap secret exists to leak. The CLI
   writes `users.json` by temp file and rename (mode `0600`), and the server
   re-reads it when its modification time changes (checked at most once a
   second), so a revoke takes effect without a restart.
9. **No user is ever deleted by the tool.** `disable` blocks the token;
   removing data is an operator's `rm`, since nothing here can undo it.
10. **`/health` stays open** and reports only `ok`.

## Non-goals

Sharing lists between users, roles or admin users, password login, OAuth,
per-user quotas, and rate limiting. The server speaks plain HTTP behind a TLS
front end (ADR-0001), which is also where a failed-login limiter belongs.
Failures are logged with the user key and never the secret.

## Consequences

- **Adds an external dependency:** SHA-256 from `sha2`, as the owner chose.
  The monorepo has `rusty_sha1` and no SHA-256, and `sha2` is already a
  pinned workspace dependency (`[workspace.dependencies]`, used by the `nexus`
  crates), so `rusty_tick` takes it with `{ workspace = true }` and adds no
  new package to the lockfile. (An earlier draft of this ADR named
  `rusty_remind_me` and `rusty_multimodal_db` as users; they use the `sha256`
  crate and `sha2` 0.10, so they are not the precedent.) This ADR is the rationale for the
  dependency policy check. A first-party SHA-256 would replace it later
  behind the same one-function seam (`digest(secret) -> [u8; 32]`).
- Adds `rusty_rand`, `rusty_base64` and `rusty_json` (already used) as
  first-party dependencies.
- A leaked `users.json` exposes digests of 256-bit secrets, which cannot be
  brute-forced; a leaked token exposes one user until it is revoked.
- Single-user mode should also take the `DirLock` at startup, a one-line fix
  worth doing with this work.
- Moving an existing single-user directory into a user is manual (stop the
  server, move the store files into `users/<key>/`). A `user adopt` command
  is left until someone needs it.
- Clients change only in what they send as the token.

## Plan

Each step is a PR that leaves the tree green; steps 1 and 2 change no
behaviour.

1. `ServicePool` in place of `StorePool` (with the tests it has, run
   against `Service`).
2. `users` module: the file format, `Token::parse`, digest compare, mtime
   reload. Pure and unit-tested, including the uniform-failure cases.
3. `Api` takes an authenticator that returns a `UserKey`; the server holds
   the pool. Single-user mode is one fixed key over the root directory.
4. The `user` subcommands.
5. README and this ADR set to Accepted; an end-to-end test with two users
   proving neither sees the other's lists, a revoked token failing, and
   eviction under a capacity of one.

## Open questions, answered

1. **SHA-256:** `sha2`, behind a one-function seam.
2. **Registry:** the `users.json` file in the data directory, not an engine
   store.
3. **Pool size:** a fixed default of 32 open users, no flag.
