# ADR-0004: Release shipped products from the monorepo, one prefixed tag series each

Status: Accepted
Date: 2026-09-24

## Context

ADR-0001 moved crates' *code* here and was explicit that each crate keeps its
own identity and governance. Until now that was enough, because nothing
imported so far was *distributed*: no crate here published binaries, and CI
only had to build and test.

`rusty_remind_me` (imported into `crates/apps/rusty_remind_me`) is the first
that is. As `baileyrd/rusty_remind_me` it shipped:

- a GitHub Release per version (tags `vX.Y.Z`, four platform archives plus a
  Claude Code plugin archive), cut automatically when
  `[workspace.package].version` moved;
- a Claude Code plugin marketplace at its repository root
  (`claude plugin marketplace add baileyrd/rusty_remind_me`);
- a `remind_me_self_update` tool that `git pull`s its own checkout and
  rebuilds `--workspace`.

Every one of those assumed the product *was* the repository: its root, its
tags, its `[workspace.package]`, its `--workspace`. None survives the move
unchanged, and "keep releasing from the old repository" would mean the
released code and the developed code live in different places.

## Decision

**A product that ships releases does so from this repository, under its own
tag prefix.**

- **Tags are `<product>-vX.Y.Z`** (`rusty-remind-me-v0.2.1`), never bare
  `vX.Y.Z`, which would claim to version the whole monorepo. Releases set
  `make_latest: false` so the Releases page's "Latest" never flips to
  whichever product shipped last, and carry a body linking the product's own
  `RELEASE_NOTES.md` rather than GitHub's generated notes, which would list
  every monorepo PR since *any* product's previous release.
- **One release workflow per product**
  (`.github/workflows/<product>-release.yml`), triggered by a push to `main`
  touching that product's manifests and doing nothing unless its version
  moved past its newest tag — the same "bumping the version is the whole
  release process" contract the product had standalone.
- **The product's version is its crates' own `[package].version`**, since
  this workspace's `[workspace.package]` belongs to other crates. A
  multi-crate product keeps its crates in lockstep with a script that fails
  when they disagree (for `rusty_remind_me`,
  `scripts/get_workspace_version.sh`), run on every PR that touches it.
- **Claude Code plugins are listed in one marketplace at this repository's
  root** (`.claude-plugin/marketplace.json`, name `rusty-mill`), each entry a
  relative `source` pointing at the product directory, which stays the
  plugin root. A product's old standalone marketplace, if it had one, is
  repointed at that directory with a `git-subdir` source so existing installs
  keep updating.
- **A product whose `--all-features` build is not meaningful is excluded
  from the generic clippy/test jobs** (`select-packages`' `exclude` input)
  and gets dedicated jobs in `ci.yml` that run the feature matrix it was
  actually verified with, gated on the `plan` job like everything else.
  `remind_me_core --all-features` would build whisper.cpp, usearch and the
  AWS SDK together and link libunwind's ptrace API, on Windows too.
  Checks that compare against another repository and so need a daily
  schedule (`rusty_remind_me`'s schema drift against `baileyrd/remind_me`)
  go in a separate workflow, because a scheduled run of `ci.yml` is a full
  workspace sweep.

## Alternatives considered

**Keep releasing from the standalone repository**, syncing code back with
`git subtree push`. Keeps existing URLs and bare tags, but makes the
standalone repository a release mirror that has to be kept in step by hand,
and a release would be built from a tree no CI here ever tested.

**Bare `vX.Y.Z` tags, since only one product releases today.** Correct
until the second product ships, then permanently ambiguous — and renaming
published tags breaks every link and pinned `ref` to them.

**One release workflow for every product, matrixed.** Premature with one
product (no abstraction before two call sites); revisit when a second one
arrives.

## Consequences

- `rusty_remind_me` releases continue from v0.2.1 as
  `rusty-remind-me-v0.2.1`; v0.2.0 and earlier stay on
  `baileyrd/rusty_remind_me`'s Releases page. That repository is frozen with
  a README notice, and its marketplace entry now points here.
- A shipped product's version bump is a PR touching its manifests, which
  also triggers its release on merge. Merging this ADR's own PR releases
  `rusty-remind-me-v0.2.1`.
- `macos-14`, which the release matrix uses, is unsupported from
  2026-11-02; the workflow's comment says what to move to.
- Self-updating products have to know they live in a monorepo. For
  `rusty_remind_me` that meant scoping "commits behind" to its own directory
  and rebuilding two packages instead of `--workspace` (its
  `docs/adr/0020`).
