# ADR-0008: Rocket League work lives in one apps family, imported via filter-repo

**Status:** Accepted
**Date:** 2026-10-08
**Deciders:** Repository owner

## Context
`baileyrd/rusty_bullet` (physics port + verification), `baileyrd/RLEvalSystem`
(replay analysis app, private) and a planned native RLBot port belong together:
the roadmap has RLEval recover inputs through `rb_env` and the RLBot port drive
the same sim. ADR-0003 forbids dependencies on an app crate from outside its
family, so separate families would block that on day one.

## Decision
1. One family, `crates/apps/rocket_league/`, with `crates/` for every Cargo
   member and one product directory per product (`rusty_bullet/`, `rleval/`,
   later `rlbot/`). Per-product docs and ADR numbering stay in the product
   directory (ADR-0001's remit rule).
2. Crate names are unchanged at import; a rename is a later, pure-rename PR.
3. Imports use `git filter-repo` into the final layout, then a merge, instead of
   ADR-0001's bare `git subtree`: the rewrite is what lets private or oversized
   content stay out of this public repository's history. Each import is one PR.
4. Replays, captures and corpora are never committed. They live in the private
   repo `baileyrd/rocket_league_private`, checked out under `rleval/` (gitignored), so the
   existing relative paths keep working with no environment variable. Fixtures stay in git
   only under `fixtures/` (or `rleval/assets/replays/`); tests that need the corpus skip
   when it is absent.
5. Reusable libraries (FlatBuffers, Protobuf) are `libs/protocol/` crates, not
   family crates.
6. Licence: MIT OR Apache-2.0.

## Consequences
- Source SHAs change; each import PR carries the old-to-new commit map.
- `rb_tape_bot` is `exclude`d from the workspace until the native RLBot port
  replaces its external `rlbot` client (ADR-0002: no new external dependency
  enters the lock through it).
- `boxcars`/`subtr-actor` stay external (Tier T) until a first-party replay
  parser is decided separately.
