# ADR-0013: Repositories outside the monorepo pin its crates to a `libs-*` tag

Status: Accepted
Date: 2026-10-28

## Context

rusty-remind-me is moving to its own repository, `Rusty-Mill/rusty-remind-me`
(Mill Board #61, decided by the owner on 2026-10-26). It is the first product
built outside the monorepo that still uses monorepo crates: 10 directly
(`rusty_mcp_server`, `rusty_axum`, `rusty_tokio`, `rusty_oauth`,
`rusty_multimodal_db_engine` and five foundation crates) and 22 more
transitively.

Those crates are not published to crates.io, and the monorepo is public, so a
Cargo git dependency can reach them without a token. What is missing is a
shared, readable name for "a commit of `main` that is safe to build against".

## Decision

**An outside repository depends on monorepo crates through git dependencies
pinned to one `libs-YYYY.MM.N` tag.**

```toml
[workspace.dependencies]
rusty_mcp_server = { git = "https://github.com/Rusty-Mill/rusty_mill", tag = "libs-2026.10.0" }
```

- **Tags are cut by `.github/workflows/libs-tag.yml`**, run by hand. It tags
  the current head of `main`, which only moves through the merge queue, so a
  tag always names a commit that passed `required-gate`. `N` counts from 0
  within the month. A head that already has a `libs-*` tag is not tagged again.
- **Tags are monorepo-wide**, not per consumer or per crate, so the next split
  reuses them.
- **One tag per consumer at a time.** Every monorepo dependency in the
  consumer's `[workspace.dependencies]` names the same tag, so Cargo resolves
  them from one checkout and the crates agree with each other.
- **Bump procedure:** change the tag in the consumer, `cargo update`, open a
  PR there and let its CI decide. A fix a consumer needs in a monorepo crate
  lands here first, by PR, then a new tag is cut.
- **`[patch.crates-io]` does not carry through git dependencies.** A consumer
  that builds a patched crate (today `jsonwebtoken`) copies the
  patch and its `vendor/` directory.
- **Published tags are never moved or deleted.** A bad tag is followed by a
  newer one.

## Consequences

- A change to a crate an outside repository uses can break that repository
  only when it bumps its tag, and its own CI catches it then. The monorepo's CI
  does not build outside consumers.
- Cargo fetches the whole monorepo for a git dependency. It is cached, but a
  consumer's first build is slower than with path dependencies.
- The monorepo's workspace inheritance (`workspace = true`) keeps working for
  consumers, because Cargo resolves a git dependency against its own workspace
  root.

## Alternatives considered

**Pin by commit SHA (`rev = "..."`).** Needs no workflow, but a SHA says
nothing about whether that commit was green, and two consumers on the same
commit cannot tell from their manifests.

**Per-crate tags or crates.io releases.** Versioning each of 32 crates on its
own is release work the monorepo does not do today (ADR-0004 versions products,
not libraries). Revisit if a crate gets outside users who are not ours.

**Vendor the crates into the consumer.** Copies drift, and fixes would have
to be made twice.
