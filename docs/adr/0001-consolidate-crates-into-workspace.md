# ADR-0001: Consolidate `baileyrd`/`Rusty-Mill` crates into one Cargo workspace via `git subtree`

Status: Accepted
Date: 2026-09-01

## Remit of this ADR series

This `docs/adr/` directory (at the workspace root) records decisions
belonging to the **workspace as a whole** — crate mergers, workspace-wide
CI, cross-crate policy. It does not record decisions internal to one
crate's own design, which stay in that crate's own `docs/adr/` if it has
one (several already do, carried over from before the merge — e.g.
`crates/rustils_async/docs/adr/` — each still numbered from its own
`0001`). If a numbering collision between a per-crate series and this one
ever reads as ambiguous, the directory path disambiguates it; this remit
line is the other half of that disambiguation.

## Context
Before this repository existed, each of these crates was an independent
`baileyrd/*` (a few `Rusty-Mill/*`) repo: its own history, its own CI run,
its own release cadence. As the crate count grew past ~90, that had real
costs — no single CI run could validate a cross-crate change (e.g. a fix
to a shared pattern touching both `rusty_git` and `rusty_term`), and
finding duplicated logic across repos required cloning and grepping each
one by hand (the problem `dedupe-loop`/`sovereignty-loop` exist to bound,
but which still needed every repo checked out first).

## Decision
Merge every crate into one Cargo workspace, one repo per crate boundary
preserved (`crates/<name>`), via `git subtree` rather than a plain copy —
so each crate keeps its full original commit history instead of it being
squashed into a single "import" commit. Crates are merged one at a time,
one PR per crate (see the repo's own commit history: `Import <crate> into
crates/<crate>` merges), so each import stays reviewable on its own rather
than landing as one enormous initial commit.

Each crate keeps its own `Cargo.toml`, its own crate-level docs (README,
and where it had one, its own `CHANGELOG.md`/`RELEASE_NOTES.md`/
`docs/adr/`) — the merge changes where the code lives and how it's built
and tested, not each crate's own identity or governance history. CI
(`.github/workflows/ci.yml`) scopes each job to the crates a PR actually
touches, via an affected-crates filter, so validating one crate's change
doesn't require running the full ~90-crate suite.

## Alternatives considered
**Keep every crate as a separate repo, coordinate via tooling instead
(dedupe-loop/sovereignty-loop's `PLATFORM_REPOS` model).** This is what
predated the monorepo, and it's still how the wider `baileyrd`/`Rusty-Mill`
ecosystem largely works for crates *not* merged here. It loses on exactly
the two costs in Context: no single CI run across crates, and a
cross-repo duplication/sovereignty scan needs every repo cloned first.
`repo-inspector` (in `baileyrd/skill_pack`) exists specifically to restore
the second capability for crates that *are* in this workspace, without
needing that clone step.

**A plain copy/squash merge instead of `git subtree`.** Simpler
mechanically, but discards each crate's pre-merge commit history — a real
cost when a duplication or bug investigation needs to see how a piece of
code got the way it is, from before the merge.

## Consequences
- A cross-crate duplication or sovereignty scan (`repo-inspector`) can now
  run against one local checkout instead of cloning ~90 repos.
- One shared `Cargo.lock` — a version bump to a shared external dependency
  now affects every crate that uses it at once, which is more visible than
  before (each crate previously pinned independently) but also means a
  bump has to be validated against every consumer in one PR rather than
  drifting silently.
- Per-crate governance files (`RELEASE_NOTES.md`, `CHANGELOG.md`,
  `docs/adr/`) that existed before the merge are **not** superseded by this
  repo's own root-level versions — see this ADR's Remit section and the
  scope note at the top of the root `RELEASE_NOTES.md`/`CHANGELOG.md`.
  Reading only the root files misses per-crate history; reading only a
  per-crate file misses workspace-wide decisions like this one.
- Migration from the pre-existing `Rusty-Mill`/`baileyrd` namespace split
  isn't complete — some crates in `references/platform-directory.md` (in
  the `dedupe-loop`/`sovereignty-loop`/`repo-inspector` skills) aren't in
  this workspace yet. A dependency or duplication finding involving one of
  those is a "not migrated yet" note, not a false negative.
